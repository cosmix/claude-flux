use std::path::Path;

use colored::Colorize;

use crate::context::graph_store::ResolvedGraph;
use crate::context::resolve::{direct_callees, direct_callers, Neighbor};

use super::{find_symbol_matches, project_relative, IMPACT_MAX_STARTS};

pub(super) struct ResolvedStarts {
    pub file_start: Option<String>,
    pub symbol_matches: Vec<String>,
    pub starts: Vec<String>,
}

pub(super) fn resolve_starts(
    graph: &ResolvedGraph,
    project_root: &Path,
    arg: &str,
) -> ResolvedStarts {
    let file_start =
        project_relative(project_root, arg).filter(|rel| graph.files.contains_key(rel));
    let symbol_matches = find_symbol_matches(graph, arg)
        .0
        .into_iter()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let starts = match &file_start {
        Some(rel) => vec![rel.clone()],
        None => symbol_matches.clone(),
    };
    ResolvedStarts {
        file_start,
        symbol_matches,
        starts,
    }
}

pub(super) fn impact_match_note(
    file_start: &Option<String>,
    symbol_matches: &[String],
    starts: &[String],
    safe_arg: &str,
) -> Option<String> {
    if file_start.is_some() && !symbol_matches.is_empty() {
        Some(format!(
            "note: {safe_arg} also matches {} symbol definition(s); showing the file's impact only",
            symbol_matches.len()
        ))
    } else if starts.len() > 1 {
        Some(format!(
            "{} {} definitions match {safe_arg}; showing impact for each",
            "→".cyan().bold(),
            starts.len()
        ))
    } else {
        None
    }
}

pub fn render_callers(
    graph: &ResolvedGraph,
    project_root: &Path,
    arg: &str,
    limit: usize,
) -> String {
    render_neighbors(graph, project_root, arg, limit, "Callers", direct_callers)
}

pub fn render_callees(
    graph: &ResolvedGraph,
    project_root: &Path,
    arg: &str,
    limit: usize,
) -> String {
    render_neighbors(graph, project_root, arg, limit, "Callees", direct_callees)
}

fn render_neighbors(
    graph: &ResolvedGraph,
    project_root: &Path,
    arg: &str,
    limit: usize,
    label: &str,
    query: fn(&ResolvedGraph, &str, usize) -> (Vec<Neighbor>, usize),
) -> String {
    let resolved = resolve_starts(graph, project_root, arg);
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
    for start_id in resolved.starts.iter().take(IMPACT_MAX_STARTS) {
        lines.push(format!(
            "{} {label} of {}",
            "→".cyan().bold(),
            crate::context::untrusted::inline_safe(start_id)
        ));
        let (neighbors, suppressed) = query(graph, start_id, limit);
        lines.extend(neighbors.iter().map(render_neighbor));
        if suppressed > 0 {
            lines.push(format!(
                "  ... {suppressed} more suppressed (raise --limit)"
            ));
        }
    }
    if resolved.starts.len() > IMPACT_MAX_STARTS {
        lines.push(format!(
            "  ... {} more suppressed",
            resolved.starts.len() - IMPACT_MAX_STARTS
        ));
    }
    lines.join("\n")
}

fn render_neighbor(neighbor: &Neighbor) -> String {
    let line_start = neighbor
        .line_start
        .map(|line| line.to_string())
        .unwrap_or_else(|| "?".to_string());
    format!(
        "  {:.2}  {}  {}  {}  ({}:{line_start})",
        neighbor.confidence,
        neighbor.provenance,
        neighbor.edge_kind,
        crate::context::untrusted::inline_safe(&neighbor.id),
        crate::context::untrusted::inline_safe(&neighbor.path),
    )
}
