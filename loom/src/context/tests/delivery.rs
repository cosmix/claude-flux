use crate::context::delivery::*;
use crate::context::retrieve::context_epoch;
use crate::context::schema::*;
use crate::models::stage::Stage;
use chrono::Utc;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

const PLAN: &str = "delivery-plan";
const STAGE: &str = "delivery-stage";

fn item(id: &str, content_hash: &str) -> ContextItem {
    ContextItem {
        id: ChunkId::from(id),
        kind: ItemKind::KnowledgeChunk,
        pointer: SourcePointer {
            path: PathBuf::from(format!("{id}.md")),
            anchor: String::new(),
            line_start: None,
            line_end: None,
        },
        summary: format!("{id} summary"),
        source: Channel::Knowledge,
        token_count: 4,
        score: 1.0,
        reasons: vec![SelectionReason::Lexical],
        confidence: Confidence::Low,
        state: LifecycleState::Active,
        content_hash: content_hash.to_string(),
        excerpt: Some(format!("body of {id}")),
        truncated: false,
        matched_term_count: 0,
    }
}

fn pack_with(items: Vec<ContextItem>, structural_revision: &str) -> ContextPack {
    ContextPack {
        query: "query".to_string(),
        scope: vec![Channel::Knowledge],
        budget_tokens: 100,
        estimated_tokens: items.iter().map(|item| item.token_count).sum(),
        structural_freshness: Freshness {
            revision: structural_revision.to_string(),
            ..Freshness::default()
        },
        semantic_freshness: Freshness::default(),
        items,
        unmet_required: Vec::new(),
        omitted: OmissionSummary::default(),
        dropped_terms: Vec::new(),
        degraded: None,
    }
}

fn record(recipient_id: &str, epoch: &str, delivered: &[(&str, &str)]) -> DeliveryRecord {
    DeliveryRecord {
        recipient_id: recipient_id.to_string(),
        launch_id: format!("launch-{recipient_id}"),
        context_epoch: epoch.to_string(),
        delivered: delivered
            .iter()
            .map(|(node_id, content_hash)| DeliveredNode {
                node_id: (*node_id).to_string(),
                content_hash: (*content_hash).to_string(),
            })
            .collect(),
        written_at: Utc::now(),
    }
}

#[test]
fn delivery_dir_sits_under_the_stage_overlay() {
    let work_dir = PathBuf::from("/project/.loom/work");
    let dir = delivery_dir(&work_dir, PLAN, STAGE);
    assert_eq!(
        dir,
        PathBuf::from("/project/.loom/work/context/delivery-plan/delivery-stage/session-retrieval")
    );
}

#[test]
fn from_pack_copies_every_item_in_order_and_stamps_a_unique_launch() {
    let pack = pack_with(
        vec![item("first", "sha256:aaa"), item("second", "sha256:bbb")],
        "structural-rev",
    );

    let first = DeliveryRecord::from_pack("session-1", &pack);
    let second = DeliveryRecord::from_pack("session-1", &pack);

    assert_eq!(first.recipient_id, "session-1");
    assert_eq!(first.context_epoch, context_epoch(&pack));
    assert_eq!(
        first.delivered,
        vec![
            DeliveredNode {
                node_id: "first".to_string(),
                content_hash: "sha256:aaa".to_string(),
            },
            DeliveredNode {
                node_id: "second".to_string(),
                content_hash: "sha256:bbb".to_string(),
            },
        ]
    );
    assert_ne!(
        first.launch_id, second.launch_id,
        "two deliveries must never share a launch id"
    );
}

#[test]
fn a_written_record_round_trips_through_the_delivery_directory() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let written = record("session-1", "epoch-a", &[("first", "sha256:aaa")]);

    record_delivery(&work_dir, PLAN, STAGE, &written).unwrap();
    let loaded = load_deliveries(&work_dir, PLAN, STAGE).unwrap();

    assert_eq!(loaded, vec![written]);
}

