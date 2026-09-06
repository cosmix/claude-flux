//! The source channel's lexical candidacy floor (A.24): a node that earned no
//! exact rung is a candidate only when the prompt named its symbol.
//!
//! Every query here is a measured specimen from this repository, and the
//! property they pin together is one sentence: an ordinary English word cannot
//! put an unrelated symbol at rank 1. Before this floor, `how do I configure the
//! hooks so sessions get the right settings` ranked `configure_loom_hooks`
//! FIRST, above every curated chunk about configuring hooks.

use super::source_fixtures::{full_node, graph, node};
use crate::context::config::RetrievalConfig;
use crate::context::lexical::name_parts;
use crate::context::rank::{tokenize, RankQuery, RankedCandidate};
use crate::context::rank_source::rank_source;
use crate::context::schema::{FileCoverage, SelectionReason, SourceNode, SourceNodeKind, Span};

/// Rank `nodes` — each in its own file — against a plain text query.
fn rank_nodes(text: &str, nodes: Vec<SourceNode>) -> Vec<RankedCandidate> {
    let paths: Vec<String> = nodes
        .iter()
        .map(|node| node.path.display().to_string())
        .collect();
    let files: Vec<(&str, Vec<SourceNode>)> = paths
        .iter()
        .map(String::as_str)
        .zip(nodes.into_iter().map(|node| vec![node]))
        .collect();
    rank_source(
        &RankQuery {
            text: text.to_string(),
            ..RankQuery::default()
        },
        &graph(files),
        &RetrievalConfig::default(),
    )
}

fn ids(candidates: &[RankedCandidate]) -> Vec<&str> {
    candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect()
}

/// The headline specimen. Both nodes match `configure`, and one of them matches
/// `hooks` as well — but the prompt never says `loom`, so it never named
/// `configure_loom_hooks`, and two words of a three-word name is a coincidence
/// rather than a reference.
#[test]
fn an_ordinary_word_cannot_make_a_half_named_symbol_a_candidate() {
    let candidates = rank_nodes(
        "how do I configure the hooks so sessions get the right settings",
        vec![
            full_node(
                "src/permissions/hooks.rs#function:configure_loom_hooks",
                "src/permissions/hooks.rs",
                &["configure_loom_hooks"],
                "pub fn configure_loom_hooks(project_root: &Path) -> Result<()>",
            ),
            full_node(
                "src/permissions/settings.rs#function:configure_settings",
                "src/permissions/settings.rs",
                &["configure_settings"],
                "pub fn configure_settings(project_root: &Path) -> Result<()>",
            ),
        ],
    );

    assert_eq!(
        ids(&candidates),
        vec!["src/permissions/settings.rs#function:configure_settings"],
        "only the symbol the prompt spelled out in full may be a candidate: {candidates:?}"
    );
}

/// The floor is about NAMING, not about back-ticks: a multi-word name whose
/// every word the prompt supplied is admitted with no rung at all, which is what
/// keeps `where is reconcile source graph called` working for a reader who does
/// not know how the identifier is punctuated.
#[test]
fn a_multi_word_name_spelled_out_in_the_prompt_is_a_candidate() {
    let candidates = rank_nodes(
        "where is reconcile source graph called",
        vec![full_node(
            "src/context/refresh/source_graph.rs#function:reconcile_source_graph",
            "src/context/refresh/source_graph.rs",
            &["reconcile_source_graph"],
            "pub fn reconcile_source_graph(store: &ContextStore) -> Result<()>",
        )],
    );

    assert_eq!(
        ids(&candidates),
        vec!["src/context/refresh/source_graph.rs#function:reconcile_source_graph"]
    );
    assert_eq!(
        candidates[0].reasons,
        vec![SelectionReason::Lexical],
        "no rung fired — the prompt punctuated nothing — so this node is here on \
         the candidacy floor alone: {candidates:?}"
    );
}

/// A one-word name the prompt does say, and still not a candidate: `remaining`
/// is spelled like a word, so neither the exact-rung gate nor this floor will
/// vouch for it. Measured as `daemon/server/admission.rs#function:remaining`
/// leading a real Knowledge Brief at 94.7 points.
#[test]
fn a_one_word_name_is_not_a_candidate_however_often_the_prompt_says_it() {
    let candidates = rank_nodes(
        "read the remaining knowledge files that are relevant",
        vec![full_node(
            "src/daemon/server/admission.rs#function:remaining",
            "src/daemon/server/admission.rs",
            &["remaining"],
            "fn remaining(&self) -> usize",
        )],
    );

    assert!(
        candidates.is_empty(),
        "an English word may not stand in for a symbol reference: {candidates:?}"
    );
}

/// The inverse of the case above, so it cannot be satisfied by a fixture that
/// matches nothing: the same node, the same prompt, one pair of back-ticks.
#[test]
fn back_ticking_that_same_one_word_name_admits_it_at_the_top() {
    let candidates = rank_nodes(
        "read the `remaining` knowledge files that are relevant",
        vec![full_node(
            "src/daemon/server/admission.rs#function:remaining",
            "src/daemon/server/admission.rs",
            &["remaining"],
            "fn remaining(&self) -> usize",
        )],
    );

    assert_eq!(
        ids(&candidates),
        vec!["src/daemon/server/admission.rs#function:remaining"]
    );
    assert!(
        candidates[0]
            .reasons
            .contains(&SelectionReason::ExactSymbol),
        "back-ticks are a code reference whatever the spelling: {candidates:?}"
    );
}

