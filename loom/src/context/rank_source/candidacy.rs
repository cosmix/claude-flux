//! Whether a source node that earned no exact rung may stand on its lexical
//! score alone (A.24).
//!
//! BM25 over the source graph is a contest between ~10-token documents — a
//! symbol's scope and its signature — so a node matching two or three ordinary
//! words of a prompt scores as though the prompt had named it. It usually has
//! not. Three measured specimens, all from this repository:
//!
//! - `how do I configure the hooks so sessions get the right settings` ranked
//!   `fs/permissions/hooks.rs#function:configure_loom_hooks` FIRST, on
//!   `configure` and `hooks`;
//! - `why doesn't loom repair --fix do it, the point is that the plan should
//!   complete` pulled in `commands/repair.rs#function:fix_issue`, on `fix`;
//! - `read the remaining knowledge files that are relevant` pulled in
//!   `daemon/server/admission.rs#function:remaining`.
//!
//! The exact-rung gate ([`crate::context::lexical::ExactGate`]) already refuses
//! all three, which is why none of them carries an `ExactSymbol` reason. The
//! leak is one level down: plain lexical CANDIDACY. And it is asymmetric
//! between the channels, which is what made it grow rather than shrink as the
//! knowledge tree did. Stopwording drops a query term that is ubiquitous in the
//! corpus being searched, so the knowledge channel drops `hooks`, `sessions`
//! and `settings` — the project's own prose vocabulary — while the source
//! channel keeps every one of them, because a corpus of function signatures
//! contains almost no prose. The words a question is asked in are exactly the
//! words the source channel finds most discriminating.
//!
//! The rule here is the one thing a source node can offer that prose cannot,
//! stated as an admission test: **the prompt has to have named the symbol.**
//! Every word of the node's own name must be a query term this pass is
//! actually scoring, and the name must be more than one word — a single word
//! spelled like a word is exactly what the gate above already declined, and
//! re-admitting it here would undo that decision one step later.
//!
//! What this costs, written down here so nobody rediscovers it as a bug: asking
//! `what does tokenize do` no longer reaches `fn tokenize`, because `tokenize`
//! is one lowercase word and nothing in the prompt says it is code. Writing it
//! as `` `tokenize` `` does, through the exact-symbol rung and at the top of
//! tier 1. Multi-word names need no back-ticks at all: `where is reconcile
//! source graph called` still reaches `reconcile_source_graph`, because the
//! prompt supplied `reconcile`, `source` AND `graph`.

use crate::context::lexical::{name_parts, ExactGate};
use crate::context::rank::RankQuery;
use crate::context::schema::{SourceNode, SourceNodeKind};
use std::collections::BTreeSet;

/// Whether `node` may become a candidate on lexical evidence alone.
///
/// Consulted only when no rung fired: a node the query pointed at by id, path
/// or symbol has already earned its place, and its lexical score rides along.
///
/// `surviving` holds the query terms that survived stopwording — the terms this
/// pass scores. Testing name parts against THOSE rather than against the raw
/// query is what keeps the rule honest: a part the corpus dropped as ubiquitous
/// contributes nothing to any document's score, so treating the prompt as
/// having "supplied" it would admit a node on evidence the ranker never used.
pub(super) fn admits_lexical_evidence(
    query: &RankQuery,
    node: &SourceNode,
    surviving: &BTreeSet<&str>,
    gate: &ExactGate<'_>,
) -> bool {
    // A caller who demanded this id by hand has said what they meant, and
    // `withhold_partial_coverage` promises such a node stays rankable even
    // after its rung is taken away. That promise outranks this rule.
    if query.required_ids.iter().any(|id| id == &node.id) {
        return true;
    }
    // A `mod foo;` declaration is not a definition: its definition is the
    // module's own file, a separate node. A one-line span is exactly what a
    // declaration looks like (`mod source_graph;`), so admitting it on lexical
    // evidence alone would surface the same name twice for no added content.
    if node.kind == SourceNodeKind::Module && node.span.line_start == node.span.line_end {
        return false;
    }
    let Some(name) = node.scope.last() else {
        return false;
    };
    let parts = name_parts(name);
    if parts.is_empty() || !parts.iter().all(|part| surviving.contains(part.as_str())) {
        return false;
    }
    // One-word names fall back to the gate, which answers `Some` only for a
    // back-ticked or otherwise code-shaped occurrence. On a fully extracted file
    // that answer would already have awarded the exact-symbol rung, so this arm
    // decides exactly one case: a node whose rungs `withhold_partial_coverage`
    // took away for incomplete extraction, which must still stand on lexical.
    parts.len() > 1 || gate.admits(name).is_some()
}
