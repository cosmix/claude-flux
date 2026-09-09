use crate::context::retrieve::*;
use crate::context::schema::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Write `contents` to `root/relative`, creating parent directories as needed.
fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// A project tree `retrieve_for_stage` can run against end to end: a
/// `.loom/work/` directory for [`crate::fs::work_dir::WorkDir`] to find, and a
/// knowledge tree under `doc/loom/knowledge/` for the chunker to ingest.
///
/// `pub(super)` so sibling test modules under `context::tests` (e.g.
/// `retrieve_source`) can reuse it instead of duplicating a fixture project.
pub(super) fn project_with_knowledge() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".loom").join("work")).unwrap();
    write_file(
        root,
        "doc/loom/knowledge/architecture.md",
        "# Architecture\n\n\
         ## Orchestrator loop\n\n\
         The orchestrator polls stage files and spawns ready stages.\n\n\
         ## Signal generation\n\n\
         Signals carry the stage assignment into a session.\n",
    );
    write_file(
        root,
        "doc/loom/knowledge/conventions.md",
        "# Conventions\n\n\
         ## Commit style\n\n\
         Commits follow conventional commits.\n",
    );
    temp
}

fn project_with_lifecycle_pair() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".loom").join("work")).unwrap();
    write_file(
        root,
        "doc/loom/knowledge/current.md",
        "---\nstate: active\n---\n# Current\n\n## Rare topic\n\nquokka-zebra invariant wording\n",
    );
    write_file(
        root,
        "doc/loom/knowledge/old.md",
        "---\nstate: superseded\n---\n# Old\n\n## Rare topic\n\nquokka-zebra invariant wording\n",
    );
    temp
}

/// A pack carrying only the two revisions `context_epoch` reads.
fn pack_with_revisions(structural: &str, semantic: &str) -> ContextPack {
    ContextPack {
        query: "query".to_string(),
        scope: vec![Channel::Knowledge],
        budget_tokens: 100,
        estimated_tokens: 0,
        structural_freshness: Freshness {
            revision: structural.to_string(),
            ..Freshness::default()
        },
        semantic_freshness: Freshness {
            revision: semantic.to_string(),
            ..Freshness::default()
        },
        items: Vec::new(),
        unmet_required: Vec::new(),
        omitted: OmissionSummary::default(),
        dropped_terms: Vec::new(),
        degraded: None,
    }
}

