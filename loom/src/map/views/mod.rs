//! Read-only CLI views over an already-resolved source graph.
//!
//! Every view is a pure `render_*` function; the command layer owns printing
//! and appends the global footer once after all requested views.
//!
//! None of these views fail on an empty result — a view that finds nothing
//! still renders a plain sentence saying so, because "the graph has no
//! answer" is not an error condition for a CLI query.
//!
//! Confidence and provenance are never flattened away: every row that comes
//! from `crate::context::impact` renders its own confidence, provenance, and
//! edge kind rather than collapsing them into a single generic line. And
//! reachability is never overclaimed: an empty reverse-impact result names
//! what was left untraversed (unresolved edges, symbol-less files) instead of
//! reading as "provably nothing depends on this".

use std::path::{Path, PathBuf};

use colored::Colorize;

use crate::context::graph_store::ResolvedGraph;
use crate::context::resolve::node_names;
use crate::context::source_graph::{
    file_node_id, FileCoverage, SourceEdgeKind, SourceNode, SourceNodeKind,
};
use crate::context::{CoverageReport, ResolutionStats};

/// Maximum number of ambiguous start definitions `impact` will expand.
pub(super) const IMPACT_MAX_STARTS: usize = 5;
/// Maximum number of rows `find_all` prints before suppressing the rest.
pub(super) const FIND_ALL_CAP: usize = 200;
/// Signature column is collapsed to single spaces and capped at this many
/// characters (including the trailing ellipsis) so an outline stays readable.
const SIGNATURE_MAX_LEN: usize = 80;

const DEFAULT_IMPACT_KINDS: [SourceEdgeKind; 4] = [
    SourceEdgeKind::Calls,
    SourceEdgeKind::References,
    SourceEdgeKind::Implements,
    SourceEdgeKind::Extends,
];
const ALL_EDGE_KINDS: [SourceEdgeKind; 6] = [
    SourceEdgeKind::Contains,
    SourceEdgeKind::Imports,
    SourceEdgeKind::Calls,
    SourceEdgeKind::References,
    SourceEdgeKind::Implements,
    SourceEdgeKind::Extends,
];

pub struct ImpactArgs {
    pub depth: usize,
    pub kinds: Vec<SourceEdgeKind>,
    pub limit: usize,
    pub path_prefix: Option<String>,
    pub min_confidence: f32,
}

/// Turn a user-supplied path into the forward-slashed, project-root-relative
/// form the graph keys on. Resolves against the CURRENT DIRECTORY first, so a
/// path the user can `cat` is a path `--outline` accepts.
pub(crate) fn project_relative(project_root: &Path, arg: &str) -> Option<String> {
    let candidate = if Path::new(arg).is_absolute() {
        Some(PathBuf::from(arg))
    } else {
        std::env::current_dir().ok().map(|dir| dir.join(arg))
    };

    if let Some(candidate) = candidate {
        if let (Ok(candidate), Ok(root)) = (candidate.canonicalize(), project_root.canonicalize()) {
            if let Ok(relative) = candidate.strip_prefix(&root) {
                return Some(file_node_id(relative));
            }
        }
    }

    // The candidate could not be resolved and canonicalized (most commonly:
    // the path does not exist yet, or the cwd could not be read) - fall back
    // to normalizing the argument itself; it may already be graph-relative.
    let normalized = arg.replace('\\', "/");
    let normalized = normalized.strip_prefix("./").unwrap_or(&normalized);
    Some(normalized.to_string())
}

/// Render every indexed node of one file, in source order.
pub fn render_outline(graph: &ResolvedGraph, project_root: &Path, arg: &str) -> String {
    let rel = project_relative(project_root, arg).unwrap_or_else(|| arg.to_string());
    // Flatten only for display - the graph lookup below keys on the raw
    // `rel`, since flattening must never change matching behaviour.
    let safe_rel = crate::context::untrusted::inline_safe(&rel);

    let Some(entry) = graph.files.get(&rel) else {
        return format!("no indexed file at {safe_rel}");
    };

    let mut lines = vec![format!("{} Outline: {}", "→".cyan().bold(), safe_rel)];
    let mut nodes: Vec<&SourceNode> = entry
        .nodes
        .iter()
        .filter(|node| node.kind != SourceNodeKind::File)
        .collect();
    nodes.sort_by_key(|node| node.span.start_byte);
    lines.extend(
        nodes
            .iter()
            .map(|node| format!("  {}", format_node_line(node))),
    );
    lines.push(file_coverage_line(&entry.coverage));
    lines.join("\n")
}

