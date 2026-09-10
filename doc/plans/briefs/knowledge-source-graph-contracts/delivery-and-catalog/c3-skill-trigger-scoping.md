# C3 — Skill recommendations scoped by the request, not by repository technology

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `delivery-and-catalog` · Single wave (parallel with C1, C2, D1, D2).
Lane: Claude `loom-software-engineer` (sonnet). Paths are repo-root relative.

Why Claude and not codex: `hooks/` is write-protected for shell writes by the stage sandbox (bare-git-repo rule; see `doc/loom/knowledge/concerns/sandbox-protected-hooks-dir.md`); only the Edit/Write tools can change files there. `hooks/skill-trigger.sh` is embedded into the binary through `include_str!` in `loom/src/fs/permissions/constants.rs`, so editing the script is the whole change.

## Why (F17, adjacent skill injection)

`hooks/skill-trigger.sh` (Python, despite the extension) promotes every skill matching a DETECTED repository type straight to the qualification bar: `_add_project_matches` sets `scores[skill] = max(scores.get(skill, 0), MIN_SCORE)` with `MIN_SCORE = 2`, and `_rank` qualifies on `score >= MIN_SCORE`. So a prompt about a plan or a review still receives Go, CI, React and TypeScript suggestions purely because those technologies exist in the tree, and for codex the rendering says to read each matched skill in full. `MAX_SUGGESTIONS = 8`.

## Files you own (write)

- `hooks/skill-trigger.sh`
- `loom/tests/integration/hooks_skill_project.rs` and `loom/tests/integration/hooks_skill_trigger.rs`. Note `hooks_skill_project.rs::both_clients_suggest_all_detected_types_without_prompt_keywords` pins the CURRENT promotion behavior — invert it into `a_detected_repository_type_alone_recommends_nothing` rather than deleting it; `frontend_paths_and_nested_cwd_do_not_suggest_backend_skills` and the catalog/index tests must stay green.
- `loom/src/fs/permissions/constants.rs` only if a doc comment there describes the promotion rule

Read-only: `loom/src/commands/hook/project_types.rs` (the Rust delegate that reports project types; unchanged), everything else.

## Changes

1. **Detection is a tie-breaker, never a qualifier.** In `_add_project_matches`, a detected repository type adds ONE point to a skill's score, written EXACTLY as `scores[skill] = scores.get(skill, 0) + 1` (the stage's wiring check greps for that expression; do not introduce a constant) — below `MIN_SCORE` — and records its `repo:<kind> (<path>)` marker as today. A repo-type skill therefore qualifies only when the prompt itself matched at least one of its keywords. Keep `_rank`'s bar at `MIN_SCORE`.
2. **Fewer, better suggestions.** `MAX_SUGGESTIONS = 5`.
3. **Codex rendering.** Keep "read in full" only for skills whose prompt keywords matched (score from keywords ≥ `MIN_SCORE`); a skill that qualified with one keyword plus detection renders as `... -- read <path> if the task touches <kind>` (the kind from its `repo:` marker).
4. **Header text.** The block header becomes: `SKILL MATCH: skills matching this request (keyword hits shown; repo: markers are context, not a reason to load).`
5. Update the doc comment at the top of the script describing the scoring.

## Tests

`loom/tests/integration/hooks_skill_project.rs` drives the script (read it first; it already fakes `loom hook project-types` output or sets `LOOM_BIN`). Add or adapt:

- `a_detected_repository_type_alone_recommends_nothing` (prompt "finish this document" in a tree detected as `react` + `typescript` → no `SKILL MATCH` block, or a block without those skills);
- `a_keyword_hit_plus_detection_qualifies_and_marks_the_repo_context`;
- `codex_rendering_says_read_in_full_only_for_keyword_matches`;
- `at_most_five_suggestions_are_rendered`.

If a `hooks/tests/*.sh` script also exercises the hook, update it the same way (Edit tool only).

## Done means

`cargo test --manifest-path loom/Cargo.toml --test integration hooks_skill_project` green (that test target self-skips without `python3`; do not add a hard dependency); the four cases above pass; `rg -n "max\(scores.get" hooks/skill-trigger.sh` returns nothing.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --test integration hooks_skill_project` — skip if unsure.

## Constraints and traps

- Edit/Write tools only under `hooks/`; `sed`, `chmod`, and shell redirection there fail with "Operation not permitted" and can look like a permissions problem — they are not.
- Preserve the JSON envelope (`hookSpecificOutput.hookEventName = UserPromptSubmit`) and the fail-open behavior (every failure prints nothing and exits 0).
- Do not touch keyword weights (`2` for phrase or name match, `1` otherwise); the change is what detection contributes.
- No `loom knowledge` writes; no git commands; no auto-memory.

## Report back

Files changed; the final scoring rule in one sentence; anything unresolved.