#[test]
fn context_epoch_is_stable_across_two_calls_on_one_pack() {
    let pack = pack_with_revisions("structural-rev", "semantic-rev");
    let first = context_epoch(&pack);
    assert_eq!(first, context_epoch(&pack));
    assert_eq!(first.len(), 16, "16 hex chars, got {first}");
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn context_epoch_changes_when_either_revision_changes() {
    let base = context_epoch(&pack_with_revisions("structural-rev", "semantic-rev"));

    // Rebuilding only the structural layer is enough to re-open delivery.
    let structural_only = context_epoch(&pack_with_revisions("rebuilt", "semantic-rev"));
    assert_ne!(base, structural_only);

    let semantic_only = context_epoch(&pack_with_revisions("structural-rev", "rebuilt"));
    assert_ne!(base, semantic_only);
    assert_ne!(structural_only, semantic_only);
}

#[test]
fn context_epoch_separates_the_two_revisions() {
    // Without a separator, ("ab", "c") and ("a", "bc") would hash identically
    // and two different derived generations would claim one epoch.
    assert_ne!(
        context_epoch(&pack_with_revisions("ab", "c")),
        context_epoch(&pack_with_revisions("a", "bc"))
    );
}

#[test]
fn context_epoch_is_defined_when_no_layer_was_ever_built() {
    let epoch = context_epoch(&pack_with_revisions("", ""));
    assert_eq!(epoch.len(), 16);
}

#[test]
fn stage_query_new_covers_every_channel() {
    let query = StageQuery::new("/tmp/somewhere", "how does the orchestrator spawn stages?");
    assert_eq!(query.scope, Channel::all().to_vec());
    assert!(query.required_ids.is_empty());
    assert!(query.stage_dependency_ids.is_empty());
    assert_eq!(
        query.overlay,
        crate::context::local_overlay::OverlayScope::Local,
        "a query naming no stage must default to reading the working-tree overlay"
    );
}

#[test]
fn retrieve_for_stage_packs_the_knowledge_tree_within_budget() {
    let temp = project_with_knowledge();
    let query = StageQuery::new(temp.path(), "orchestrator loop spawns ready stages");

    let pack = retrieve_for_stage(&query, 500).unwrap();

    assert!(pack.within_budget());
    assert!(!pack.items.is_empty(), "the catalog should match the query");
    for item in &pack.items {
        assert!(
            !item.content_hash.is_empty(),
            "{} carries no content hash",
            item.id
        );
        assert!(item.excerpt.is_some(), "{} carries no excerpt", item.id);
    }
}

#[test]
fn retrieve_for_stage_is_deterministic_over_the_same_bytes() {
    let temp = project_with_knowledge();
    let query = StageQuery::new(temp.path(), "signal generation");

    let first = retrieve_for_stage(&query, 500).unwrap();
    let second = retrieve_for_stage(&query, 500).unwrap();

    assert_eq!(first, second);
    assert_eq!(context_epoch(&first), context_epoch(&second));
}

#[test]
fn retrieve_for_stage_refuses_a_required_id_the_catalog_does_not_hold() {
    let temp = project_with_knowledge();
    let mut query = StageQuery::new(temp.path(), "anything");
    query.required_ids = vec!["no-such-chunk".to_string()];

    let error = retrieve_for_stage(&query, 500).unwrap_err();
    assert!(
        error.to_string().contains("no-such-chunk"),
        "the error must name the unknown id, got: {error}"
    );
}

#[test]
fn a_superseded_chunk_is_not_retrieved_under_the_current_policy_even_with_identical_rare_wording() {
    let temp = project_with_lifecycle_pair();
    let query = StageQuery::new(temp.path(), "quokka zebra invariant wording");

    let pack = retrieve_for_stage(&query, 2_000).unwrap();

    assert!(pack
        .items
        .iter()
        .any(|item| item.state == LifecycleState::Active));
    assert!(pack
        .items
        .iter()
        .all(|item| item.state != LifecycleState::Superseded));
}

#[test]
fn a_superseded_chunk_is_retrieved_under_the_historical_policy() {
    let temp = project_with_lifecycle_pair();
    let mut query = StageQuery::new(temp.path(), "quokka zebra invariant wording");
    query.lifecycle = LifecyclePolicy::Historical;

    let pack = retrieve_for_stage(&query, 2_000).unwrap();

    assert!(pack
        .items
        .iter()
        .any(|item| item.state == LifecycleState::Superseded));
}

#[test]
fn a_required_superseded_chunk_is_still_returned() {
    let temp = project_with_lifecycle_pair();
    let mut discovery = StageQuery::new(temp.path(), "quokka zebra invariant wording");
    discovery.lifecycle = LifecyclePolicy::Historical;
    let historical = retrieve_for_stage(&discovery, 2_000).unwrap();
    let old_id = historical
        .items
        .iter()
        .find(|item| item.state == LifecycleState::Superseded)
        .expect("historical retrieval should expose the superseded id")
        .id
        .as_str()
        .to_string();
    let mut query = StageQuery::new(temp.path(), "unmatched query");
    query.required_ids = vec![old_id.clone()];

    let pack = retrieve_for_stage(&query, 2_000).unwrap();

    assert!(pack
        .items
        .iter()
        .any(|item| item.id.as_str() == old_id && item.state == LifecycleState::Superseded));
}

#[test]
fn retrieve_for_stage_fingerprints_the_knowledge_tree_once() {
    let temp = project_with_knowledge();
    take_evaluate_state_call_count();

    retrieve_for_stage(&StageQuery::new(temp.path(), "signal generation"), 500).unwrap();

    assert_eq!(
        take_evaluate_state_call_count(),
        0,
        "successful refresh already evaluated the tree; retrieval must reuse its outcome"
    );
}

#[test]
fn retrieve_for_stage_boosts_a_chunk_named_by_stage_dependency_ids() {
    let temp = project_with_knowledge();

    // Learn a real chunk id from the fixture catalog with an ordinary lexical
    // query, the way a caller would learn one from a prior pack or a delivery
    // record.
    let discovery = StageQuery::new(temp.path(), "signal generation");
    let discovered = retrieve_for_stage(&discovery, 500).unwrap();
    let dependency_chunk_id = discovered.items[0].id.as_str().to_string();

    // Free text that matches nothing lexically in the fixture, so the only
    // way the dependency chunk can appear in the pack — and the only reason
    // it can carry — is the stage-dependency boost itself.
    let mut query = StageQuery::new(temp.path(), "zzqx wobbleflorp unmatched terms");
    query.stage_dependency_ids = vec![dependency_chunk_id.clone()];

    let pack = retrieve_for_stage(&query, 500).unwrap();

    let boosted = pack
        .items
        .iter()
        .find(|item| item.id.as_str() == dependency_chunk_id)
        .unwrap_or_else(|| {
            panic!(
                "expected {dependency_chunk_id} in the pack, got {:?}",
                pack.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>()
            )
        });
    assert_eq!(
        boosted.reasons,
        vec![SelectionReason::StageDependency],
        "the boost must fire on its own, with no lexical match to piggyback on"
    );
}

/// No knowledge tree is a DEGRADED retrieval, not a failed one: the source
/// channel needs no catalog, so a checkout with a mapped graph and no
/// `doc/loom/knowledge/` still gets a pack. What must not happen is a pack that
/// reads as authoritative over nothing — both layers say plainly that they were
/// never built. (`retrieve_source.rs` covers the same absence with a graph
/// actually on disk.)
#[test]
fn retrieve_for_stage_degrades_rather_than_failing_with_no_knowledge_tree() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();
    let query = StageQuery::new(temp.path(), "anything");

    let pack = retrieve_for_stage(&query, 500).expect("a missing knowledge tree is not an error");

    assert!(pack.items.is_empty(), "nothing built, nothing to pack");
    assert!(
        pack.structural_freshness.stale && pack.semantic_freshness.stale,
        "a layer that was never built must never report itself current"
    );
}

/// The `loom knowledge` commands read and write the tree itself, so for them an
/// absent tree IS the error — `resolve_roots` still says so.
#[test]
fn resolve_roots_still_refuses_a_project_with_no_knowledge_tree() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join(".loom").join("work")).unwrap();

    let error = resolve_roots(temp.path()).unwrap_err();
    assert!(
        error.to_string().contains("Knowledge directory not found"),
        "got: {error}"
    );
}