/// A node whose file was only partly extracted has its rungs withheld, and
/// `withhold_partial_coverage` promises it still stands on whatever lexical
/// score it earns. The floor honours that promise through its gate arm: the
/// prompt back-ticked the name, so the evidence exists even though the rung
/// does not.
#[test]
fn a_withheld_rung_still_leaves_a_backticked_name_standing_on_lexical() {
    let degraded = node(
        "src/daemon/server/admission.rs#function:remaining",
        "src/daemon/server/admission.rs",
        &["remaining"],
        "fn remaining(&self) -> usize",
        SourceNodeKind::Function,
        FileCoverage::Partial {
            detail: "3 query matches had no named capture".to_string(),
        },
    );

    let candidates = rank_nodes("what does `remaining` return", vec![degraded]);

    assert_eq!(
        candidates[0].reasons,
        vec![SelectionReason::Lexical],
        "the rung is withheld, the candidate is not: {candidates:?}"
    );
}

/// A `mod foo;` declaration line is not a definition: its definition is the
/// module's own file, a separate node. Every part of its name survives
/// stopwording here, so without the one-line guard it would be admitted on
/// lexical evidence alone and reach the brief as a useless second hit.
#[test]
fn a_one_line_module_declaration_is_not_a_lexical_candidate() {
    let mut declaration = node(
        "loom/src/context/refresh.rs#module:source_graph",
        "loom/src/context/refresh.rs",
        &["source_graph"],
        "mod source_graph;",
        SourceNodeKind::Module,
        FileCoverage::Full,
    );
    declaration.span = Span {
        start_byte: 400,
        end_byte: 418,
        line_start: 19,
        line_end: 19,
    };

    let candidates = rank_nodes("what does the source graph do", vec![declaration]);

    assert!(
        candidates.is_empty(),
        "a one-line module declaration must not stand as a lexical candidate: {candidates:?}"
    );
}

/// The same name, but the node spans more than one line — the module's own
/// definition rather than a declaration referencing it elsewhere. The
/// one-line guard must not reject this one.
#[test]
fn a_multi_line_module_node_with_the_same_name_is_still_a_candidate() {
    let mut definition = node(
        "loom/src/context/source_graph/mod.rs#module:source_graph",
        "loom/src/context/source_graph/mod.rs",
        &["source_graph"],
        "pub mod source_graph {",
        SourceNodeKind::Module,
        FileCoverage::Full,
    );
    definition.span = Span {
        start_byte: 0,
        end_byte: 4000,
        line_start: 1,
        line_end: 120,
    };

    let candidates = rank_nodes("what does the source graph do", vec![definition]);

    assert_eq!(
        ids(&candidates),
        vec!["loom/src/context/source_graph/mod.rs#module:source_graph"],
        "a multi-line module node is a definition, not a bare declaration: {candidates:?}"
    );
}

/// A caller who demanded a one-line module declaration by its exact id has
/// said what they meant, and that promise outranks the one-line guard — the
/// same precedence `admits_lexical_evidence` (`context/rank_source/candidacy.rs`)
/// already gives `required_ids` over the name check.
///
/// Coverage is `Partial` rather than `Full` so `withhold_partial_coverage`
/// clears the `ExplicitId` rung `required_ids` would otherwise award on its
/// own — the only way to reach `admits_lexical_evidence`'s own `required_ids`
/// branch rather than being admitted before it is ever consulted.
#[test]
fn a_required_id_admits_a_one_line_module_declaration_regardless() {
    let mut declaration = node(
        "loom/src/context/refresh.rs#module:source_graph",
        "loom/src/context/refresh.rs",
        &["source_graph"],
        "mod source_graph;",
        SourceNodeKind::Module,
        FileCoverage::Partial {
            detail: "3 query matches had no named capture".to_string(),
        },
    );
    declaration.span = Span {
        start_byte: 400,
        end_byte: 418,
        line_start: 19,
        line_end: 19,
    };
    let id = declaration.id.clone();
    let path = declaration.path.to_string_lossy().into_owned();

    let candidates = rank_source(
        &RankQuery {
            text: "what does the source graph do".to_string(),
            required_ids: vec![id.clone()],
            ..RankQuery::default()
        },
        &graph(vec![(path.as_str(), vec![declaration])]),
        &RetrievalConfig::default(),
    );

    assert_eq!(ids(&candidates), vec![id.as_str()]);
}

#[test]
fn name_parts_splits_a_symbol_into_words_a_prompt_could_supply() {
    assert_eq!(
        name_parts("configure_loom_hooks"),
        vec!["configure", "loom", "hooks"],
        "the compound token tokenize also emits is deliberately absent"
    );
    assert_eq!(name_parts("ResidentPoint"), vec!["resident", "point"]);
    assert_eq!(name_parts("Foo::Bar"), vec!["foo", "bar"]);
    assert_eq!(name_parts("remaining"), vec!["remaining"]);
    assert_eq!(name_parts("HTTPServer"), vec!["httpserver"]);
    assert_eq!(name_parts("sha256_digest"), vec!["sha256", "digest"]);
    assert_eq!(name_parts("__"), Vec::<String>::new());
    assert_eq!(name_parts(""), Vec::<String>::new());
}

/// The one shape where the parts of a name are NOT a subset of the terms
/// `tokenize` derives from it, pinned rather than left to be rediscovered:
/// `tokenize`'s camel-case scan does not restart at `_`, so it fuses `foo_bar`
/// and never emits a bare `bar`. A prompt saying all three words still names the
/// symbol for candidacy; BM25 simply cannot score the middle one.
#[test]
fn a_name_mixing_underscores_and_camel_case_yields_a_part_tokenize_never_emits() {
    assert_eq!(name_parts("foo_barBaz"), vec!["foo", "bar", "baz"]);
    assert_eq!(
        tokenize("foo_barBaz"),
        vec!["foo_barbaz", "foo", "barbaz", "foo_bar", "baz"],
        "no bare `bar` here — that is the asymmetry name_parts documents"
    );
}