/// Format one outline row: line range, kind, scope path, signature.
///
/// `scope` and `signature` both originate in the parsed source file, not in
/// this program - they go through [`crate::context::untrusted::inline_safe`]
/// like every other graph-derived value rendered on this agent-facing
/// surface (see the module's containment rule).
fn format_node_line(node: &SourceNode) -> String {
    let range = format!("L{}-L{}", node.span.line_start, node.span.line_end);
    let scope = crate::context::untrusted::inline_safe(&node.scope.join("::"));
    let signature = crate::context::untrusted::inline_safe(&node.signature);
    let signature = crate::utils::truncate_for_display(&signature, SIGNATURE_MAX_LEN);
    format!(
        "{:<11} {:<11} {:<30} {}",
        range,
        node.kind.as_str(),
        scope,
        signature
    )
}

/// A file's own coverage status, with the detail that explains a degraded
/// status — a file with no symbols must say why.
///
/// Every `detail` here is flattened through
/// [`crate::context::untrusted::inline_safe`]: `ParseError`'s detail is
/// built from a raw line of the offending source file
/// (`context::extract::treesitter::collect::first_error`), so it is
/// repo-controlled text reaching agent-visible stdout the same way a
/// Knowledge Brief field is.
fn file_coverage_line(coverage: &FileCoverage) -> String {
    let status = coverage.status();
    match coverage {
        FileCoverage::Deleted | FileCoverage::Full => format!("coverage: {status}"),
        FileCoverage::Partial { detail } | FileCoverage::LexicalOnly { detail } => {
            format!(
                "coverage: {status} - {}",
                crate::context::untrusted::inline_safe(detail)
            )
        }
        FileCoverage::Oversized { bytes, limit } => {
            format!("coverage: {status} - {bytes} bytes (limit {limit})")
        }
        FileCoverage::ParseError { detail, .. } => format!(
            "coverage: {status} - {}",
            crate::context::untrusted::inline_safe(detail)
        ),
    }
}

/// Render every indexed node whose name matches `symbol`: an exact,
/// case-sensitive match first, falling back to a case-insensitive substring
/// match only when the exact pass finds nothing.
pub fn render_find_all(graph: &ResolvedGraph, symbol: &str) -> String {
    let (mut hits, label) = find_symbol_matches(graph, symbol);

    // Flatten only for display - matching above stays on the raw `symbol`.
    let safe_symbol = crate::context::untrusted::inline_safe(symbol);

    if hits.is_empty() {
        return format!("no nodes match {safe_symbol}");
    }

    hits.sort_by(|a, b| (&a.path, a.span.line_start).cmp(&(&b.path, b.span.line_start)));
    let mut lines = vec![format!(
        "{} {} - {} matches{}",
        "→".cyan().bold(),
        safe_symbol,
        hits.len(),
        label
    )];
    lines.extend(
        hits.iter()
            .take(FIND_ALL_CAP)
            .map(|node| find_all_row(node)),
    );
    if hits.len() > FIND_ALL_CAP {
        lines.push(format!(
            "  ... {} more suppressed",
            hits.len() - FIND_ALL_CAP
        ));
    }
    lines.join("\n")
}

/// Find nodes whose name matches `symbol`: an exact, case-sensitive match
/// first, falling back to a case-insensitive substring match only when the
/// exact pass finds nothing. The returned label distinguishes the two cases
/// for display.
pub(super) fn find_symbol_matches<'a>(
    graph: &'a ResolvedGraph,
    symbol: &str,
) -> (Vec<&'a SourceNode>, &'static str) {
    let exact: Vec<&SourceNode> = graph
        .nodes()
        .filter(|node| node_names(node).iter().any(|name| name == symbol))
        .collect();

    if exact.is_empty() {
        let needle = symbol.to_lowercase();
        let substring = graph
            .nodes()
            .filter(|node| {
                node_names(node)
                    .iter()
                    .any(|name| name.to_lowercase().contains(&needle))
            })
            .collect::<Vec<_>>();
        (substring, " (substring matches)")
    } else {
        (exact, "")
    }
}

/// One `find_all` row: location, kind, name, and — whenever the owning file's
/// coverage is not `full` — the coverage status, so a hit inside a degraded
/// file never renders identically to one from a fully-parsed file.
///
/// `node.path` and the matched name both come from the source graph (a
/// tracked file's path, or a symbol name parsed out of it), so both go
/// through [`crate::context::untrusted::inline_safe`] before reaching
/// agent-visible stdout.
fn find_all_row(node: &SourceNode) -> String {
    let path = crate::context::untrusted::inline_safe(&node.path.display().to_string());
    let location = format!("{path}:{}", node.span.line_start);
    let name = crate::context::untrusted::inline_safe(
        &node_names(node).into_iter().next().unwrap_or_default(),
    );
    let mut row = format!("  {:<36} {:<10} {}", location, node.kind.as_str(), name);
    if node.coverage.status() != "full" {
        row.push_str(&format!(" [{}]", node.coverage.status()));
    }
    row
}

