# K — Tier-1 files become summaries; stale claims corrected; touched topics get verified evidence

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `knowledge-hierarchy-repair` (stage_type `knowledge`, runs in the MAIN checkout, no worktree).
This ONE brief serves the orchestrator and its six `loom-software-engineer` (sonnet) subagents; the per-file assignment table is at the end. Paths are repo-root relative.

## The binary

The installed `loom` on PATH predates this plan. Run `cargo build --manifest-path loom/Cargo.toml` first and use `loom/target/debug/loom` (call it `$LOOM` below) for `knowledge check`, `knowledge annotate`, `knowledge update`, `knowledge replace-section`. Verification-harness lesson: the PATH binary is not this build.

## Why (F11, F8)

Six of seven tier-1 files exceed the coded 250-line limit (`mistakes.md` 1194, `concerns.md` 875, `patterns.md` 799, `conventions.md` 697, `entry-points.md` 576, `architecture.md` 543); seven topic blurbs are still the scaffold text (`> Topic notes for the … knowledge area.`); `architecture/knowledge-hierarchy.md` contradicts the tree (it says `loom knowledge check` does not exist, that no size limits are enforced, that the CLI has five subcommands, and describes an index-staleness check that does not exist) while another section of the same file states the opposite; one link is broken; and `loom knowledge check` still lists live missing source references after `delivery-and-catalog` reclassified examples, runtime paths and external references. A tier-1 file that is an archive is not a summary an agent can afford to read.

## Tools (CLI only — no Write/Edit on `doc/loom/knowledge/**`)

- `$LOOM knowledge update <category>/<slug> -` (body on stdin) creates or appends a tier-2 topic; the scaffold header is healed automatically.
- `$LOOM knowledge replace-section <tier1-or-topic> "<Heading>" -` replaces one `##…######` section in place (body WITHOUT the heading line); if it prints "appended it as a new" the heading did not match — read that line.
- `$LOOM knowledge annotate <target> --blurb "<one line ≤ 80 chars>"` fixes a topic's `>` blurb; `--sources <path> --verified HEAD` records evidence.
- `$LOOM knowledge check --strict` is the gate; `$LOOM knowledge check --json` lists every issue with its file.
- `rg -n '^## ' doc/loom/knowledge/<file>.md` lists a file's sections; `wc -l` measures.

## Orchestrator tasks (before fan-out)

