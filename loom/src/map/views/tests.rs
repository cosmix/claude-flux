use super::*;
use crate::context::graph_store::FileEntry;
use crate::context::source_graph::{NodeLanguage, SourceEdge, SourceEdgeKind, Span};
use serial_test::serial;
use std::collections::BTreeMap;
use tempfile::TempDir;

pub(super) fn impact_args(kinds: Vec<SourceEdgeKind>) -> ImpactArgs {
    ImpactArgs {
        depth: 3,
        kinds,
        limit: 0,
        path_prefix: None,
        min_confidence: 0.0,
    }
}

fn node(
    id: &str,
    path: &str,
    kind: SourceNodeKind,
    scope: &[&str],
    coverage: FileCoverage,
) -> SourceNode {
    SourceNode {
        id: id.to_string(),
        kind,
        path: PathBuf::from(path),
        scope: scope.iter().map(|s| s.to_string()).collect(),
        span: Span::default(),
        signature: String::new(),
        body_hash: "sha256:test".to_string(),
        language: NodeLanguage::Rust,
        parser_version: "test".to_string(),
        coverage,
    }
}

fn graph_of(files: Vec<(&str, SourceNode, Vec<SourceEdge>)>) -> ResolvedGraph {
    let mut map = BTreeMap::new();
    for (path, n, edges) in files {
        map.insert(
            path.to_string(),
            FileEntry {
                content_hash: "sha256:test".to_string(),
                coverage: n.coverage.clone(),
                nodes: vec![n],
                edges,
            },
        );
    }
    ResolvedGraph {
        base_revision: "test".to_string(),
        overlaid: Default::default(),
        files: map,
    }
}

#[test]
#[serial]
fn project_relative_resolves_an_absolute_path() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();

    let absolute = root.join("src/lib.rs");
    let rel = project_relative(&root, absolute.to_str().unwrap());
    assert_eq!(rel, Some("src/lib.rs".to_string()));
}

#[test]
#[serial]
fn project_relative_resolves_a_cwd_relative_path() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();

    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&root).unwrap();
    let rel = project_relative(&root, "src/lib.rs");
    std::env::set_current_dir(original_dir).unwrap();

    assert_eq!(rel, Some("src/lib.rs".to_string()));
}

#[test]
#[serial]
fn project_relative_falls_back_for_a_nonexistent_path() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();

    let rel = project_relative(&root, "./src/missing.rs");
    assert_eq!(rel, Some("src/missing.rs".to_string()));
}

#[test]
fn outline_of_a_lexical_only_file_states_status_and_detail() {
    let coverage = FileCoverage::LexicalOnly {
        detail: "no grammar for .toml".to_string(),
    };
    let n = node(
        "src/config.toml",
        "src/config.toml",
        SourceNodeKind::File,
        &[],
        coverage,
    );
    let graph = graph_of(vec![("src/config.toml", n, vec![])]);

    let root = TempDir::new().unwrap();
    let rendered = render_outline(&graph, root.path(), "src/config.toml");

    assert!(rendered.contains("lexical-only"));
    assert!(rendered.contains("no grammar for .toml"));
    assert!(!rendered.contains("coverage: 1 files"));
}

#[test]
fn find_all_falls_back_to_substring_only_when_exact_is_empty() {
    let n = node(
        "src/language.rs#type:DetectedLanguage",
        "src/language.rs",
        SourceNodeKind::Type,
        &["DetectedLanguage"],
        FileCoverage::Full,
    );
    let graph = graph_of(vec![("src/language.rs", n, vec![])]);

    assert!(render_find_all(&graph, "DetectedLanguage").contains("1 matches"));
    assert!(render_find_all(&graph, "detectedlang").contains("(substring matches)"));
    assert!(render_find_all(&graph, "NoSuchSymbol").contains("no nodes match"));
}

#[test]
fn find_all_still_lists_a_parse_error_file_and_reports_its_status() {
    let coverage = FileCoverage::ParseError {
        span: Span::default(),
        detail: "unexpected token".to_string(),
    };
    let n = node(
        "src/broken.rs",
        "src/broken.rs",
        SourceNodeKind::File,
        &[],
        coverage,
    );
    let graph = graph_of(vec![("src/broken.rs", n, vec![])]);

    let rendered = render_find_all(&graph, "broken.rs");
    assert!(rendered.contains("1 matches"));
    assert!(rendered.contains("[parse-error]"));
    assert!(!rendered.contains("coverage: 1 files"));
}