/// Render what reaches a symbol or file, with per-edge confidence and provenance.
pub fn render_impact(
    graph: &ResolvedGraph,
    project_root: &Path,
    arg: &str,
    stats: &ResolutionStats,
    args: &ImpactArgs,
) -> String {
    let resolved = resolve_starts(graph, project_root, arg);

    // Flatten only for display - `file_start`/`symbol_matches`/`starts` above
    // are already resolved against the raw `arg`.
    let safe_arg = crate::context::untrusted::inline_safe(arg);

    if resolved.starts.is_empty() {
        return format!("no indexed node named {safe_arg}");
    }

    let mut lines = Vec::new();
    lines.extend(impact_match_note(
        &resolved.file_start,
        &resolved.symbol_matches,
        &resolved.starts,
        &safe_arg,
    ));

    let shown = resolved.starts.len().min(IMPACT_MAX_STARTS);
    for start_id in resolved.starts.iter().take(IMPACT_MAX_STARTS) {
        lines.push(render_impact_for(graph, start_id, stats, args));
    }
    if resolved.starts.len() > shown {
        lines.push(format!(
            "  ... {} more suppressed",
            resolved.starts.len() - shown
        ));
    }
    lines.join("\n")
}

/// Render one start node's impact heading and its reverse-reachability rows.
///
/// `start_id` and every `hit.id` are source-graph node ids (a file path or a
/// `scope::symbol` path lifted from the parsed source), so both are
/// flattened for display; the graph traversal itself still runs on the raw
/// `start_id`.
fn render_impact_for(
    graph: &ResolvedGraph,
    start_id: &str,
    stats: &ResolutionStats,
    args: &ImpactArgs,
) -> String {
    let safe_start_id = crate::context::untrusted::inline_safe(start_id);
    let heading = format!(
        "{} Impact of {safe_start_id} (depth <= {}, kinds: {}, reverse edges)",
        "→".cyan().bold(),
        args.depth,
        impact_kinds_label(&args.kinds),
    );
    let result = crate::context::resolve::impact_with(
        graph,
        start_id,
        &crate::context::resolve::ImpactOptions {
            max_depth: args.depth,
            kinds: args.kinds.clone(),
            limit: args.limit,
            path_prefix: args.path_prefix.clone(),
            min_confidence: args.min_confidence,
        },
    );
    if result.hits.is_empty() {
        return format!("{heading}\n  {}", untraversed_summary(graph, stats));
    }
    let mut lines = vec![heading];
    for hit in &result.hits {
        lines.push(format!(
            "  d{}  {:.2}  {}  {}  {}",
            hit.depth,
            hit.min_confidence,
            hit.weakest_provenance,
            hit.weakest_kind,
            crate::context::untrusted::inline_safe(&hit.id)
        ));
    }
    if result.suppressed > 0 {
        lines.push(format!(
            "  ... {} more suppressed (raise --limit)",
            result.suppressed
        ));
    }
    lines.join("\n")
}

fn impact_kinds_label(kinds: &[SourceEdgeKind]) -> String {
    if kinds.is_empty() || ALL_EDGE_KINDS.iter().all(|kind| kinds.contains(kind)) {
        "all".to_string()
    } else {
        kinds
            .iter()
            .map(SourceEdgeKind::as_str)
            .collect::<Vec<_>>()
            .join(",")
    }
}

pub(super) fn effective_impact_kinds(kinds: &[SourceEdgeKind]) -> Vec<SourceEdgeKind> {
    if kinds.is_empty() {
        DEFAULT_IMPACT_KINDS.to_vec()
    } else {
        kinds.to_vec()
    }
}

/// Explain an empty reverse-impact result honestly: the traversal only walks
/// RESOLVED edges, so it never proves nothing depends on a node — it can only
/// report what it did not (and could not) traverse.
fn untraversed_summary(graph: &ResolvedGraph, stats: &ResolutionStats) -> String {
    let coverage = CoverageReport::of(graph);
    let untraversed_files = coverage.files.saturating_sub(coverage.symbol_level_files);
    format!(
        "no resolved edge reaches this node ({} unresolved edges and {untraversed_files} \
         files without symbol coverage were not traversed)",
        stats.unresolved
    )
}

/// Render the invocation-wide coverage and resolution footer.
pub fn render_footer(graph: &ResolvedGraph, stats: &ResolutionStats) -> String {
    format!(
        "{}\nresolution: {} retargeted, {} ambiguous (left unresolved), {} unresolved",
        CoverageReport::of(graph),
        stats.retargeted,
        stats.ambiguous,
        stats.unresolved
    )
}

pub mod json;
mod neighbors;
pub use json::{callees_json, callers_json, find_all_json, footer_json, impact_json, outline_json};
use neighbors::{impact_match_note, resolve_starts};
pub use neighbors::{render_callees, render_callers};

#[cfg(test)]
mod tests;
