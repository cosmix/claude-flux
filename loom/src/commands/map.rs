//! Map command - read-only queries over the derived source graph.

use std::path::Path;

use anyhow::{bail, Context, Result};
use colored::Colorize;

use crate::context::graph_store::{GraphStore, ResolvedGraph};
use crate::context::refresh::{ensure_snapshot, SnapshotAction, SnapshotPolicy};
use crate::context::source_graph::SourceEdgeKind;
use crate::context::store::ContextStore;
use crate::context::{resolve_graph, ResolutionStats};
use crate::fs::work_dir::WorkDir;
use crate::map::views::ImpactArgs;

const EDGE_KIND_NAMES: &str = "contains, imports, calls, references, implements, extends";

/// Arguments for `loom map`. Lives here rather than in the CLI enum so the
/// command owns its own surface.
#[derive(Debug, clap::Args)]
pub struct MapArgs {
    /// Print the indexed symbols of one file, in source order
    #[arg(long, value_name = "PATH")]
    pub outline: Option<String>,
    /// Print every indexed node whose name matches
    #[arg(long, value_name = "SYMBOL")]
    pub find_all: Option<String>,
    /// Print what reaches a symbol or file, with path confidence
    #[arg(long, value_name = "SYMBOL_OR_PATH")]
    pub impact: Option<String>,
    /// Print direct and transitive callers of a symbol
    #[arg(long, value_name = "SYMBOL")]
    pub callers: Option<String>,
    /// Print direct and transitive callees of a symbol
    #[arg(long, value_name = "SYMBOL")]
    pub callees: Option<String>,
    /// Maximum impact traversal depth
    #[arg(long, default_value_t = 3)]
    pub depth: usize,
    /// Comma-separated edge kinds included in impact traversal
    #[arg(
        long,
        value_name = "LIST",
        value_delimiter = ',',
        default_value = "contains,imports,calls,references,implements,extends",
        value_parser = parse_edge_kind
    )]
    pub kinds: Vec<SourceEdgeKind>,
    /// Maximum impact/caller/callee rows; zero means unlimited
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    /// Restrict impact hits to a project-relative path prefix
    #[arg(long, value_name = "PREFIX")]
    pub path: Option<String>,
    /// Minimum edge confidence included in impact traversal
    #[arg(long, default_value_t = 0.0)]
    pub min_confidence: f32,
    /// Emit one machine-readable JSON object
    #[arg(long)]
    pub json: bool,
}

/// Execute the map command in a checkout whether or not `loom init` has run.
pub fn execute(args: MapArgs) -> Result<()> {
    require_view(&args)?;
    let work_dir = WorkDir::new(".")?;
    let project_root = work_dir
        .project_root()
        .context("Could not determine project root")?
        .to_path_buf();
    run_views(&project_root, &work_dir, &args)
}

fn require_view(args: &MapArgs) -> Result<()> {
    if requested_views(args) == 0 {
        bail!(
            "loom map needs a view flag: --outline <PATH>, --find-all <SYMBOL>, \
             --impact <SYMBOL_OR_PATH>, --callers <SYMBOL>, or --callees <SYMBOL>"
        );
    }
    Ok(())
}

