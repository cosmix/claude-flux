# A3 — CLI surface, stage brief budget and query text, brief frame cost

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `retrieval-contracts` · Wave 1 (parallel with A1, A2; after A0).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why

1. **F12 (wiring).** `RetrievalConfig::stage_brief_budget_tokens` is declared, defaulted (3000), parsed and tested in `loom/src/context/config.rs` and read by nothing. `orchestrator/signals/retrieval.rs` hard-codes `const STAGE_BRIEF_BUDGET_TOKENS: usize = 3000;` and passes it to `retrieve_for_stage`. The advertised control does not control stage brief size.
2. **F1 (CLI contract).** After A1, a required id that cannot fit lands in `ContextPack::unmet_required`. `loom knowledge context` must expose it: a non-zero exit, a human line naming the id and the budget it needs, and the field in `--json`. The caller must also be able to ask for the compact representation and for historical material.
3. **F17 (stage query text).** `build_stage_query_text` concatenates acceptance-criterion COMMANDS (`cargo test ...`) and wiring `source pattern description` triples into the retrieval query, so orchestration vocabulary competes with the task's own words. Drop the command text and the wiring `pattern`; keep the wiring `description`.
4. **F12 (frame cost).** A0 introduced `BRIEF_FRAME_TOKENS = 128` as a provisional estimate of the brief's header + footer; A1 charges it. Its value must be measured by a test that renders the frame.

A0 moved the per-item renderers into `loom/src/context/render.rs`; `brief.rs` delegates to them. Codex's `loom map` answers from the published base and will not show A0's or A1's edits — read those files directly.

## Files you own (write)

- `loom/src/commands/knowledge/context.rs` and `loom/src/commands/knowledge/tests_context.rs`
- `loom/src/cli/types_memory.rs` (the `Context` variant only) and `loom/src/cli/dispatch.rs` (the `Context` arm only)
- `loom/src/orchestrator/signals/retrieval.rs` (and its inline `#[cfg(test)]` tests)
- `loom/src/orchestrator/signals/format/brief.rs`, `brief_tests.rs`, `brief_tests_confidence.rs`
- `loom/src/orchestrator/signals/tests_brief_rendering.rs`, `tests_brief_e2e.rs` (only if an assertion breaks because the query inputs text changed)
- `loom/src/context/render.rs` (only if a render tweak is needed for `truncated` — see step 5)

Read-only: `schema.rs`, `retrieve.rs`, `pack.rs` (A1 is editing it in parallel — do not touch), `config.rs`, `context/untrusted.rs`.

## Contract

### `loom knowledge context`

New flags on the `Context` variant (`cli/types_memory.rs`), threaded through `dispatch.rs` into `knowledge::context::context(...)`:

- `--history` (bool): sets `StageQuery::lifecycle = LifecyclePolicy::Historical`. Help: "Include deprecated, superseded and historical material (default: current knowledge only)".
- `--require-compact` (bool): sets `StageQuery::required_representation = RequiredRepresentation::Compact`. Help: "Represent --require-id items by their bounded excerpt instead of verbatim; the JSON marks them truncated".

Output:

- `--json`: unchanged shape plus `unmet_required` (serde already emits it; no code) and `truncated` per item.
- Human: after the items, one line per unmet requirement: `! required <id> did not fit: needs ~<needed> tokens, <available> were available (raise --budget-tokens or pass --require-compact)`. Under `--explain`, print `truncated: yes` for truncated items.
- Exit status: when `pack.unmet_required` is non-empty, print everything, then `std::process::exit(3)` — in BOTH modes, so a script can rely on it. Document the code in the `Context` help text ("exit 3 when a --require-id could not be honored").

### Stage brief budget

In `orchestrator/signals/retrieval.rs`: delete `STAGE_BRIEF_BUDGET_TOKENS`; in `retrieve_stage_pack` resolve the main project root the way `knowledge_tree_is_empty` already does (`WorkDir::new(work_dir)` → `main_project_root()`, falling back to `work_dir`), load `RetrievalConfig::load(&main_root)`, and pass `config.stage_brief_budget_tokens`.

### Stage query text (F17)

