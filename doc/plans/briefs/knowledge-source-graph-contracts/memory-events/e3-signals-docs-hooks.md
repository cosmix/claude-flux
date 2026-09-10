# E3 — Truthful maintenance doctrine: distill signal, `/distill`, pre-compaction, handoff memory, plan-writer template

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `memory-events` · Wave 1 (parallel with E2; after E1).
Lane: Claude `loom-software-engineer` (sonnet). Paths are repo-root relative; the crate is `loom/`.

Why Claude and not codex: `hooks/` is write-protected for shell writes by the stage sandbox (bare-git-repo rule); only the Edit/Write tools can change files there. The `.sh` bodies are embedded into the binary through `include_str!` in `loom/src/fs/permissions/constants.rs`, so editing `hooks/*.sh` is the whole change.

## Why (F10, F9 doctrine half)

- `loom/src/orchestrator/signals/cache.rs::generate_knowledge_distill_stable_prefix` tells the curator that "the ENTIRE `.loom/work/` directory — including EVERY `loom memory` entry — is DELETED" the moment distillation finishes. False: `fs/plan_lifecycle.rs::mark_plan_done_if_all_merged` renames and commits; deletion happens only in `loom clean` / `loom init --clean`, and E2 now archives before either. The same prefix names memories the "source of truth" with spot-reads only on ambiguity, and never asks for an outcome per entry.
- `commands/distill.md` (the `/distill` slash command) still says "the files are append-only — add, don't rewrite", contradicting the corrections pass that mandates `loom knowledge replace-section`.
- `hooks/pre-compact.sh` instructs `loom memory note "CONTEXT DUMP: ..."` — a procedural recovery payload into the channel distillation treats as durable knowledge — while already invoking `loom handoff --trigger precompact`, which is where working state belongs.
- `loom/src/commands/handoff/create.rs` reads `memory/<session_id>.md`; journals are `memory/<stage_id>.md`, so a CLI-triggered handoff (the pre-compact hook, CLAUDE.md Rule 3) carries no memory section.
- The plan-writer template and `CLAUDE.md.template` do not know receipts exist.

E1 gave entries ids, `--evidence`, and `--json`; E2 (running in parallel with you) adds `loom memory resolve <id> --outcome ... --target ...` and `loom memory pending [--strict]`. Write doctrine against those exact spellings.

## Files you own (write)

- `hooks/pre-compact.sh`
- `commands/distill.md`
- `loom/src/orchestrator/signals/cache.rs` — `generate_knowledge_distill_stable_prefix` only; `loom/src/orchestrator/signals/format/sections.rs` — the two `## Stage Memory` tables only; their tests (`orchestrator/signals/tests_doctrine*.rs` pin phrases — update the pinned strings you change)
- `loom/src/commands/handoff/create.rs` (+ its tests)
- `skills/loom-plan-writer/SKILL.md` — the knowledge-distill bookend text and its YAML template only
- `CLAUDE.md.template` — Rule 5's memory block and Rule 12 only

Read-only: everything under `fs/memory/**` and `commands/memory/**` (E1/E2).

## Changes

1. **`cache.rs` distill prefix.** Replace the deletion paragraph with the truth: the run state is archived to `<main>/.loom/memory/archive/<plan>-<timestamp>/` at plan completion and before any cleanup, and `loom review` plus the archive are the only readers after this stage. Replace step 3 ("memories are your PRIMARY evidence … only SPOT-READ code when ambiguous") with: memories are CANDIDATE evidence; every code-grounded claim is checked against the final tree before it is written (`loom map --find-all`, `loom knowledge context --query`, then the named lines). Add the receipt protocol as numbered steps: (a) `loom memory show --all --json` to get ids; (b) for EVERY Note/Decision/Question entry record exactly one outcome — `loom memory resolve <id> --outcome promoted --target <file#heading>` after the `loom knowledge update`/`replace-section` that used it, `merged` when it folded into an existing section, `discarded --reason "..."` (duplicate, regenerable, wrong), `deferred --reason "..."` (needs evidence not available now); (c) finish with `loom memory pending --strict` and resolve whatever it lists; (d) the corrections pass stays first. Keep the single-agent mandate and the tier-routing sentence.
2. **`sections.rs` Stage Memory tables.** Add `--evidence <path:line>` to the Mistake/Decision/Surprise rows' example commands (`loom memory note "mistake: ..." --evidence loom/src/x.rs:42`) and one sentence: "Evidence makes the entry checkable at distillation; an entry without evidence is a claim."
3. **`commands/distill.md`.** Rewrite (keep it under 40 lines): corrections first with `replace-section`; then route insights with `loom knowledge update` (tier routing unchanged); every insight taken from a memory gets `loom memory resolve`; finish with `loom memory pending --strict`; state that files are corrected in place with `replace-section` and never merely added to. The stage acceptance greps the file for the phrase the old text used to describe itself (the hyphenated "append" wording) and fails if it survives, so do not quote it, not even to negate it. Mention `loom memory pending` as the standalone queue to run any time, not only at plan end.
4. **`hooks/pre-compact.sh`.** Remove the `CONTEXT DUMP` memory instruction. The intercept message tells the agent: working state is in the handoff loom just wrote (print its path, already parsed at `:109-124`); record only DURABLE lessons with `loom memory note`, never task state. Keep every other behavior (the handoff call, `loom hook pre-compact`, exit codes). Check `loom/src/fs/permissions/constants.rs` still embeds the file (no change needed) and that any test pinning the old string is updated (`rg -n "CONTEXT DUMP" loom hooks`).
5. **`handoff/create.rs`.** Replace the `memory/<session_id>.md` read with `crate::fs::memory::format_memory_for_handoff(work_dir, &stage_id)` (the daemon path in `orchestrator/monitor/handlers.rs:145-148` is the model). Add a test that a CLI handoff for a stage with a journal contains `## Stage Memory`.
6. **`skills/loom-plan-writer/SKILL.md`.** In the knowledge-distill section and its canonical YAML: add the receipt steps (one sentence each) and `- "loom memory pending --strict"` to the stage's `acceptance` with the note that it reads `.loom/work/memory` only and is acceptance-safe. **`CLAUDE.md.template`**: Rule 5's MEMORY block gains `--evidence`; Rule 12 gains one line: "In a knowledge-distill stage every memory ends with `loom memory resolve`; `loom memory pending` is the queue."

## Done means

- `cargo build --manifest-path loom/Cargo.toml` warning-free; `cargo test --lib orchestrator::signals::` and `--lib commands::handoff::` green with the pinned strings updated.
- `rg -n "CONTEXT DUMP" hooks loom/src` returns nothing (no comment may keep the phrase either); the hyphenated "append" phrase is absent from `commands/distill.md`; `rg -n "ENTIRE .loom/work" loom/src` returns nothing.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib orchestrator::signals::cache` — skip if unsure.

## Constraints and traps

- Doctrine strings are pinned by equality tests (`mistakes/doctrine-and-acceptance.md`: "a one-phrase grep proves presence but never agreement"). Update the test AND the text together; do not weaken a test to a substring.
- Hook files are executed by bash and embedded verbatim; keep `set -euo pipefail` semantics and the exit-code contract of `pre-compact.sh` intact.
- E2's exact CLI spelling: `loom memory resolve <EVENT_ID> --outcome <promoted|merged|discarded|deferred> [--target ...] [--reason ...]`, `loom memory pending [--stage] [--json] [--strict]`. Do not invent flags.
- No `loom knowledge` writes (implementation stage); no git commands; no auto-memory.

## Report back

Files changed; the final distill prefix text (verbatim); the pinned-string tests you updated; anything unresolved.
