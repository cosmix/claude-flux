use crate::context::schema::*;
use std::path::PathBuf;

#[test]
fn estimate_tokens_is_four_bytes_per_token() {
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("abc"), 0);
    assert_eq!(estimate_tokens("abcd"), 1);
    assert_eq!(estimate_tokens("abcdefgh"), 2);
}

#[test]
fn confidence_ranks_identity_above_structure_above_lexical() {
    assert_eq!(
        Confidence::from_reasons(&[SelectionReason::Lexical, SelectionReason::ExactPath]),
        Confidence::High
    );
    assert_eq!(
        Confidence::from_reasons(&[SelectionReason::Lexical, SelectionReason::LinkedFrom]),
        Confidence::Medium
    );
    assert_eq!(
        Confidence::from_reasons(&[SelectionReason::Lexical]),
        Confidence::Low
    );
    assert_eq!(Confidence::from_reasons(&[]), Confidence::Low);
}

#[test]
fn never_built_freshness_reports_stale() {
    let freshness = Freshness::never_built("catalog has never been built");
    assert!(freshness.stale);
    assert!(freshness.revision.is_empty());
    assert!(freshness.detail.is_some());
}

#[test]
fn lifecycle_state_defaults_to_active_and_round_trips() {
    assert_eq!(LifecycleState::default(), LifecycleState::Active);
    let json = serde_json::to_string(&LifecycleState::Superseded).unwrap();
    assert_eq!(json, "\"superseded\"");
    let back: LifecycleState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, LifecycleState::Superseded);
}

#[test]
fn lifecycle_policy_defaults_to_current_and_round_trips_kebab_case() {
    assert_eq!(LifecyclePolicy::default(), LifecyclePolicy::Current);
    let json = serde_json::to_string(&LifecyclePolicy::Historical).unwrap();
    assert_eq!(json, "\"historical\"");
    assert_eq!(
        serde_json::from_str::<LifecyclePolicy>(&json).unwrap(),
        LifecyclePolicy::Historical
    );
}

#[test]
fn required_representation_defaults_to_full_and_round_trips_kebab_case() {
    assert_eq!(
        RequiredRepresentation::default(),
        RequiredRepresentation::Full
    );
    let json = serde_json::to_string(&RequiredRepresentation::Compact).unwrap();
    assert_eq!(json, "\"compact\"");
    assert_eq!(
        serde_json::from_str::<RequiredRepresentation>(&json).unwrap(),
        RequiredRepresentation::Compact
    );
}

#[test]
fn new_lifecycle_and_selection_variants_have_stable_display_names() {
    assert_eq!(LifecycleState::Historical.to_string(), "historical");
    assert_eq!(SelectionReason::GraphNeighbor.to_string(), "graph-neighbor");
}

#[test]
fn unmet_requirement_round_trips_through_json() {
    let requirement = UnmetRequirement {
        id: "architecture.md#pipeline#0".to_string(),
        needed_tokens: 240,
        available_tokens: 120,
        reason: "required representation exceeds remaining budget".to_string(),
    };

    let json = serde_json::to_string(&requirement).unwrap();
    assert_eq!(
        serde_json::from_str::<UnmetRequirement>(&json).unwrap(),
        requirement
    );
}

fn estimated_item(id: &str, token_count: usize) -> ContextItem {
    ContextItem {
        id: ChunkId::from(id),
        kind: ItemKind::KnowledgeChunk,
        pointer: SourcePointer {
            path: PathBuf::from("architecture.md"),
            anchor: String::new(),
            line_start: None,
            line_end: None,
        },
        summary: String::new(),
        source: Channel::Knowledge,
        token_count,
        score: 0.0,
        reasons: Vec::new(),
        confidence: Confidence::Low,
        state: LifecycleState::Active,
        content_hash: String::new(),
        excerpt: None,
        truncated: false,
        matched_term_count: 0,
    }
}

#[test]
fn recompute_estimate_adds_the_brief_frame_to_item_tokens() {
    let mut pack = ContextPack {
        query: String::new(),
        scope: vec![Channel::Knowledge],
        budget_tokens: 1_000,
        estimated_tokens: 0,
        structural_freshness: Freshness::default(),
        semantic_freshness: Freshness::default(),
        items: vec![estimated_item("first", 12), estimated_item("second", 30)],
        unmet_required: Vec::new(),
        omitted: OmissionSummary::default(),
        dropped_terms: Vec::new(),
        degraded: None,
    };

    pack.recompute_estimate();

    assert_eq!(pack.estimated_tokens, BRIEF_FRAME_TOKENS + 42);
}

/// `content_hash`, `excerpt`, and `truncated` were added to `ContextItem` after
/// packs were already being serialized, so a document written without them
/// must still deserialize — the fields are `#[serde(default)]` precisely for
/// this.
#[test]
fn context_item_reads_a_document_written_before_the_new_fields_existed() {
    let legacy = r#"{
        "id": "architecture.md#pipeline#0",
        "kind": "knowledge-chunk",
        "pointer": { "path": "architecture.md", "anchor": "pipeline" },
        "summary": "Pipeline",
        "source": "knowledge",
        "token_count": 12,
        "score": 1.5,
        "reasons": ["lexical"],
        "confidence": "low",
        "state": "active"
    }"#;

    let item: ContextItem = serde_json::from_str(legacy).unwrap();
    assert_eq!(item.content_hash, "");
    assert_eq!(item.excerpt, None);
    assert!(!item.truncated);

    // And what we write back is itself readable — the absent excerpt is skipped
    // rather than serialized as an explicit null.
    let json = serde_json::to_string(&item).unwrap();
    assert!(!json.contains("excerpt"), "got {json}");
    assert_eq!(serde_json::from_str::<ContextItem>(&json).unwrap(), item);
}

#[test]
fn chunk_id_is_transparent_in_json() {
    let id = ChunkId::new("architecture/hook-system.md#hooks#0");
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"architecture/hook-system.md#hooks#0\"");
    assert_eq!(id.as_str(), "architecture/hook-system.md#hooks#0");
}