`build_stage_query_text` keeps: id, `stage_type` (Debug), name, description, working_dir, files, artifacts, wiring `description` (only), dependency ids. It drops acceptance criterion commands and wiring `source`/`pattern`. Update `STAGE_QUERY_INPUTS` to: `"this stage's id, type, name, description, working dir, files, artifacts, wiring descriptions and dependencies"` and `describe_wiring_check` accordingly.

### Brief rendering

- `format_knowledge_brief` renders, after the source section and before `Omitted:`, one line per `pack.unmet_required`: `Required but unmet: <id> (needs ~N tokens, M available)` through `inline_safe`.
- A truncated knowledge item's excerpt already ends with `EXCERPT_TRUNCATION_MARKER` (A1 emits it); no extra rendering.
- Frame cost test (`brief_tests.rs`): `frame_cost_is_within_the_constant` — render `format_knowledge_brief` for a pack with zero items, `omitted = 0`, `Some("stage-id")`, `STAGE_QUERY_INPUTS`; assert `estimate_tokens(&rendered) <= BRIEF_FRAME_TOKENS`. If it fails, RAISE `BRIEF_FRAME_TOKENS` in `schema.rs` to the smallest multiple of 16 that passes and report the measured value. (Yes, this is the one edit to `schema.rs` you may make.)

## Steps

1. `types_memory.rs` + `dispatch.rs`: add the two flags; keep argument order stable for the existing ones.
2. `context.rs`: set the two `StageQuery` fields (the literal there already carries them after A0); add the unmet lines, the `--explain` truncated line, the exit code. Keep `context.rs` under 400 lines — move the human printing helpers into `commands/knowledge/context/print.rs` if needed (declare the module the way `context.rs` declares its tests).
3. `retrieval.rs`: budget from config; trim `build_stage_query_text`; update `STAGE_QUERY_INPUTS`; update the inline tests that pin the query text (`build_stage_query_text` has tests near `stage.files = vec!["src/lib.rs"...]`).
4. `brief.rs`: unmet lines; `brief_tests.rs`: the frame test plus `an_unmet_requirement_is_rendered_inline_safe`.
5. Tests in `tests_context.rs`: `an_unmet_required_id_prints_the_needed_budget_and_exits_3` (drive the printing helper, not `process::exit` — factor the exit decision into `fn exit_code_for(pack) -> Option<i32>` and test that), `history_flag_sets_the_historical_policy`, `require_compact_flag_sets_the_compact_representation`, `explain_prints_truncated_for_a_cut_item`.
6. `retrieval.rs` tests: `the_stage_brief_budget_comes_from_config` (write `.loom/config.toml` with `[retrieval] stage_brief_budget_tokens = 700` in a temp project and assert the pack's `budget_tokens == 700`; reuse the temp-project shape from `tests_brief_e2e.rs`), `acceptance_commands_and_wiring_patterns_are_not_query_text`.

## Done means

- `cargo test --manifest-path loom/Cargo.toml --lib commands::knowledge::` and `--lib orchestrator::signals::` green.
- `rg -n "STAGE_BRIEF_BUDGET_TOKENS" loom/src` returns nothing; `rg -n "stage_brief_budget_tokens" loom/src/orchestrator` returns the new read.
- `loom/target/debug/loom knowledge context --help` lists `--history` and `--require-compact` (the orchestrator checks this after the build).

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib commands::knowledge::tests_context` — skip if unsure.

## Constraints and traps

- The prompt hook's `compose` also renders through `format_knowledge_brief`; the unmet line must be `inline_safe` like every other untrusted value (chunk ids come from unvalidated frontmatter).
- `format_stage_brief` (the spawn-time variant) must render the unmet lines too — one code path for both.
- Do not reintroduce a literal budget anywhere; `RetrievalConfig` is the only source, and `retrieve_for_stage` clamps nothing — `config.rs::budget()` already clamps at parse time.
- `tests_brief_e2e.rs` and `tests_user_prompt_e2e.rs` are `#[serial]` and set env vars; if you touch `tests_brief_e2e.rs`, keep the `enter/leave` discipline intact.
- No git commands.

## Report back

Files changed; the measured `BRIEF_FRAME_TOKENS` if you changed it; the final `STAGE_QUERY_INPUTS` string; anything unresolved.
