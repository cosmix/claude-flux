# C1 — One hook target for every hook; the prompt hook reads its stage's overlay; per-item admission and abstention

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `delivery-and-catalog` · Single wave (parallel with C2, C3, D1, D2, D3).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F4, F6, the "Overlay isolation" and "Delivery suppression" contracts)

- **F4.** `commands/hook/user_prompt.rs::retrieve_for_prompt` builds `StageQuery::new(&target.project_root, prompt)` and never sets `query.overlay`, so it reads `OverlayScope::Local` (`_local/map-<dir>`) even inside a stage, while the spawn brief (`orchestrator/signals/retrieval.rs::stage_overlay_scope`) and the background reconcile (`reconcile_graph.rs`, `SourceGraphScope::Overlay { plan, stage }`) use `<plan>/<stage>`. Three modules re-derive the same identity independently (`user_prompt.rs::DeliveryTarget`, `pre_compact.rs::CompactionTarget`, `reconcile_graph.rs::ReconcileTarget`), each with a comment admitting it.
- **F6.** `user_prompt_compose.rs::compose` checks `clears_emit_floor` on the FULL pack (one qualifying item admits everything), THEN dedupes, and never re-checks; there is no per-item relevance threshold and no abstention once the floor cleared. A previously delivered strong hit carries fresh weak items into a later prompt.

Merged before you: `retrieval-contracts` (`ContextPack::unmet_required`, `SelectionReason::GraphNeighbor`, `context::render`), `source-graph-snapshot` (`refresh::snapshot::ensure_snapshot`, `SnapshotPolicy::{LocalCurrent, StageOverlay, BaseOnly}`), `memory-events`. Codex's `loom map` answers from the published base — read those files directly.

## Files you own (write)

- `loom/src/commands/hook/target.rs` — NEW; `loom/src/commands/hook/mod.rs` (declaration)
- `loom/src/commands/hook/user_prompt.rs`, `user_prompt_compose.rs`, `pre_compact.rs`, `reconcile_graph.rs`
- Tests: `commands/hook/tests_user_prompt.rs`, `tests_user_prompt_gates.rs`, `tests_user_prompt_e2e.rs`, `tests_pre_compact.rs`, `tests_reconcile_graph.rs`, plus a NEW `tests_hook_target.rs`

Read-only: `context/**` (including `delivery.rs`, which C2 owns in this stage), `orchestrator/**`, `telemetry/**` (C2), `hooks/*.sh` (C3), `commands/knowledge/**` (C2/D*).

## Contract

### `commands/hook/target.rs`

```rust
pub(crate) struct HookTarget {
    pub work_dir: PathBuf,            // `.loom/work` root (may not exist yet in a checkout)
    pub project_root: PathBuf,
    pub plan: String,                 // `delivery::plan_key(...)` for a stage; `_local` for a checkout
    pub stage_id: String,             // the stage id, or the `map-<dir>` checkout key
    pub pull_stage: Option<String>,   // Some(stage id) inside a stage
    pub overlay: OverlayScope,        // Stage { plan, stage } inside a stage, Local otherwise
}
impl HookTarget {
    /// `for_stage()` (LOOM_STAGE_ID + LOOM_WORK_DIR, validated, stage record loaded) else `for_checkout()` (LOOM_WORK_DIR or ".").
    pub(crate) fn from_environment() -> Option<Self>;
    pub(crate) fn retrieval_config(&self) -> RetrievalConfig;
    pub(crate) fn snapshot_policy(&self) -> SnapshotPolicy;   // StageOverlay { plan, stage } inside a stage, LocalCurrent otherwise
    pub(crate) fn exists(&self) -> bool;                       // work_dir.exists()
}
pub(crate) fn non_empty_env(name: &str) -> Option<String>;
```