#[test]
fn a_delivery_under_a_new_epoch_replaces_the_recipients_record() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");

    record_delivery(
        &work_dir,
        PLAN,
        STAGE,
        &record("session-1", "epoch-a", &[("first", "sha256:aaa")]),
    )
    .unwrap();
    let second = record("session-1", "epoch-b", &[("second", "sha256:bbb")]);
    record_delivery(&work_dir, PLAN, STAGE, &second).unwrap();

    // One recipient still owns exactly one file, and a rebuilt derived layer
    // re-opens delivery: nothing from the old epoch is carried across.
    assert_eq!(
        load_deliveries(&work_dir, PLAN, STAGE).unwrap(),
        vec![second]
    );
}

/// The node ids one recipient's record holds after `deliveries` are written in
/// order, all under a single epoch.
fn merged_ids(deliveries: &[&[(&str, &str)]]) -> Vec<String> {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    for delivered in deliveries {
        let written = record("prompt-stage", "epoch-a", delivered);
        record_delivery(&work_dir, PLAN, STAGE, &written).unwrap();
    }

    let loaded = load_deliveries(&work_dir, PLAN, STAGE).unwrap();
    assert_eq!(loaded.len(), 1, "one recipient owns one file");
    loaded[0]
        .delivered
        .iter()
        .map(|node| node.node_id.clone())
        .collect()
}

#[test]
fn two_deliveries_in_one_epoch_accumulate_rather_than_replacing() {
    // The prompt hook keys every question a stage asks on ONE recipient id, so
    // a replacing write would erase what the previous prompt was handed — and
    // the prompt after that would be re-quoted the whole of it verbatim.
    assert_eq!(
        merged_ids(&[&[("a", "h1"), ("b", "h2")], &[("b", "h2"), ("c", "h3")]]),
        vec!["a", "b", "c"]
    );
}

#[test]
fn the_merged_set_does_not_depend_on_the_order_deliveries_arrived_in() {
    assert_eq!(
        merged_ids(&[&[("b", "h2"), ("c", "h3")], &[("a", "h1"), ("b", "h2")]]),
        vec!["a", "b", "c"]
    );
}

#[test]
fn plan_key_is_one_derivation_for_every_writer_and_reader() {
    let mut stage = Stage::new("Named".to_string(), None);
    assert_eq!(plan_key(&stage), "default", "a stage naming no plan");

    stage.plan_id = Some("   ".to_string());
    assert_eq!(plan_key(&stage), "default", "a blank id names no directory");

    stage.plan_id = Some("my-plan".to_string());
    assert_eq!(plan_key(&stage), "my-plan");

    // The id-only helper must agree with the stage-shaped one: the path is the
    // join key, so two derivations file records where the other never looks.
    assert_eq!(plan_key_from(stage.plan_id.as_deref()), plan_key(&stage));
    assert_eq!(plan_key_from(None), "default");
}

#[test]
fn loading_a_directory_that_was_never_written_reports_no_deliveries() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    assert!(load_deliveries(&work_dir, PLAN, STAGE).unwrap().is_empty());
}

#[test]
fn a_malformed_record_is_skipped_rather_than_failing_the_read() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let good = record("session-1", "epoch-a", &[("first", "sha256:aaa")]);
    record_delivery(&work_dir, PLAN, STAGE, &good).unwrap();

    let dir = delivery_dir(&work_dir, PLAN, STAGE);
    fs::write(dir.join("broken.json"), "{ not json at all").unwrap();
    fs::write(dir.join("ignored.txt"), "not a record").unwrap();

    // A delivery record is an optimisation, never state the run depends on.
    assert_eq!(load_deliveries(&work_dir, PLAN, STAGE).unwrap(), vec![good]);
}

#[test]
fn delivered_in_epoch_ignores_records_from_another_generation() {
    let records = vec![
        record("session-1", "epoch-a", &[("first", "sha256:aaa")]),
        record("session-2", "epoch-b", &[("second", "sha256:bbb")]),
    ];

    let delivered = delivered_in_epoch(&records, "epoch-a");

    assert_eq!(delivered.len(), 1);
    assert!(delivered.contains(&("first".to_string(), "sha256:aaa".to_string())));
    assert!(delivered_in_epoch(&records, "epoch-missing").is_empty());
}

