# A0 — Shared type foundation (behavior-neutral)

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `retrieval-contracts` · Wave 0 (runs ALONE, before A1/A2/A3).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why

Three later units (A1 packing, A2 source-lane expansion, A3 CLI/brief) and two later stages
(`source-graph-snapshot`, `delivery-and-catalog`) all add behavior on top of the same shared types.
Every one of those types is built as a full struct literal in tests spread across five modules, and
several enums are matched exhaustively. Adding the fields and variants in ONE unit, with defaults
and no behavior change, lets every later unit compile against a settled contract and edit only its
own files. Everything you add here is inert until a later unit uses it.

## Files you own (write)

- `loom/src/context/schema.rs` — new variants, new types, new fields, new constants
- `loom/src/context/retrieve.rs` — `StageQuery` gains two fields; `StageQuery::new` sets defaults; nothing else
- `loom/src/context/graph_store/mod.rs` — `GraphLayer` gains `generation`
- `loom/src/context/source_graph/node.rs` — `FileCoverage::Deleted`
- `loom/src/context/render.rs` — NEW: the per-item renderers moved out of `brief.rs`
- `loom/src/context/mod.rs` — `pub mod render;`
- `loom/src/orchestrator/signals/format/brief.rs` — delegates per-item rendering to `context::render`
- `loom/src/context/fuse.rs` — `has_exact_rung` (explicit list, see step 4)
- `loom/src/map/views/mod.rs` — `file_coverage_line` gains the `Deleted` arm
- `loom/src/context/coverage.rs` — only if it matches `FileCoverage` exhaustively (grep first)
- Every struct-literal site listed under "Literal sites" (test files included)

Read-only: everything else. Do not touch `pack.rs`, `rank_source.rs`, `channels.rs`,
`commands/knowledge/context.rs` beyond the one `StageQuery` literal fix, or any hook logic.

## Contract (exact)

Add to `loom/src/context/schema.rs`:

```rust
/// Which lifecycle states a query may retrieve. `Current` is every caller's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecyclePolicy {
    /// `Active` and `Draft` chunks only.
    #[default]
    Current,
    /// Every state, historical material included.
    Historical,
}

/// How a `--require-id` item is represented in the pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequiredRepresentation {
    /// The whole unit, verbatim, never truncated. Reported as unmet when it cannot fit.
    #[default]
    Full,
    /// The excerpt-bounded representation; `ContextItem::truncated` says whether it was cut.
    Compact,
}

/// A required id the pack could not honor under its budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnmetRequirement {
    pub id: String,
    /// Estimated tokens the requested representation needs, wrappers included.
    pub needed_tokens: usize,
    /// Tokens the pack could still spend when the item was considered.
    pub available_tokens: usize,
    pub reason: String,
}
```

Extend existing types (all with `#[serde(default)]` so persisted JSON and packs round-trip):

