# Concerns & Technical Debt

> Technical debt, warnings, issues, and improvements needed.
> Every section here must be an OPEN concern. When one is resolved, DELETE it — do not strike the
> heading and leave the body, which is how eleven dead entries accumulated before 2026-08-26. Git
> history keeps the record and [mistakes.md](mistakes.md) keeps the lesson. If a resolved concern
> leaves a genuine residual, keep only the residual, under a plain heading.
>
> **Related files:** [mistakes.md](mistakes.md) for lessons learned, [architecture.md](architecture.md) for context.

## Architecture Concerns

### Layering Violations (2026-01-29)

> **Full details:** See [architecture.md § Review Findings - Layering Violations](architecture.md#review-findings---layering-violations-2026-01-29)

Critical violations where lower layers import from higher layers:

- daemon imports commands (mark_plan_done_if_all_merged)
- orchestrator imports commands (check_merge_state)
- git/worktree imports orchestrator (hook config)
- models imports plan/schema (type definitions)

## Code Quality Concerns

### Oversized Rust Units Remain Controlled Debt (2026-08-09)

The maintainability gate does not yet prove every production Rust file is at most 400 lines or
every function at most 50 lines; exceptions are tracked by exact identity and size in
`loom/maintainability-baseline.txt` and pinned by `loom/tests/maintainability.rs`, which rejects
new entries, growth of an existing entry, or a stale one, so the exception set can only shrink.
This entry now also covers the wider code-quality/hook-debt cluster: duplicated
extension-to-language tables, hook debug logging to `/tmp/`, hook files over the 400-line cap, the
`subagent-verify-guard.sh` raw-regex gap, and undelivered remote hooks.

→ [Code Quality and Hook Debt](concerns/code-quality-and-hook-debt.md)

### Code Consolidation Needed

> **Full details:** See [conventions.md § Code Consolidation Opportunities](conventions.md#code-consolidation-opportunities-2026-01-29)

Key duplications needing consolidation:

- parse_stage_from_markdown: 4 copies
- branch_name_for_stage: 22+ inline format!() calls
- extract_yaml_frontmatter: 2 copies
- compute_level: 4 copies in status modules

## `loom pressure` Known Gaps

### Vendored commands / Codex skill install LOCAL-only

`install.sh` installs `commands/*.md` (→ `~/.claude/commands/`) and `codex/skills/pressure/SKILL.md` (→ `~/.codex/skills/pressure/`) ONLY in the local (cloned-repo) branch — `install_commands`/`install_codex_skill` run under the `else` of `is_curl_pipe` in `main()` (~install.sh:619). The remote `curl | bash` install path does NOT ship the `loom pressure` slash commands or the Codex skill. A user who installs via curl-pipe and then runs `loom pressure` will be missing `/pressure`, `/address`, and the `$pressure` skill.

### `loom pressure` real-invocation smokes are manual-only

The two end-to-end smokes — Claude `/pressure` actually editing the plan, and Codex `$pressure` writing the `codex-` sidecar — need network + agent auth and are NOT exercised by `loom stage complete`. They are manual release-validation. Automated coverage is dry-run + 10 unit tests (argv, step order, exit classification, path resolution).

### `git rev-parse --show-toplevel` Duplication (2026-08-18, updated 2026-09-10)

Repo-root resolution was inlined in three places: `commands/knowledge/spawn.rs` (no longer exists —
removed entirely when commit `36268adc` collapsed the knowledge CLI to `update`/`context`/`sync`;
it held `resolve_project_root`), `commands/stage/merge.rs` (inline), and `commands/pressure/mod.rs`
(`resolve_repo_root`). The merge-side resolution now lives at `commands/stage/merge/preflight.rs`
(inline) and the pressure-side one at `commands/pressure/paths.rs::resolve_repo_root`. Two
duplicates now remain, below the "extract at 3+" threshold in conventions.md Import
Deduplication — no longer an active consolidation candidate, but the two copies could still drift.

## Deferred Worktree Cleanup Has Two Residual Edge Cases (2026-07-22)

The main deleted-working-directory hook failure is fixed. Two residuals remain: daemon cleanup can
still race SessionEnd hooks during the short SIGTERM teardown window, and manual `loom stage
complete` with no daemon running defers cleanup until the next recovery pass or an explicit
`loom worktree remove`.

## Sandbox Denial Has No End-to-End CI Canary

The generated sandbox policy is covered by unit and flow tests, but nothing proves denial holds
against a live Claude runtime end to end — that verification is manual release validation. This
entry now also covers the wider sandbox/confinement cluster: two diverging stage-env allowlists,
confined commands still reaching a live credential bus, uncalled path-escape validators,
sandbox-widening fields needing no author acknowledgement, ReDoS-able plan regexes, the bootstrap
settings backup risk, and the `Read(...)` deny-rule ban.

→ [Sandbox and Confinement Gaps](concerns/sandbox-and-confinement-gaps.md)

## Sandbox `Write(path)` Rules Are Inert (2026-07-31, split 2026-08-17, RESOLVED 2026-08-31)

Claude Code's file permission check consults **only** `Edit(path)`; a `Write(path)` rule parses,
warns at startup, and is then ignored. Both halves are now fixed: `sandbox/settings.rs` emits
`Edit(...)` throughout, and the `Write(.work/**)` rules in a project's `.claude/settings.json`
turned out to be loom's own output from `fs/permissions/constants.rs` (that file is generated and
untracked, not committed config), replaced by `Edit(.work/handoffs/**)`. Loom now also prunes the
legacy grants and migrates inherited `Write(...)` denies on every `loom init`.

→ [Sandbox Write Rules Inert](concerns/sandbox-write-rules-inert.md) for what each half emitted,
where the pruning lives, and the deny-beats-allow caution that shaped the migration rule.

## Long Codex Runs Starve the Loom Heartbeat (2026-08-07)

A foreground codex-lane run is ONE blocking Bash call, so neither `PostToolUse` nor
`SubagentStop` can refresh the heartbeat until it returns — a codex run longer than
the stage's hung-timeout still produces a spurious, advisory-only `appears hung`
warning. Partly closed 2026-08-27 for the Task-subagent-wait case; the pure codex
case stands. Mitigation is doctrine (bound the task, set `subagent_timeout_secs`),
not a monitor change — raising the global timeout was considered and rejected. The
same topic now also covers the independent `loom status` "Stale" badge mismatch
(two 300s constants, one stage-aware, one not).

Full detail: [codex-heartbeat-starvation.md](concerns/codex-heartbeat-starvation.md).

## Potential Concerns

- **18 TODO comments** found in source files
- **7 FIXME comments** found in source files

## Open After PLAN-automatic-knowledge-and-source-graph (2026-08-18)

Five smaller open items from this plan: an unbounded whole-file read ahead of the
extraction size cap, four production-dead `KnowledgeDir` methods kept alive only by
each other's tests, a writer/reader plan-key normalisation mismatch, a permission
deny that now reaches the `loom` binary's own child processes, and a fossilized
`LOOM_PERMISSIONS_WORKTREE` grant with no real consumers. The same topic now also
covers the retrieval-degradation ambiguity, natural-language stopwording, and two
items resolved since (`Channel::Source` wiring via `rank_source.rs`, overlay-deletion
tombstones via `FileCoverage::Deleted`).

Full detail: [automatic-knowledge-source-graph-followups.md](concerns/automatic-knowledge-source-graph-followups.md).

## iTerm2 Windows Survive Stage Completion — Spawn Never Names the Window (GitHub #7, 2026-08-29)

Teardown closes an iTerm2 window by title, but the iTerm2 spawn arm never names the
window (`git log -S 'set name of'` is empty), so the close query matches nothing and
falls through to killing just the `claude` process — the shell (and window) survive.
A second, independent defect in the same path: teardown addresses `tell application
"iTerm2"` while iTerm2's real scriptable name is `iTerm`. Naming the window alone is
necessary but not sufficient. Terminal.app is unverified (needs a macOS host).

Full detail: [iterm2-window-teardown.md](concerns/iterm2-window-teardown.md).

## Guard Hooks: Four Design Questions Deliberately Left Open (2026-08-30)

The adversarial review of the guard hooks left four behaviours deliberately unpatched pending a
spec decision (the deny threshold, ledger TSV atomicity, `loom_deny_enabled`'s line-oriented
match, an unreachable `poll-guard` branch). This entry now also covers the wider runtime/session
cluster: `evaluate_new_session`'s tmux-warning false failure, live overview-attach panes, the
unreaped viewer socket, path-swap races, `loom attach`'s stage-bound lifetime, daemon-startup-only
orphan adoption, `get_work_dir()`'s substring trust, `is_ancestor("1")`'s deterministic-false gap,
and process-global cwd mutation in memory tests.

→ [Runtime and Session Safety](concerns/runtime-and-session-safety.md)

## Tier-1 Knowledge Housekeeping Backlog

`loom knowledge check --strict` enforces 250 lines per tier-1 file, 40 per section, and a 16 KB
cap on `INDEX.md`; the remaining backlog is `MissingSourceRef` resolution (needs the full
src-relative path) and generic tier-2 blurbs. This entry now also covers the CLI's own rough
edges: knowledge signals never teaching the tier-2 form, no delete-section verb, `replace-section`'s
CRLF/trailing-blank-line quirks, `update`'s stdin-vs-inline trim mismatch, no heading-rename
support, and `loom memory`'s pre-2026-08-11 usability gap.

→ [Knowledge CLI Gaps](concerns/knowledge-cli-gaps.md)

## Claude Code Sandbox Protects the Repo's hooks/ Directory (2026-09-02)

The project-root `hooks/` directory is write-protected by Claude Code's sandbox as part of its bare-git-repo rule, so shell writes there fail even when permission config would allow them. [Sandbox Protected hooks/ Directory](concerns/sandbox-protected-hooks-dir.md)

## Web Dashboard Latent Issues

Four issues reviewed and deliberately left unchanged in `loom/src/commands/status/web/`: a mutex-poisoning cascade risk, a cosmetic `GET /ws` status-code mismatch, an inherited partial-frame truncation risk shared with the TUI, and a left-in-place bundle-size warning. The former unused-`DEFAULT_PORT` concern was resolved when bare `--web` gained automatic port fallback. Detail: [concerns/web-dashboard-latent-issues.md](concerns/web-dashboard-latent-issues.md).

## Merge Path Follow-Ups After the Silent-Unmerged Fix (2026-09-06)

Found while fixing the silent `Completed + !merged` outcome (`mistakes/phantom-merges.md`, last
entry): a probe-failure exemption from `MAX_MERGE_RESOLVER_ATTEMPTS`, `loom stage merge`'s
worktree-cwd requirement, a git-error-as-unverified revert path, and `merge_stage`'s
success-only branch restore. This entry now also covers the wider merge/recovery cluster: the
`BranchMissing` phantom-merge risk, the heuristic `BaseConflict` carve-out, `retry --force` racing
orphan-recovery, `started_at` not resetting on retry, and the completion-broker's
post-transition nonce-burn ordering.

→ [Merge and Recovery Edge Cases](concerns/merge-and-recovery-edge-cases.md)

## Resolved

Concerns that were open and are now closed. Detail and lessons stay where they were recorded;
this is a pointer, not an archive — see git history for the fix commits.

- **`Dead Code: is_knowledge_stage()`** — the function was removed entirely; `rg` finds zero
  references in `loom/src`.
- **`Dead Configurability: analyze_gc_metrics_with_promoted`** — the function was removed
  entirely along with its unused `max_promoted_blocks` parameter.
- **`Channel::Source` Is Accepted Everywhere and Consulted Nowhere** — `context/rank_source.rs`
  now scores source-graph nodes for real. See
  [automatic-knowledge-source-graph-followups.md](concerns/automatic-knowledge-source-graph-followups.md#resolved-channelsource-and-the-source-graph-deletion-gap-2026-08-17-both-resolved-by-2026-09-10).
- **`Source-Graph Overlay Cannot Express a Deletion`** — `GraphStore` now carries
  `FileCoverage::Deleted` tombstones. Same pointer as above.