1. Build; run `$LOOM knowledge check --json` and save the issue list to `.loom/work/handoffs/k-issues-before.json` (the handoffs dir is writable).
2. Corrections pass with `replace-section`, in `architecture/knowledge-hierarchy.md`: `Coverage Blast Radius` (the check exists; six subcommands became eight: `update`, `replace-section`, `context`, `eval`, `sync`, `check`, `annotate`, `telemetry`), `Thresholds` (the three size limits ARE enforced by `catalog/size.rs`; `MAX_INDEX_BYTES` is 16 384; blurbs cap at 80), `Audit Rules — the Two Checks Disagree About Link Form` (rename the heading's claim: nine `CatalogIssue` kinds now, `EvidenceChanged`/`UnverifiableReference` are review-only), `INDEX.md Generation` (16 384 / 80), and delete the invented "index staleness textual check" sentences. Verify each claim against the merged tree with `loom map --outline loom/src/fs/knowledge/catalog.rs` and `rg` before writing it.
3. Same pass for `architecture/context-retrieval.md` (required items and `unmet_required`; lifecycle eligibility and `--history`; rendered-cost charging and `BRIEF_FRAME_TOKENS`; match-centred excerpts; source-lane graph expansion; the single evaluate; the stage-budget config now wired; the prompt hook reads its stage overlay through `HookTarget`; per-item admission and abstention), `architecture/source-graph.md` (`FileCoverage::Deleted` tombstones; untracked files in overlays; bases built from committed content; `blob_index`/`generation`; `ensure_snapshot` and the counters; the "Known gap — an overlay cannot express a deletion" paragraph is now history — replace it), `architecture/memory-spool.md` (blurb; ids, evidence, receipts, `pending`, the archive), `entry-points/hooks.md` and `architecture/hook-system.md` (`commands/hook/target.rs`, telemetry events, the skill-trigger scoring change). Sources: the merged code; quote symbol names, not line numbers.
4. Fix the broken link and the remaining live `MissingSourceRef` issues by correcting the text of the section that carries them (`replace-section`).
5. Spawn the six subagents (table below) in ONE message, each with this brief's path and its row; run ONE `loom subagents watch --timeout 3600` in the background; harvest.
6. After harvest: `$LOOM knowledge check --strict` must pass; `wc -c doc/loom/knowledge/INDEX.md` ≤ 16 384; annotate the four architecture topics this plan touched with `--sources` (the primary files: `loom/src/context/pack.rs loom/src/context/retrieve.rs` for context-retrieval; `loom/src/context/refresh/source_graph.rs loom/src/context/graph_store/mod.rs` for source-graph; `loom/src/fs/knowledge/catalog.rs loom/src/commands/knowledge/check.rs` for knowledge-hierarchy; `loom/src/fs/memory/types.rs loom/src/fs/memory/spool.rs` for memory-spool) `--verified HEAD`; run `loom knowledge sync` (PATH loom is fine for sync) and confirm it reports the catalog rebuilt.
7. Record to `loom memory` what was surprising (a section that was wrong in a way the report did not name; a topic that had to be merged with an existing one), then commit and complete.

## Subagent procedure (each owns ONE tier-1 file and ONE category directory)

Preamble: CLAUDE.md Rule 5 fence applies (no git, no `loom stage complete`, no verification beyond ONE `$LOOM knowledge check --json | rg -c "<your file>"`). Use ONLY the CLI above to write; never Write/Edit under `doc/loom/knowledge/`.

1. Read your tier-1 file in full (it is the one file you must read whole) and `fd . doc/loom/knowledge/<category>` to see existing topics.
2. Group the file's `##` sections into COHESIVE topics by subsystem or concept — not one topic per section, not one giant topic. Target 4-10 topics per file, each 60-250 lines, named `<category>/<slug>` with a slug that says what the topic is about (`merge-and-worktree-lifecycle`, not `misc-2`). When an existing topic already covers the theme, APPEND to it with `update` and shrink its summary in tier-1 instead of creating a near-duplicate.
3. For each topic: write the body (every fact from the source sections preserved, including conditions, dates and file paths; reorganised, not paraphrased away), `update` it, then `replace-section` each moved tier-1 section with a 2-8 line summary ending in `→ [Title](<category>/<slug>.md)`; when several sections collapse into one topic, keep ONE summary section in tier-1 and replace the others with a one-line pointer to it. Give every new topic a real blurb (`annotate --blurb`, ≤ 80 chars, says why the topic matters and when to read it).
4. Stop when your tier-1 file is ≤ 250 lines and every section ≤ 40 lines. Do not touch other tier-1 files or other categories.
5. Report: topics created/extended (slug + line count), tier-1 line count before/after, anything you could not place.

## Assignment table (DISJOINT territories; workers NEVER spawn subagents)

| Worker | Owns (write, via CLI) | Read-only | Notes |
| --- | --- | --- | --- |
| K1 | `mistakes.md`, `mistakes/**` | everything else | 1194 lines, 34 existing topics — expect mostly appends into existing topics |
| K2 | `concerns.md`, `concerns/**` | everything else | resolved concerns become a `## Resolved` pointer line, not a topic |
| K3 | `patterns.md`, `patterns/**` | everything else | |
| K4 | `conventions.md`, `conventions/**` | everything else | one existing topic (`commits`) |
| K5 | `entry-points.md`, `entry-points/**` | everything else | pair each entry point with the subsystem topic it belongs to |
| K6 | `architecture.md`, `architecture/**` (blurbs and sizes only — the orchestrator owns the four correction passes above; K6 must NOT rewrite `context-retrieval`, `source-graph`, `knowledge-hierarchy`, `memory-spool`) | everything else | 21 existing topics |

`INDEX.md` regenerates on every write under a parent-directory lock; concurrent writers are safe. `stack.md` (116 lines) needs no split.

## Done means (stage acceptance)

`cargo build --manifest-path loom/Cargo.toml` then `loom/target/debug/loom knowledge check --strict` passes; `! rg -qF "Topic notes for the" doc/loom/knowledge/INDEX.md`; `! rg -qF "five subcommands" doc/loom/knowledge/architecture/knowledge-hierarchy.md`; every tier-1 file keeps its `##` headings.

## Constraints and traps

- Never delete a fact to make a file shorter; move it. The report's F11 is about organisation, not volume ("retaining substantial durable knowledge is compatible with sending very little of it").
- A heading passed to `replace-section` must match the existing `##` text exactly (case and punctuation); the CLI tells you when it appended instead.
- `loom knowledge update` APPENDS; use it for new topic bodies and for adding to a topic, never to "fix" text.
- The catalog's `DuplicateHeading` issue fires on a repeated normalised H2 inside ONE file — when merging sections into a topic, rename colliding headings.
- Memory: `loom memory note`/`decision` for surprises only; this is a knowledge stage, so `loom knowledge` writes are the deliverable, not a violation.
