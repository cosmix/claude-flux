# A1 — Required items, honest budgets, match-centred excerpts, lifecycle eligibility, one evaluate

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `retrieval-contracts` · Wave 1 (parallel with A2, A3; after A0).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (four defects, one packer)

1. **F1.** `loom knowledge context --require-id X --budget-tokens 500` exits 0 without X when X does not fit: `check_require_ids` (`retrieve.rs`, near `reject_unknown_require_ids`) only checks existence, and `select` (`pack.rs`) skips any candidate whose `token_count` exceeds the remaining budget. A required id must be included or reported as unmet with the budget it needs.
2. **F12 (charging).** `select` charges `candidate.token_count` = the chunk's WHOLE body estimate, but the item carries at most a 400-token leading excerpt, and nothing charges the pointer line, reason line, or fences. Budgets are therefore fiction in both directions.
3. **F12 (excerpt).** `bounded_excerpt` (`pack.rs`) always keeps the leading 1600 bytes, so the sentence that matched the query is often outside the excerpt.
4. **F7.** `rank_channel_cached` (`rank.rs`) never reads `chunk.state`; a `state: superseded` file ranks like an active one. Eligibility must be a policy applied before packing.
5. **F14 (retrieval side).** `retrieve_for_stage` fingerprints the knowledge tree twice per call: `resolve_catalog` → `refresh(store, root, true)` → `evaluate`, then `evaluate_state` → `evaluate` again.

A0 already added the types you need: `LifecyclePolicy`, `RequiredRepresentation`, `UnmetRequirement`, `ContextPack::unmet_required`, `ContextItem::truncated`, `BRIEF_FRAME_TOKENS`, `ContextPack::recompute_estimate`, `StageQuery::{lifecycle, required_representation}`, and `context::render::rendered_item_tokens`. Codex's `loom map` answers from the published base and will NOT show A0's edits — read those files directly.

## Files you own (write)

- `loom/src/context/pack.rs`, `loom/src/context/pack/twins.rs`
- `loom/src/context/retrieve.rs`, `loom/src/context/retrieve/channels.rs`
- `loom/src/commands/hook/user_prompt_compose.rs` — ONLY the `carrying` helper (call `recompute_estimate`)
- Tests: `loom/src/context/tests/pack.rs`, `tests/pack_twins.rs`, `tests/pack_source.rs`, `tests/retrieve.rs`, `context/retrieve/channels/tests.rs`

Read-only: `schema.rs`, `render.rs`, `rank.rs`, `rank_source.rs` (A2 is editing it in parallel — do not touch), `fuse.rs`, `commands/knowledge/context.rs` and `orchestrator/signals/**` (A3 owns them).

## Contract

### Eligibility (F7)

In `retrieve/channels.rs`, after ranking and before fusion, drop every knowledge candidate whose chunk `state` is ineligible under `query.lifecycle`, unless its reasons contain `SelectionReason::ExplicitId`:

- `LifecyclePolicy::Current` → eligible states are `Active` and `Draft`.
- `LifecyclePolicy::Historical` → everything is eligible.

Source-node candidates are always eligible. Implement as `fn apply_lifecycle_policy(candidates, chunks_by_id, policy) -> Vec<RankedCandidate>`; keep the lexical cache untouched (the corpus statistics stay whole; only admission changes).

### Required items (F1)

`select` runs two passes over the fused list:

1. **Reservation pass.** Every candidate with `ExplicitId` in `reasons` is built first, in fused order, at the representation `query.required_representation` asks for:
   - `Full` → the item's `excerpt` is the whole chunk body (no cap, `truncated = false`); a source node's whole signature.
   - `Compact` → the bounded excerpt; `truncated` set when it was cut.
   Its cost is `render::rendered_item_tokens(&item)`. If `cost > remaining`, push `UnmetRequirement { id, needed_tokens: cost, available_tokens: remaining, reason: "required item exceeds the remaining budget" }` and do not substitute anything for it. Otherwise include it and charge `cost`.
2. **Optional pass.** The remaining candidates as today, compact representation, each charged `rendered_item_tokens`; a candidate that does not fit is skipped and counted in `omitted`.

`pack` starts `estimated_tokens` at `BRIEF_FRAME_TOKENS` and finishes with `pack.recompute_estimate()`; `within_budget()` keeps its meaning. The twin rule (`details_before_summaries`, `explicitly_required`) is unchanged.

### Charging and excerpts (F12)