/// A three-hop chain (`baz -> bar -> foo`) with a strong parser-derived edge
/// nearest `foo` and a weaker inferred edge nearest `baz`, for
/// [`impact_row_shows_provenance_and_the_weakest_confidence_on_the_path`] to
/// assert the impact view reports each edge's provenance and the WEAKEST
/// confidence along the path, not just the nearest hop's.
pub(super) fn impact_chain_graph() -> ResolvedGraph {
    let foo = node(
        "src/a.rs#function:foo",
        "src/a.rs",
        SourceNodeKind::Function,
        &["foo"],
        FileCoverage::Full,
    );
    let bar = node(
        "src/b.rs#function:bar",
        "src/b.rs",
        SourceNodeKind::Function,
        &["bar"],
        FileCoverage::Full,
    );
    let baz = node(
        "src/c.rs#function:baz",
        "src/c.rs",
        SourceNodeKind::Function,
        &["baz"],
        FileCoverage::Full,
    );

    // bar --(parser, 1.0)--> foo ; baz --(inferred, 0.5)--> bar
    let bar_calls_foo = SourceEdge::parser(
        "src/b.rs#function:bar",
        "src/a.rs#function:foo",
        SourceEdgeKind::Calls,
        "foo",
    );
    let baz_calls_bar = SourceEdge::inferred(
        "src/c.rs#function:baz",
        "src/b.rs#function:bar",
        SourceEdgeKind::Calls,
        "bar",
        0.5,
    );

    graph_of(vec![
        ("src/a.rs", foo, vec![]),
        ("src/b.rs", bar, vec![bar_calls_foo]),
        ("src/c.rs", baz, vec![baz_calls_bar]),
    ])
}

#[test]
fn impact_row_shows_provenance_and_the_weakest_confidence_on_the_path() {
    let graph = impact_chain_graph();

    let root = TempDir::new().unwrap();
    let rendered = render_impact(
        &graph,
        root.path(),
        "foo",
        &ResolutionStats::default(),
        &impact_args(Vec::new()),
    );

    assert!(rendered.contains("parser"));
    assert!(rendered.contains("inferred"));
    assert!(rendered.contains("0.50"));
    assert!(
        !rendered.contains("d2  1.00"),
        "the second hop's weakest link is 0.5, not the first hop's 1.0: {rendered}"
    );
    assert!(!rendered.contains("coverage:"));
    assert!(!rendered.contains("resolution:"));
}

#[test]
fn empty_impact_names_what_was_not_traversed() {
    let lonely = node(
        "src/lonely.rs#function:lonely",
        "src/lonely.rs",
        SourceNodeKind::Function,
        &["lonely"],
        FileCoverage::Full,
    );
    let graph = graph_of(vec![("src/lonely.rs", lonely, vec![])]);
    let stats = ResolutionStats {
        retargeted: 0,
        ambiguous: 0,
        unresolved: 3,
    };

    let root = TempDir::new().unwrap();
    let rendered = render_impact(
        &graph,
        root.path(),
        "lonely",
        &stats,
        &impact_args(Vec::new()),
    );

    assert!(rendered.contains("no resolved edge reaches this node"));
    assert!(rendered.contains("3 unresolved edges"));
    assert!(!rendered.contains("nothing in the graph reaches this node"));
}

#[test]
fn impact_heading_names_the_kind_filter() {
    let graph = impact_chain_graph();
    let root = TempDir::new().unwrap();
    let mut args = impact_args(vec![SourceEdgeKind::Calls, SourceEdgeKind::References]);
    args.depth = 2;

    let rendered = render_impact(
        &graph,
        root.path(),
        "foo",
        &ResolutionStats::default(),
        &args,
    );

    assert!(rendered.contains(
        "Impact of src/a.rs#function:foo (depth <= 2, kinds: calls,references, reverse edges)"
    ));
}

#[test]
fn callers_view_lists_direct_callers_with_provenance() {
    let graph = impact_chain_graph();
    let root = TempDir::new().unwrap();

    let rendered = render_callers(&graph, root.path(), "foo", 0);

    assert!(rendered.contains("Callers of src/a.rs#function:foo"));
    assert!(rendered.contains("  1.00  parser  calls  src/b.rs#function:bar  (src/b.rs:0)"));
    assert!(!rendered.contains("src/c.rs#function:baz"));
}

#[test]
fn render_footer_prints_coverage_once() {
    let graph = impact_chain_graph();
    let stats = ResolutionStats {
        retargeted: 2,
        ambiguous: 1,
        unresolved: 3,
    };

    let rendered = render_footer(&graph, &stats);

    assert_eq!(rendered.matches("coverage:").count(), 1);
    assert!(
        rendered.contains("resolution: 2 retargeted, 1 ambiguous (left unresolved), 3 unresolved")
    );
}