`user_prompt.rs`, `pre_compact.rs`, `reconcile_graph.rs` delete their private target structs and use `HookTarget`. `retrieve_for_prompt` sets `query.overlay = target.overlay.clone()`. `reconcile_graph::reconcile` uses `ensure_snapshot(store, &graph_store, project_root, target.snapshot_policy())` for both arms (the checkout arm's clean-HEAD-base / dirty-local-overlay policy now lives inside `LocalCurrent`).

### Admission and abstention (`user_prompt_compose.rs`)

```rust
/// True when `item` earns its place on its own: an exact rung, or the knowledge term floor,
/// or a graph neighbour whose pack still holds an exact-rung item.
fn admits(item: &ContextItem, pack: &ContextPack, config: &RetrievalConfig) -> bool
/// The pack narrowed to admitted items, `omitted` raised by the count dropped; None when nothing is admitted.
fn admitted(pack: &ContextPack, config: &RetrievalConfig) -> Option<ContextPack>
/// Would the hook print this pack for a session that has seen nothing yet? (D2's eval calls this.)
pub(crate) fn would_emit(pack: &ContextPack, config: &RetrievalConfig) -> bool
```

`compose` becomes: `admitted(pack)` → `undelivered(...)` → **re-check** that at least one SURVIVING item is admitted on its own merits (an exact rung or the term floor; a graph neighbour alone does not carry a pack) → byte-ceiling shedding loop → payload. Every `None` is an abstention; `retrieve_for_prompt` records WHY through C2's telemetry (below) and prints nothing. Re-export `would_emit` from `user_prompt.rs` as `pub(crate) use compose::would_emit;` so `crate::commands::hook::user_prompt::would_emit` resolves.

### Telemetry calls (C2 implements the variants in parallel; call them exactly like this)

```rust
crate::telemetry::emit(&target.work_dir, &crate::telemetry::TelemetryEvent::PromptBrief {
    stage_id: target.pull_stage.clone(), session_id: session_id.map(str::to_string),
    items: handed_over.items.len(), estimated_tokens: handed_over.estimated_tokens, omitted: handed_over.omitted.omitted,
});
crate::telemetry::emit(&target.work_dir, &crate::telemetry::TelemetryEvent::PromptAbstained {
    stage_id: ..., session_id: ..., reason: "<floor|all-delivered|over-ceiling|no-target|no-retrieval>".to_string(),
});
```

`emit` returns `Result<()>` and is best effort; ignore the result with `let _ =`. `read_prompt` declines (`machine-generated`, `too-short`) happen before a target exists and are NOT recorded.

## Steps

1. `target.rs` + `tests_hook_target.rs` (env-driven: stage vs checkout, validation failures, `overlay` and `snapshot_policy` values; `#[serial]` and restore env like `tests_user_prompt_e2e.rs::enter_checkout/leave`).
2. Rewire the three hook modules; delete the duplicate `non_empty_env` copies.
3. Admission/abstention in `compose`; telemetry calls in `retrieve_for_prompt`.
4. Tests:
   - `tests_user_prompt_e2e.rs`: `a_stage_session_reads_its_own_stage_overlay_not_the_local_one` — write THREE overlays with conflicting definitions of one distinctive symbol: `_local/map-<dir>` (`ZorbleLocal`), `test-plan/stage-a` (`ZorbleStageA`), `test-plan/stage-b` (`ZorbleStageB`), all through `GraphStore::save_overlay`; with `LOOM_STAGE_ID=stage-a` the emission's pack contains the `stage-a` node and neither of the others; with no stage the `_local` node; with `stage-b`, that one. Assert on `item.id` prefixes. (The "Overlay isolation" contract: a test with one layer cannot catch the mismatch.)
   - `tests_reconcile_graph.rs`: `reconcile_graph_in_a_stage_reconciles_that_stages_overlay_through_ensure_snapshot` (assert the stage overlay file's `generation` is filled after the run).
   - `tests_user_prompt.rs` / `tests_user_prompt_gates.rs`: `a_delivered_strong_hit_does_not_carry_fresh_weak_items` (deliver the only strong item; repeat with the same strong item plus two below-floor items → `None`); `weak_items_are_dropped_before_the_floor_is_judged`; `a_graph_neighbour_is_admitted_only_alongside_an_exact_rung_item`; `would_emit_is_false_for_a_pack_with_only_lexical_items_below_the_floor`.
   - `tests_pre_compact.rs`: adapt to `HookTarget` (behavior unchanged; existing three-recipient tests must still pass).

## Done means

`cargo test --manifest-path loom/Cargo.toml --lib commands::hook::` green; `rg -n "struct (DeliveryTarget|CompactionTarget|ReconcileTarget)" loom/src` returns nothing; `rg -n "fn non_empty_env" loom/src/commands/hook` returns exactly one definition.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib commands::hook::tests_user_prompt_e2e` — skip if unsure.

## Constraints and traps

- The prompt hook has a hard 5-second wall (`hooks/user-prompt-context.sh` runs it under `timeout 5`): `ensure_snapshot` is NOT called on the prompt path; only the detached `reconcile-graph` child calls it. Retrieval stays read-only.
- Session-scoped dedupe keys (`delivery::hook_recipient_id`, `delivered_to_session`, the `LOOM_SESSION_ID` join) are unchanged; only the overlay address and admission change.
- The emit floor stays hook-only by design (`user_prompt_compose.rs` doc): `loom knowledge context`, `eval`, and the spawn brief are not gated — do not move `admits` into `context::pack`.
- C2 owns `telemetry/mod.rs` and defines the variants above with exactly those field names; if compile fails on them, report — do not edit `telemetry/**`.
- No git commands.

## Report back

Files changed and created; the final `HookTarget` definition; the abstention reasons you emit; anything unresolved.
