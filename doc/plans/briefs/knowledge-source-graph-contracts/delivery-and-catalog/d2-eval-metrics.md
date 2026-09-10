# D2 — Truthful retrieval metrics: hit rate, real precision, relevant-token fraction, mandatory recall, abstention, rendered cost

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `delivery-and-catalog` · Single wave (parallel with C1, C2, C3, D1).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F15)

`commands/knowledge/eval.rs::aggregate` publishes `precision_at_5 = cases_with_any_expected_hit_in_top_5 / cases_with_expect` — a hit rate, named precision, in both the human line (`Aggregate: precision@5 = ...`) and the JSON key. A pack with one relevant and four irrelevant items scores 1.0. `run_case` calls `retrieve_for_stage` and scores ids only: nothing about the delivered brief's size, whether the hook would even emit it, whether every `--require-id` landed, or how many of the returned tokens were relevant.

Merged before you: `retrieval-contracts` (`ContextPack::unmet_required`, `ContextItem::truncated`, `StageQuery::{lifecycle, required_representation}`, `context::render`). C1 (parallel) exposes `crate::commands::hook::user_prompt::would_emit(pack: &ContextPack, config: &RetrievalConfig) -> bool` — call it by exactly that path.

## Files you own (write)

- `loom/src/commands/knowledge/eval.rs` (split `eval/{cases.rs, metrics.rs, report.rs}` as needed; each under 400 lines), `tests_eval.rs`
- `loom/eval/retrieval-cases.yaml`
- `loom/src/cli/types_memory.rs` — the `Eval` variant's help strings only (no new flags); coordinate nothing else there (D1 owns the enum's new variants)

Read-only: everything else.

## Contract

### Case file additions (all optional, backwards-free — this is an unreleased project)

```yaml
  - name: ...
    query: "..."
    expect: [ids]           # unchanged: any of these in the top 5 = a hit
    relevant: [ids]         # ids judged relevant beyond `expect` (union with expect is the relevant set)
    require_ids: [ids]      # unchanged
    forbid: [ids]           # unchanged
    abstain: true           # the hook must NOT emit a brief for this prompt
    max_rendered_tokens: N  # optional per-case ceiling on the rendered brief
```

Validation: `abstain: true` with `expect` is a construction error; a case must have at least one of `expect`, `forbid`, `abstain`.

### Per-case metrics (`metrics.rs`, pure functions over ids/packs)

- `hit_at_5` (existing boolean) and `mrr` (existing).
- `precision_at_5 = |relevant ∩ top5| / min(5, |top5|)` (0.0 when nothing was returned); `relevant = expect ∪ relevant`.
- `relevant_token_fraction = Σ token_count of returned items in`relevant`/ pack.estimated_tokens` (0.0 when the pack is empty).
- `mandatory_recall = |require_ids present in items| / |require_ids|` (only for cases with `require_ids`); `unmet_required` count from the pack.
- `would_emit: bool` from C1's function with `RetrievalConfig::load(main_root)`; `abstention_correct = case.abstain == !would_emit` (only for `abstain` cases, and — reported separately — for every case as `emit`).
- `rendered_tokens = estimate_tokens(&format_knowledge_brief(&pack, None, "eval"))` (import `crate::orchestrator::signals::format_knowledge_brief`; it is `pub(crate)` from the signals module).
- `forbid_violations` (existing).

### Aggregates and gates (`report.rs`)

Rename the existing field to `hit_rate_at_5` (human: `hit@5`, JSON `hit_rate_at_5`, with `hit_rate_hits`/`hit_rate_applicable`). Add means over applicable cases: `precision_at_5`, `relevant_token_fraction`, `mandatory_recall`, `abstention_accuracy` (over `abstain` cases), `rendered_tokens_mean`, `rendered_tokens_max`. Gates: FAIL when `hit_rate_at_5 < pass_floor` (unchanged meaning), when `forbid_violations > 0`, when any `abstain` case emitted, when any `require_ids` id is unmet, or when a case exceeds its `max_rendered_tokens`. New optional file-level `precision_floor` (default 0.0 so today's file passes) gates `precision_at_5`. The human report prints one line per case: `name  hit=✓  p@5=0.40  rel-tok=0.62  mand=1/1  emit=yes  rendered=812` and the aggregate block; JSON mirrors it.

### Case file

Rename nothing. For the nine `expect` cases add `relevant:` with at least the `expect` ids (add more only when you can name the chunk from the corpus with certainty — do not guess). Add two abstention cases: `conversational-correction-abstains` (query: `no, use the repository version of those files, not the installed copies`, `abstain: true`) and `farewell-abstains` (query: `thanks, that is all for now, nothing else to do here`, `abstain: true`). Add `max_rendered_tokens: 1600` to the two `prose-symbol-collision-*` expect cases (budget 800 → the rendered brief must stay near budget).

## Steps

1. Split the module; keep every existing test in `tests_eval.rs` passing (rename assertions on `precision_at_5` to `hit_rate_at_5`).
2. `run_case` returns a richer `CaseResult` carrying the pack-derived numbers; scoring stays testable with synthetic packs (build `ContextPack`/`ContextItem` literals through the fixtures in `commands/knowledge/tests_context.rs` — copy the two helpers, do not import private items).
3. Tests: `precision_at_5_counts_only_relevant_items_among_the_first_five` (one relevant of five → 0.2; the report's "Quality metrics" row); `hit_rate_and_precision_disagree_on_a_one_relevant_pack` (hit=1.0, p@5=0.2); `relevant_token_fraction_is_zero_for_an_empty_pack`; `mandatory_recall_counts_present_require_ids`; `an_abstain_case_fails_when_the_hook_would_emit`; `an_unmet_required_id_fails_the_run`; `max_rendered_tokens_gates_a_case`; `validate_cases_rejects_abstain_with_expect`.

## Done means

`cargo test --manifest-path loom/Cargo.toml --lib commands::knowledge::tests_eval` green; `rg -n "precision_at_5" loom/src/commands/knowledge` shows only the TRUE precision; `loom/target/debug/loom knowledge eval --json` runs on this checkout (the orchestrator runs it from the main checkout after the merge, not in the worktree — it opens the context store) and reports the new fields.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib commands::knowledge::tests_eval` — skip if unsure.

## Constraints and traps

- `eval` stays a CLI gate, not a `cargo test` (module doc): it reads the live index.
- `would_emit` judges a FRESH session (no delivery ledger); do not try to simulate suppression.
- Keep `pass_floor` semantics attached to hit rate so the checked-in file keeps passing at 0.5 while the new metrics start being reported — tuning is a later human job, not this unit's.
- No git commands.

## Report back

Files changed and created; the final JSON aggregate keys; the case-file diff summary; anything unresolved.