- `LifecycleState` += `Historical` (Display: `"historical"`; keep `Active` as `#[default]`).
- `SelectionReason` += `GraphNeighbor` (Display: `"graph-neighbor"`). It is NOT an exact rung.
- `ContextItem` += `pub truncated: bool` (default `false`).
- `ContextPack` += `pub unmet_required: Vec<UnmetRequirement>` (default empty).
- `ContextPack::recompute_estimate(&mut self)` — sets `estimated_tokens = BRIEF_FRAME_TOKENS + Σ item.token_count`. Add the method; do NOT call it anywhere yet.
- `pub const BRIEF_FRAME_TOKENS: usize = 128;` doc: "Estimated cost of the Knowledge Brief's header and footer around the items; measured by `brief_tests::frame_cost_is_within_the_constant` (A3 owns that test)."
- `StageQuery` (in `retrieve.rs`) += `pub lifecycle: LifecyclePolicy` and `pub required_representation: RequiredRepresentation`; `StageQuery::new` sets both to `Default::default()`.
- `GraphLayer` += `#[serde(default)] pub generation: String` (empty means unknown; `source-graph-snapshot` fills it) and `#[serde(default)] pub blob_index: BTreeMap<String, String>` (path → the git blob object id whose bytes produced that path's `FileEntry::content_hash`; empty until `source-graph-snapshot` fills it).
- `FileCoverage` += `Deleted` (unit variant; `status()` returns `"deleted"`; `has_symbols()` stays `false` for it).

New module `loom/src/context/render.rs` — move these from `brief.rs` VERBATIM, keeping their behavior byte-identical: `render_knowledge_item`, `render_excerpt_block`, `fence_for`, `render_reasons`, `confidence_word`, `render_source_entry` (the single-entry line renderer; the path-grouping loop `render_source_group`/`render_source_section` stays in `brief.rs` and calls into `render`). Make them `pub(crate)`. Add:

```rust
/// Estimated tokens of the exact text this item renders to inside a brief.
pub(crate) fn rendered_item_tokens(item: &ContextItem) -> usize
```

which renders the item through the same function the brief uses for its kind (`KnowledgeChunk` → `render_knowledge_item`, `SourceNode` → `render_source_entry`) and returns `estimate_tokens(&rendered)`. `brief.rs` and `commands/knowledge/context.rs` (which already reuses `render_excerpt_block`) must import from `crate::context::render` afterwards; `brief.rs` keeps `format_knowledge_brief`, `format_stage_brief`, `render_status_line`, `render_pull_command`, the section builders and every existing `pub(crate)` symbol other code imports (grep `format::brief::` and `brief::` first; re-export moved names from `brief.rs` if anything outside `format/` imports them by that path).

## Steps

1. `schema.rs`: add the types, variants, fields, constant and method above. Update the two exhaustive `Display` impls (`LifecycleState`, `SelectionReason`).
2. `Confidence::from_reasons` (`schema.rs:183`) lists the three exact rungs explicitly — leave the list as it is; `GraphNeighbor` must not promote confidence. Grep `rg -n "SelectionReason::" loom/src` and check every predicate: `fuse.rs::has_exact_rung` (:74), `commands/hook/user_prompt_compose.rs::is_exact_rung`, `context/pack/twins.rs::explicitly_required`, `rank/rungs.rs`. Each must keep treating only `ExplicitId | ExactPath | ExactSymbol | LinkedFrom | StageDependency` as exact. If any predicate is written as "anything other than `Lexical`", rewrite it as the explicit list.
3. `retrieve.rs`: add the two `StageQuery` fields and defaults in `new()`; fix the full struct literal in `loom/src/commands/knowledge/context.rs:88-100` (add both fields with `Default::default()`; change nothing else there).
4. `graph_store/mod.rs`: add `generation` to `GraphLayer` (derives `Default` already). `node.rs`: add `Deleted`; update `status()`; `views/mod.rs::file_coverage_line` (:130) gains an arm rendering `coverage: deleted`; run `rg -n "FileCoverage::" loom/src` and fix every exhaustive match the compiler reports.
5. Create `render.rs` by moving the functions; wire `pub mod render;` in `context/mod.rs`; repoint `brief.rs` and `commands/knowledge/context.rs`. Then `cargo build` — the moved rendering must produce identical bytes (the brief tests in `orchestrator/signals/format/brief_tests*.rs` and `tests_brief_rendering.rs` are the proof; do not edit their expectations).
6. Fix every struct literal the compiler now rejects. Known sites (line numbers advisory):

   - `ContextPack`: `commands/hook/tests_reconcile_graph.rs:277,298`, `commands/hook/tests_user_prompt.rs:64`, `context/tests/delivery.rs:37`, `context/tests/retrieve.rs:47`, `commands/knowledge/tests_context.rs:320`, `orchestrator/signals/format/brief_tests.rs:78`, `orchestrator/signals/tests_brief_rendering.rs:45`, `context/pack.rs:309` (production: set `unmet_required: Vec::new()`).
   - `ContextItem`: `context/pack.rs:91,142` (production: `truncated: false`), `commands/hook/tests_user_prompt.rs:33`, `commands/hook/tests_user_prompt_gates.rs:18`, `context/tests/delivery.rs:14`, `orchestrator/signals/tests_brief_rendering.rs:25`, `orchestrator/signals/format/brief_tests.rs:17,53`, `orchestrator/signals/format/brief_tests_confidence.rs:16,62`, `commands/knowledge/tests_context.rs:157,196,233,282`.
   - `GraphLayer`: `context/graph_store/tests.rs:12,55,246`, `commands/hook/tests_user_prompt_e2e.rs:86`, `context/tests/retrieve_source.rs:72,265`, `commands/run/tests.rs:308`, `orchestrator/signals/tests_brief_e2e.rs:254`, `context/refresh/source_graph.rs:288` (production: `generation: String::new(), blob_index: BTreeMap::new()`).
   - `StageQuery`: `commands/knowledge/context.rs:88`.

   In test literals prefer adding the one field explicitly over `..Default::default()` (these types do not all derive `Default`).
7. Add unit tests in `context/tests/schema.rs`: `LifecyclePolicy` and `RequiredRepresentation` default and kebab-case round-trip; `LifecycleState::Historical` and `SelectionReason::GraphNeighbor` display strings; `UnmetRequirement` JSON round-trip; `recompute_estimate` equals `BRIEF_FRAME_TOKENS` plus the item sum. In `context/tests/fuse.rs` add one test: a candidate whose only reason is `GraphNeighbor` lands in tier 2 (after a tier-1 `ExactSymbol` candidate with a lower raw score).

## Done means

- `cargo build --manifest-path loom/Cargo.toml` succeeds with no new warnings.
- No behavior changed: every existing test passes unmodified except the literal additions in step 6.
- `rg -n "GraphNeighbor" loom/src` shows it only in `schema.rs`, `fuse.rs`'s explicit list (absent from it), and the new tests — nothing awards it yet.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib context::tests::schema` — skip it if unsure; the orchestrator runs the full gate.

## Constraints and traps

- Behavior-neutral: no new logic, no new callers of the new API. A1/A2/A3 build on it in the next wave.
- `has_exact_rung` and `from_reasons` are the two predicates that decide tier-1 fusion and `High` confidence. If `GraphNeighbor` leaks into either, A2's expansion later floods tier 1. The fuse test in step 7 pins this.
- `brief.rs` output is byte-exact-tested; moving functions must not reformat any string.
- `loom map`/`loom knowledge context` cannot show your in-progress edits (they answer from the published base). Read files you changed directly.
- No git commands. Do not create files outside the list above except the two test additions named.

## Report back

Files changed; any struct-literal site not on the list above that you had to fix (the plan author's list was grepped at HEAD but may be short); any predicate you rewrote in step 2; anything unresolved.