fn requested_views(args: &MapArgs) -> usize {
    [
        args.outline.is_some(),
        args.find_all.is_some(),
        args.impact.is_some(),
        args.callers.is_some(),
        args.callees.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count()
}

fn run_views(project_root: &Path, work_dir: &WorkDir, args: &MapArgs) -> Result<()> {
    let (graph, stats) = load_graph(project_root, work_dir)?;
    let impact_args = ImpactArgs {
        depth: args.depth,
        kinds: args.kinds.clone(),
        limit: args.limit,
        path_prefix: args.path.clone(),
        min_confidence: args.min_confidence,
    };
    if args.json {
        print_json_views(&graph, project_root, &stats, args, &impact_args)
    } else {
        print_human_views(&graph, project_root, &stats, args, &impact_args);
        Ok(())
    }
}

fn print_human_views(
    graph: &ResolvedGraph,
    project_root: &Path,
    stats: &ResolutionStats,
    args: &MapArgs,
    impact_args: &ImpactArgs,
) {
    let views = rendered_views(graph, project_root, stats, args, impact_args);
    let show_headings = views.len() > 1;
    for (name, rendered) in views {
        if show_headings {
            println!("{}", format!("== {name} ==").bold());
        }
        println!("{rendered}");
    }
    println!("{}", crate::map::views::render_footer(graph, stats));
}

fn rendered_views(
    graph: &ResolvedGraph,
    project_root: &Path,
    stats: &ResolutionStats,
    args: &MapArgs,
    impact_args: &ImpactArgs,
) -> Vec<(&'static str, String)> {
    let mut views = Vec::new();
    if let Some(arg) = &args.outline {
        views.push((
            "outline",
            crate::map::views::render_outline(graph, project_root, arg),
        ));
    }
    if let Some(arg) = &args.find_all {
        views.push(("find-all", crate::map::views::render_find_all(graph, arg)));
    }
    if let Some(arg) = &args.impact {
        views.push((
            "impact",
            crate::map::views::render_impact(graph, project_root, arg, stats, impact_args),
        ));
    }
    if let Some(arg) = &args.callers {
        views.push((
            "callers",
            crate::map::views::render_callers(graph, project_root, arg, args.limit),
        ));
    }
    if let Some(arg) = &args.callees {
        views.push((
            "callees",
            crate::map::views::render_callees(graph, project_root, arg, args.limit),
        ));
    }
    views
}

fn print_json_views(
    graph: &ResolvedGraph,
    project_root: &Path,
    stats: &ResolutionStats,
    args: &MapArgs,
    impact_args: &ImpactArgs,
) -> Result<()> {
    let mut views = serde_json::Map::new();
    if let Some(arg) = &args.outline {
        views.insert(
            "outline".into(),
            crate::map::views::outline_json(graph, project_root, arg),
        );
    }
    if let Some(arg) = &args.find_all {
        views.insert(
            "find_all".into(),
            crate::map::views::find_all_json(graph, arg),
        );
    }
    if let Some(arg) = &args.impact {
        views.insert(
            "impact".into(),
            crate::map::views::impact_json(graph, project_root, arg, stats, impact_args),
        );
    }
    if let Some(arg) = &args.callers {
        views.insert(
            "callers".into(),
            crate::map::views::callers_json(graph, project_root, arg, args.limit),
        );
    }
    if let Some(arg) = &args.callees {
        views.insert(
            "callees".into(),
            crate::map::views::callees_json(graph, project_root, arg, args.limit),
        );
    }
    let payload = serde_json::json!({
        "views": views,
        "coverage": crate::map::views::footer_json(graph, stats),
    });
    println!("{}", serde_json::to_string(&payload)?);
    Ok(())
}

/// Ensure the local snapshot, load its selected layers, and resolve inferred edges.
fn load_graph(project_root: &Path, work_dir: &WorkDir) -> Result<(ResolvedGraph, ResolutionStats)> {
    let store = ContextStore::open(work_dir)?;
    store.ensure()?;
    let graph_store = GraphStore::new(store.root(), work_dir.root());
    let snapshot = ensure_snapshot(
        &store,
        &graph_store,
        project_root,
        SnapshotPolicy::LocalCurrent,
    )?;
    if snapshot.action != SnapshotAction::Reused {
        eprintln!("{}", snapshot.describe());
    }
    let overlay = snapshot
        .overlay
        .as_ref()
        .map(|(plan, stage)| (plan.as_str(), stage.as_str()));
    let mut graph = graph_store.resolved(&snapshot.revision, overlay)?;
    let stats = resolve_graph(&mut graph);
    Ok((graph, stats))
}

fn parse_edge_kind(value: &str) -> std::result::Result<SourceEdgeKind, String> {
    [
        SourceEdgeKind::Contains,
        SourceEdgeKind::Imports,
        SourceEdgeKind::Calls,
        SourceEdgeKind::References,
        SourceEdgeKind::Implements,
        SourceEdgeKind::Extends,
    ]
    .into_iter()
    .find(|kind| kind.as_str() == value)
    .ok_or_else(|| format!("unknown edge kind '{value}'; valid kinds: {EDGE_KIND_NAMES}"))
}

#[cfg(test)]
#[path = "tests_map.rs"]
mod tests;