#[test]
fn delivered_in_epoch_keys_on_the_hash_so_changed_bytes_re_deliver() {
    let records = vec![record(
        "session-1",
        "epoch-a",
        &[("first", "sha256:original")],
    )];

    let delivered = delivered_in_epoch(&records, "epoch-a");

    assert!(delivered.contains(&("first".to_string(), "sha256:original".to_string())));
    assert!(!delivered.contains(&("first".to_string(), "sha256:rewritten".to_string())));
}

#[test]
fn a_recipient_id_that_could_escape_the_directory_is_refused() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");

    // Every rejected shape, named: an id that reaches the filesystem unchecked
    // is a path-traversal write, and one that is merely sanitized files the
    // delivery under a name no reader looks up.
    for bad in [
        "",
        "../escape",
        "..",
        "nested/session",
        "nested\\session",
        "session id",
        "session:1",
        "sessio\0n",
    ] {
        let refused = record_delivery(&work_dir, PLAN, STAGE, &record(bad, "epoch-a", &[]));
        assert!(
            refused.is_err(),
            "recipient id {bad:?} should have been refused"
        );
    }

    // ...and nothing was written for any of them.
    assert!(load_deliveries(&work_dir, PLAN, STAGE).unwrap().is_empty());
}

#[test]
fn dependency_chunk_ids_unions_two_dependencies_deduplicated_and_sorted() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");

    record_delivery(
        &work_dir,
        PLAN,
        "dep-one",
        &record(
            "session-1",
            "epoch-a",
            &[("zeta", "sha256:aaa"), ("alpha", "sha256:bbb")],
        ),
    )
    .unwrap();
    record_delivery(
        &work_dir,
        PLAN,
        "dep-two",
        &record(
            "session-1",
            "epoch-a",
            &[("alpha", "sha256:bbb"), ("mid", "sha256:ccc")],
        ),
    )
    .unwrap();

    let ids = dependency_chunk_ids(
        &work_dir,
        PLAN,
        &["dep-one".to_string(), "dep-two".to_string()],
    );

    assert_eq!(
        ids,
        vec!["alpha".to_string(), "mid".to_string(), "zeta".to_string()]
    );
}

#[test]
fn dependency_chunk_ids_ignores_a_dependency_with_no_record_on_disk() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");

    record_delivery(
        &work_dir,
        PLAN,
        "dep-one",
        &record("session-1", "epoch-a", &[("alpha", "sha256:aaa")]),
    )
    .unwrap();

    let ids = dependency_chunk_ids(
        &work_dir,
        PLAN,
        &["dep-one".to_string(), "never-ran".to_string()],
    );

    assert_eq!(ids, vec!["alpha".to_string()]);
}

#[test]
fn dependency_chunk_ids_skips_a_malformed_record_rather_than_failing() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");

    record_delivery(
        &work_dir,
        PLAN,
        "dep-one",
        &record("session-1", "epoch-a", &[("alpha", "sha256:aaa")]),
    )
    .unwrap();
    let dir = delivery_dir(&work_dir, PLAN, "dep-one");
    fs::write(dir.join("broken.json"), "{ not json at all").unwrap();

    let ids = dependency_chunk_ids(&work_dir, PLAN, &["dep-one".to_string()]);

    assert_eq!(ids, vec!["alpha".to_string()]);
}

#[test]
fn an_ordinary_recipient_id_is_accepted() {
    let temp = TempDir::new().unwrap();
    let work_dir = temp.path().join(".loom").join("work");
    let accepted = record("session-2026.08.17_abc-DEF", "epoch-a", &[]);

    record_delivery(&work_dir, PLAN, STAGE, &accepted).unwrap();

    assert_eq!(
        load_deliveries(&work_dir, PLAN, STAGE).unwrap(),
        vec![accepted]
    );
}

// Session-scoped delivery dedupe (A.16) and its compaction reset (A.21) have
// their own tests in a submodule, split out to keep this file under the
// maintainability line limit — the standard `<name>.rs` + `<name>/` layout
// (as `context/rank.rs` + `context/rank/` do), which needs no edit to
// `context/tests/mod.rs`: this `mod` declaration lives here, not there.
mod session;