- `ContextItem::token_count` = `render::rendered_item_tokens(&item)` — the estimated tokens of the exact text the brief prints for that item, wrappers included. `RankedCandidate::token_count` stays the chunk estimate (ranking still needs it); packing charges the rendered figure.
- `bounded_excerpt(body: &str, terms: &[String]) -> (String, bool)`: when `estimate_tokens(body) <= EXCERPT_MAX_TOKENS` return the whole body, `false`. Otherwise choose the window of at most `EXCERPT_MAX_TOKENS * BYTES_PER_TOKEN_ESTIMATE` bytes, aligned to line boundaries, that starts at the heading line (line 1 of the body) when the best-matching line is within the first window, else at the best-matching line minus one line of lead-in. "Best-matching line" = the line containing the most distinct `terms` (case-insensitive whole-word), ties to the earliest line; with no `terms` matching, the leading window as today. Prefix `[… earlier lines omitted]` when the window does not start at line 1; suffix `EXCERPT_TRUNCATION_MARKER` when it does not reach the end. Never split a UTF-8 char; never split inside a fenced code block if avoidable (end the window before an unclosed fence).
- `terms` are the surviving query terms of the knowledge channel: extend `RankedChannels` with `surviving_terms: Vec<String>` (per channel, union), filled from `Corpus::surviving_terms` in `rank_channels_cached`, carried through `PackRequest` into `pack`.

### One evaluate (F14)

`resolve_catalog` returns `(Catalog, StoreState)`: on a successful `refresh` build the state from the outcome — `structural: outcome.structural`, `semantic: outcome.semantic.freshness`, `catalog_revision: catalog.revision` — and `retrieve_for_stage` uses it instead of calling `evaluate_state`. Keep `evaluate_state` only for the in-memory fallback branch (refresh failed) and the no-knowledge-tree branch.

## Steps

1. `channels.rs`: add `surviving_terms` to `RankedChannels`; add `apply_lifecycle_policy` and call it from `rank_channels_cached` for the knowledge channel. `build_rank_query` is unchanged.
2. `retrieve.rs`: pass `query.lifecycle` and the chunk map into channels; `build_pack_request` carries `surviving_terms` and `required_representation`; restructure `resolve_catalog`/`retrieve_for_stage` per "One evaluate".
3. `pack.rs`: `PackRequest` gains `surviving_terms: Vec<String>` and `required_representation: RequiredRepresentation`; rewrite `bounded_excerpt`; rewrite `select` as the two passes; charge `rendered_item_tokens`; set `truncated`; fill `unmet_required`; start at `BRIEF_FRAME_TOKENS`.
4. `user_prompt_compose.rs::carrying`: replace the manual sum with `narrowed.recompute_estimate()`.
5. Tests (new or updated, names given so the plan's acceptance can filter on them):
   - `tests/pack.rs`: `a_required_full_item_larger_than_the_budget_is_reported_unmet_not_substituted`; `two_required_ids_cannot_be_satisfied_by_packing_only_one` (both listed in `unmet_required` or both present — never a success with one); `a_required_compact_item_is_marked_truncated_when_cut`; `an_item_is_charged_its_rendered_cost_not_its_body_estimate`; `an_excerpt_is_centred_on_the_matching_line_when_it_sits_past_the_window` (put the matching sentence near the end of a 3000-token body and assert the excerpt contains it and the omitted-prefix marker); `the_pack_estimate_includes_the_frame`; update `property_pack_never_exceeds_budget` to the rendered charge.
   - `tests/retrieve.rs`: `a_superseded_chunk_is_not_retrieved_under_the_current_policy_even_with_identical_rare_wording` (two files, same rare token, one `state: superseded`; only the active one appears) and `_is_retrieved_under_the_historical_policy`; `a_required_superseded_chunk_is_still_returned`; `retrieve_for_stage_fingerprints_the_knowledge_tree_once` — instrument via the existing store: assert `store.load_state()` is written once, or count `fingerprint_tree` calls through a test-only counter if you add one (keep it `#[cfg(test)]`).
   - `channels/tests.rs`: `apply_lifecycle_policy_keeps_explicitly_required_ineligible_chunks`.

## Done means

- `cargo test --manifest-path loom/Cargo.toml --lib context::` green including every new test; `commands::hook::` and `commands::knowledge::` still green (A3 owns their new behavior, but your `carrying` change must not break existing hook tests).
- `--require-id` with a too-small budget yields `unmet_required` non-empty and no substitute item (A3 turns that into the CLI exit code).

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib context::tests::pack` — skip if unsure; the orchestrator runs the full gate.

## Constraints and traps

- `GraphNeighbor` candidates (A2) arrive in your fused list as tier-2 lexical-like items; treat them like any optional candidate.
- Do not change `RankedCandidate::token_count` semantics — `fuse` and the tier-2 tie-break read scores only, but `pack::build_omission_summary` uses candidate token counts for `coverage`; keep `Coverage` on candidate estimates and document that `included_tokens` is rendered cost.
- A required id in `Compact` mode that still does not fit is unmet — never fall back to a pointer-only item.
- Keep `pack.rs` under 400 lines: put the excerpt window in `pack/excerpt.rs` and the required pass in `pack/required.rs`.
- No git commands. `loom map`/`loom knowledge context` do not show A0's edits.

## Report back

Files changed; the final `bounded_excerpt` signature; any place `estimated_tokens` is compared to a budget outside your files (grep `estimated_tokens` and list them); anything unresolved.
