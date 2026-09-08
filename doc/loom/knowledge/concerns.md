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

The maintainability gate does not yet prove that every production Rust file is at most 400 lines or
every function at most 50 lines. Existing violations are recorded by exact identity and size in
`loom/maintainability-baseline.txt`; `loom/tests/maintainability.rs` rejects new entries, growth of an
existing entry, or a stale baseline entry. CI runs that gate, so the exception set can only stay flat
or shrink. Treat this as controlled decomposition debt, not as completed decomposition, and remove
an entry whenever its unit is brought under the limit.

### Code Consolidation Needed

> **Full details:** See [conventions.md § Code Consolidation Opportunities](conventions.md#code-consolidation-opportunities-2026-01-29)

Key duplications needing consolidation:

- parse_stage_from_markdown: 4 copies
- branch_name_for_stage: 22+ inline format!() calls
- extract_yaml_frontmatter: 2 copies
- compute_level: 4 copies in status modules

### Debug Output in Production

`eprintln!` statements with 'Debug:' prefix in production code (complete.rs, orchestrator.rs). Should use tracing crate with log levels.

## ReDoS Potential in Plan Pattern Regex

User-provided regex patterns in plan files (failure_patterns, wiring patterns) are compiled and executed without complexity checks. While mitigated by trust model (plan authors = trusted), consider adding regex timeout or complexity limits for defense in depth.

Files: src/verify/baseline/capture.rs:76-79, src/verify/baseline/compare.rs:155-158

## Bootstrap Settings Backup Risk

`bootstrap.rs:write_bootstrap_sandbox()` keeps the settings.local.json backup in memory only (`Option<String>`). If the process is killed between writing sandbox settings and restoring the original, user settings are permanently lost. Low probability since bootstrap is interactive, but a disk-based temp backup would be more robust.

## Hook Debug Logging to /tmp/ (2026-03-31)

Several hooks (worktree-isolation.sh, commit-filter.sh, prefer-modern-tools.sh) hardcode debug log paths to `/tmp/<name>-debug.log`. Under `set -euo pipefail`, if `/tmp/` is not writable (e.g., sandboxed environments), the hook script exits immediately with error. `git-add-guard.sh` already uses a gated `debug()` pattern that only writes when `GIT_ADD_GUARD_DEBUG=1` is set. Other hooks should adopt the same pattern.

## Rust/Shell Heredoc Terminator Divergence

The Rust `strip_embedded_content` in `bash.rs:79` uses `line.trim() == marker` (tolerates indented terminators), while the shell version in `_common.sh:44` uses `$0 == marker` (exact line match). Both fail-safe but should be aligned for consistency.

## BranchMissing Phantom-Merge Risk in merge_handler.rs (2026-04-16)

`handle_merge_session_completed` at line 97-103 treats `MergeState::BranchMissing` as a successful merge by calling `finalize_merge_resolution` which unconditionally sets `merged=true`. This violates the project invariant that daemon-side paths must never write `merged=true` without git ancestry verification.

Scenario: merge session dies, `check_merge_state` returns Conflict/Unknown, branch was deleted without being merged (e.g., manual `git branch -D`), code assumes "branch missing = cleaned up after merge."

Pre-existing issue, not introduced by the merge conflict session lifecycle fix. The `ProgressiveMergeResult::is_success()` method also still classifies `NoBranch` as success, inconsistent with `progressive_complete.rs` treating it as `Blocked`.

## Dead Code: is_knowledge_stage()

models/stage/methods.rs:443 defines is_knowledge_stage() but it is never called. All call sites use direct stage_type comparison. Contains fragile heuristic name matching that duplicates detect_stage_type() logic. Consider removing or consolidating with detect_stage_type and check_knowledge_recommendations.

## BaseConflict Carve-out is Heuristic (2026-04-27)

`attribute_main_repo_merge` carves out `loom/_base/*` merges with a heuristic on the current branch name and on `SessionType::BaseConflict` session metadata. If a base-merge ever runs from a non-`loom/_base/*` branch (manual flow, future refactor) and no `BaseConflict` session is alive, attribution would tie the active merge to the stage whose branch HEAD shows up in `MERGE_HEAD` — leading to a spurious revert.

**Hardening path:** Tag base merges explicitly via session metadata (e.g., a marker file or distinct `SessionType::BaseConflict` always present during the base-merge window) and key the carve-out off that signal alone, not the current branch name. Until then, the heuristic is documented here so future work knows where to look.

## Recovery: `retry --force` races daemon orphan-recovery on existing worktree (2026-05-13)

**Observed:** `loom stage retry --force --context "..."` correctly set `integration-verify` to `Queued`, but on the next daemon poll the orphan-recovery routine in `orchestrator/core/recovery.rs:638-705` saw the (now-stale) session_id, found commits-ahead-of-base on the worktree branch, and immediately re-routed the stage to `NeedsHandoff` (commits_ahead path at `recovery.rs:668`). To the user, the stage looked stuck — they typed `retry`, it was ready for a second, then back to a handoff state with no agent activity.

This is a logically defensible design (commits exist, don't burn tokens redoing them), but the user-visible interaction is confusing. The "fix" — using `retry --force` a _second_ time after acknowledging the handoff — is undocumented in the recovery flow.

**What's needed (pick one or both):**

- `retry --force` should clear `stage.session` before saving, so subsequent orphan recovery doesn't treat the prior session as live and doesn't rerun its decision tree.
- Orphan-recovery should respect a recently-saved "retry intent" marker (e.g., a timestamp on the stage indicating user-driven retry within the last poll interval) and skip its commits-ahead reroute for those.

**Where to look:**

- `commands/stage/skip_retry.rs` (the `retry` command sets Queued at line 122 but leaves `stage.session` populated)
- `orchestrator/core/recovery.rs:633-707` (the orphan-recovery decision tree that re-routes to `NeedsHandoff`)

## Status Dashboard: `started_at` not refreshed on retry, stage appears "stale/orphaned" (2026-05-13)

**Observed:** After a successful `loom stage retry --force` that spawned a fresh session, the status dashboard rendered `integration-verify` as `19h4m · 🔄 · orphaned (stale)` for the duration of the new attempt. The number came from the original (long-dead) `started_at`; the new session was actually `Up About a minute` in podman and actively making tool calls.

**What's needed:** `stage_executor`'s spawn path (or `retry`) should reset `stage.started_at` to `Utc::now()` when a new session is created. The dashboard's "stale" heuristic should key off the new attempt, not the cumulative duration.

**Where to look:**

- `commands/stage/skip_retry.rs::retry` (where retry mutates stage fields)
- `orchestrator/core/stage_executor.rs:291-293` (`begin_attempt(Utc::now())` is already called here — confirm it's the only writer of `started_at` and that it's reached on retry).
- The "stale" indicator emitter — likely in `commands/graph/indicators.rs` or a dashboard renderer.

## `loom pressure` Known Gaps

### Vendored commands / Codex skill install LOCAL-only

`install.sh` installs `commands/*.md` (→ `~/.claude/commands/`) and `codex/skills/pressure/SKILL.md` (→ `~/.codex/skills/pressure/`) ONLY in the local (cloned-repo) branch — `install_commands`/`install_codex_skill` run under the `else` of `is_curl_pipe` in `main()` (~install.sh:619). The remote `curl | bash` install path does NOT ship the `loom pressure` slash commands or the Codex skill. A user who installs via curl-pipe and then runs `loom pressure` will be missing `/pressure`, `/address`, and the `$pressure` skill.

### `loom pressure` real-invocation smokes are manual-only

The two end-to-end smokes — Claude `/pressure` actually editing the plan, and Codex `$pressure` writing the `codex-` sidecar — need network + agent auth and are NOT exercised by `loom stage complete`. They are manual release-validation. Automated coverage is dry-run + 10 unit tests (argv, step order, exit classification, path resolution).

### `git rev-parse --show-toplevel` duplicated 3×

Repo-root resolution is now inlined in three places: `commands/knowledge/spawn.rs` (`resolve_project_root`), `commands/stage/merge.rs` (inline), and `commands/pressure/mod.rs` (`resolve_repo_root`). conventions.md Import Deduplication says extract at 3+ — candidate for a shared `git::repo_root()` helper (deferred during the parallel plan to avoid cross-module merge conflicts).

## Deferred Worktree Cleanup Has Two Residual Edge Cases (2026-07-22)

The main deleted-working-directory hook failure is fixed. Two residuals remain: daemon cleanup can
still race SessionEnd hooks during the short SIGTERM teardown window, and manual `loom stage
complete` with no daemon running defers cleanup until the next recovery pass or an explicit
`loom worktree remove`.

## Knowledge Signals Never Teach Tier-2 (2026-07-28)

`orchestrator/signals/` generates `loom knowledge update <tier-1-file>` guidance, but **no
prefix teaches the `category/slug` tier-2 form**. Verified functionally: tier-2 works
(`loom knowledge update patterns/lock-ordering` creates the file and `INDEX.md` picks it up on
the next knowledge write) — it is simply never advertised to an orchestrated knowledge stage.

Consequence: the hierarchy grows only through the interactive `bootstrap`/`gc` paths, not during
`loom run`. Not a defect in what landed; follow-up stage material.

## `loom knowledge` Has No Delete-Section Verb (2026-07-28)

`update` appends and `replace-section` replaces, but nothing removes a section. Consolidating
several tier-1 sections into one tier-2 topic therefore cannot be completed with the CLI alone —
the migration in this plan replaced the lead section with a summary and had to strip the
remaining N-1 headings with an external script. A `loom knowledge drop-section` (or a
`replace-section --delete`) would close the gap.

## Tier-2 Topic Blurbs Cannot Be Set From the CLI (2026-07-28)

A new topic is seeded with a fixed scaffold — a title derived from the slug and the blurb
"Topic notes for the `<category>` knowledge area" — and user content is appended _after_ it.
`scan_topics` harvests the **first** `#` and `>` lines for the INDEX.md table, so the generic
seeded blurb always wins and every topic reads identically in the index unless the file is edited
afterwards. The index's Blurb column is its main routing signal, so this directly costs
navigability. Wanted: a `--blurb` flag, or have the scaffold defer to a leading `>` line in the
supplied content.

## Remote Releases Do Not Deliver Hooks (PRE-EXISTING, 2026-07-28)

`install.sh::install_hooks_remote` fetches each hook from `${GITHUB_RELEASES}/<name>`, but
`.github/workflows/release.yml` publishes **no hook assets**. A clean remote install now fails
when all downloads miss rather than falsely reporting hook success, but the underlying delivery
contract is still broken: remote installation cannot install token-governance hooks, and
self-update has no hook update path either. This is currently low priority because development
installation (`dev-install.sh`) uses the repository's local hook files.

**Fix shape:** publish and checksum a complete hook bundle (or embed hooks in the binary), verify
its exact `LOOM_HOOKS` inventory before an atomic replacement, and add a clean-install fixture.

## No-Verify Doctrine Block Carries Only a Rust Example (2026-07-28)

The doctrine block must stay byte-identical across the signal, the template, and the hook's
refusal message, so it carries a single scoped-command example — a `cargo` one. A blocked Python
or Go subagent is shown a Rust example. `hooks/subagent-verify-guard.sh` is at the 400-line cap
with no slack, so the fix is to append language-specific examples **after** the pinned block as
explicitly hook-local guidance, the same way the `BLOCKED:` framing line already sits outside it.

## Dead Configurability: `analyze_gc_metrics_with_promoted` (2026-07-28)

No caller passes a non-default `max_promoted_blocks`. Pre-existing and kept deliberately —
Engineering Discipline C says record pre-existing dead code rather than delete it as a drive-by.

## Sandbox Denial Has No End-to-End CI Canary

The generated sandbox policy is covered by unit and flow tests, but nothing proves denial actually
holds against a live Claude runtime: CI has no callable credentialed Claude sandbox runtime, so
Bash, interpreter, build-script, symlink, and file-tool denial cannot be exercised end to end.
That verification is manual release validation.

(Residual of a resolved concern: the fail-open defect itself — generated settings not carrying
sensitive reads into `denyRead`, and `failIfUnavailable` unset — was fixed 2026-08-08.)

## Sandbox `Write(path)` Rules Are Inert (2026-07-31, split 2026-08-17, RESOLVED 2026-08-31)

Claude Code's file permission check consults **only** `Edit(path)`; a `Write(path)` rule parses,
warns at startup, and is then ignored. Both halves are now fixed: `sandbox/settings.rs` emits
`Edit(...)` throughout, and the `Write(.work/**)` rules in a project's `.claude/settings.json`
turned out to be loom's own output from `fs/permissions/constants.rs` (that file is generated and
untracked, not committed config), replaced by `Edit(.work/handoffs/**)`. Loom now also prunes the
legacy grants and migrates inherited `Write(...)` denies on every `loom init`.

→ [Sandbox Write Rules Inert](concerns/sandbox-write-rules-inert.md) for what each half emitted,
where the pruning lives, and the deny-beats-allow caution that shaped the migration rule.

## GC Flags Tier-1 Files for Section Extraction With No Oversized Sections (2026-07-31)

`analyze_gc_metrics` flags a tier-1 file whenever its **total** exceeds `DEFAULT_MAX_TIER1_LINES`
(250), independently of whether any individual section exceeds the section threshold. All six
tier-1 files here currently report `0 oversized sections` yet appear as extraction targets, and
the GC system prompt's first instruction is "Extract oversized tier-1 sections into tier-2 topic
files" — sections the analyzer itself says do not exist.

The agent is left to invent a split with no guidance on where the seams are, which is exactly the
condition under which a restructuring run drops content.

**Fix:** when a file is over budget but has no oversized section, say so in the prompt and ask for
a split proposal by topic cohesion instead of naming a section-extraction target that isn't there.

## Long Codex Runs Starve the Loom Heartbeat (2026-08-07)

A foreground codex-lane run is ONE blocking Bash call, so neither `PostToolUse` nor
`SubagentStop` can refresh the heartbeat until it returns — a codex run longer than
the stage's hung-timeout still produces a spurious, advisory-only `appears hung`
warning. Partly closed 2026-08-27 for the Task-subagent-wait case; the pure codex
case stands. Mitigation is doctrine (bound the task, set `subagent_timeout_secs`),
not a monitor change — raising the global timeout was considered and rejected.

Full detail: [codex-heartbeat-starvation.md](concerns/codex-heartbeat-starvation.md).

## `loom status` "Stale" Badge Is Not Stage-Aware (2026-08-07)

Two independent 300s constants with different consumers:

- **detection** — `orchestrator::monitor::heartbeat::DEFAULT_HUNG_TIMEOUT_SECS`
  (`monitor/heartbeat.rs:21`), per-stage overridable via `subagent_timeout_secs`.
- **display** — `models::constants::STALENESS_THRESHOLD_SECS` (`models/constants.rs:37`), hardcoded
  at `commands/status/data/collector.rs:49` and `commands/status/render/activity.rs:21`.

`subagent_timeout_secs` reroutes only the detection one. A stage with `subagent_timeout_secs: 900`
stays healthy to the orchestrator until 900s but renders `Stale` / "session may be hung" in
`loom status` from 301s — which can push an operator into intervening on a healthy stage.

Flagged twice (implementation, then confirmed by integration-verify) and NOT fixed on purpose:
`determine_activity_status(session, staleness_secs)` takes no `Stage`, so making it stage-aware
means threading the effective timeout through that call site AND the render path — a status
subsystem change with its own test surface, unrelated to the codex lane. Fix if picked up later:
pass `Stage::effective_subagent_timeout_secs()` into `determine_activity_status` and the activity
renderer instead of the constant.

## `evaluate_new_session` Fails a Working Spawn on a Benign `~/.tmux.conf` Warning (2026-08-08)

The rule "any stderr with exit 0 is a failure" is a plan mandate, pinned by unit tests and carried by
an explicit stage decision — so it was **deliberately left as-is**. But tmux prints `~/.tmux.conf`
deprecation warnings to stderr _while creating the session fine_, so one benign warning now: fails the
spawn, kills a **working** server via the abort path, and writes the sticky
`.work/terminal-backend-fallback` marker — disabling tmux for the whole repo until someone runs
`loom run --backend tmux`.

The `has-session` probe that immediately follows is the authoritative signal and would distinguish the
two cases. Gating the stderr rule on that probe is a design call for the plan owner, not an
integration-verify defect.

## `loom attach` Overview Panes Are Live, Writable Agent Terminals (2026-08-08)

The overview's panes are full interactive attach clients (no `-r`), so a stray keystroke goes into a
live autonomous agent's input, and `C-b x` kills the **stage's** pane — the inner servers have no
`remain-on-exit`, only the viewer window does.

Left as-is deliberately: the plan mandates the exact pane string. If the overview is meant to be a
_viewer_ rather than N live terminals, adding `-r` to `attach-session` on the **overview path only**
(never on `loom attach <stage-id>`, which is the intentionally interactive path) is a one-word change.

## Nothing Reaps the Overview Viewer Socket (2026-08-08)

`list_loom_sockets` skips `loom-view-<8hex>` by name so `clean`/`init` do not report it as
unattributable, and the skip comment concedes "Nothing currently reaps this socket". After
`loom attach` the viewer server and its nested clients outlive the operator's detach, and
`loom init --clean` now kills every attributed _session_ server while stranding the viewer.

It is the one socket whose name is a pure function of the repo root, so reaping it carries no
cross-checkout risk — roughly two lines in `cleanup_orphaned_sessions` and `clean::sessions` once
`viewer_socket_name` is crate-visible.

## PreToolUse File Guards Cannot Eliminate Path-Swap Races (2026-08-08)

The canonical `worktree-file-guard.sh` now covers Read, Write, Edit, Glob, and Grep; canonicalizes
paths; compares path components; and rejects both leaf and parent symlinks. This closes the concrete
absolute-path, common-prefix, and final-symlink escapes found in the security review.

The guard is still a check before a separate built-in tool performs the real open. A hostile process
can replace a parent path after the hook returns, so the guard cannot bind its decision to the inode
the tool later uses. Treat it as defense in depth; the host OS sandbox remains the authoritative
boundary. A race-free design requires a dedicated file-operation broker that performs traversal and
the actual open relative to an already-open worktree directory using no-follow semantics. Do not
describe the current hook boundary as TOCTOU-free.

## Completion Broker: Nonce Burn After Transition Can Return Err for a Landed Completion (2026-08-09)

`daemon/server/control_complete.rs::handle_complete_stage` consumes the replay nonce AFTER
`update_stage(...).try_complete(...)`. This ordering is deliberate: a daemon crash between the
transition and the burn is benign — a replay of the same nonce is rejected by
`validate_active_identity` because the stage is no longer `Executing`, so no completion can be
duplicated, and (unlike the old burn-first ordering) none can be lost to a pre-effect burn.

The residual edge introduced by the reorder: a genuine IO failure on the replay-marker directory
(disk full, permissions) occurring after the transition makes the handler return `Err` for a
completion that durably landed. Self-healing in practice — the stage file is `Completed` on disk
and the daemon reads state from disk — but the caller observes a false negative for a succeeded
operation. If this is ever observed in the wild, split the response so callers can distinguish
"completed, replay marker failed" from "not completed".

## `subagent-verify-guard.sh` Still Regexes Raw Command Strings

It is the last hook that matches patterns against the raw command string, so it still cannot tell
an argument's _value_ from an argument's _mention_: text quoted inside a command is scanned as if
it were shell. The shared `loom_tokens_*` helpers it needs already exist in `hooks/_common.sh`.

It was left for last because its failure direction is the mild one — it blocks project-wide
build/test/lint runs by subagents, so a false positive strands a subagent rather than admitting a
dangerous command. That is a reason to do it last, not a reason it is safe.

**Converting the fifth hook is not a mechanical edit.** The 2026-08-26 conversion of three hooks
opened seven bypasses that the raw regexes had blocked, all found only by adversarial probing
against the OLD pattern — a fully green suite showed nothing. Read
`mistakes/shell-command-matchers.md` § "Converting a Raw-String Matcher to Token Scanning
Silently Narrows It" before starting, and budget for the differential testing it describes.

## `loom memory` Is Unusable Without an Initialised `.work` (2026-08-11)

`loom memory note` exits non-zero with `.work directory not found. Run 'loom init' first.`
(`commands/memory/handlers.rs:45`), and even past that gate the recording handlers require a
stage id from `--stage` or `LOOM_STAGE_ID` (e.g. `note()` at handlers.rs:64-66). Neither holds in
an interactive or ad-hoc session.

This collides head-on with doctrine: the mandatory subagent preamble orders every subagent to
record mistakes and decisions via `loom memory`, while auto-memory is prohibited whenever
`doc/loom/knowledge/` exists — which it does here. Agents are therefore ordered to record and
given no working way to do it, and the failure is silent from the orchestrator's point of view.
Three agents lost insights to this in a single session before it was noticed.

**Fixed in-tree 2026-08-11** (`commands/memory/handlers.rs`): the four recording commands
(`note`, `decision`, `change`, `question`) now create `<repo_root>/.work/memory/` when cwd is
inside a git repo, and default the stage to the sentinel `ad-hoc`; `query`/`list`/`show` degrade
to exit 0 without creating anything. Outside a git repo the original error stands, so `.work` is
never scattered into arbitrary directories — see
[`find_repo_root_from_cwd` Returns `Some(cwd)` Outside Any Repo](mistakes.md) for the trap that
guard exists to dodge.

**Still true until the built binary is installed:** a `loom` on PATH from before this change keeps
the old behaviour. When delegating outside a loom run against an older binary, tell subagents
explicitly that `loom memory` will fail, that auto-memory is still forbidden, and that they must
return insights in their final report for the orchestrator to record by hand.

## Two Diverging Copies of the Stage Environment Allowlist (2026-08-17)

The host env allowlist exists twice, and the copies have **already** diverged:

| Copy | Form | Consumer |
| --- | --- | --- |
| `process/environment.rs:14-59` `STAGE_HOST_ENV_ALLOWLIST` | Rust `&[&str]` | `spawn_confined`, i.e. plan-authored commands |
| `orchestrator/terminal/native/wrapper.rs:181-195` `ENV_ALLOWLIST` | embedded shell loop | the native terminal wrapper, i.e. stage agent sessions |

The shell copy omits `CARGO_HOME`, `RUSTUP_HOME`, all eight proxy variables
(`HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY`/`ALL_PROXY` and their lowercase twins) and
all three CA-bundle locations (`SSL_CERT_FILE`, `SSL_CERT_DIR`,
`NIX_SSL_CERT_FILE`). Verified by reading both.

**Concrete failure mode, not hypothetical:** on a host behind a corporate proxy, a
plan-authored acceptance command can fetch and a stage agent session cannot — and
the symptom is a mysterious network failure inside the agent, with no error
pointing at an env allowlist. The same divergence hides a relocated
`CARGO_HOME`.

**Fix:** derive the shell loop from the Rust constant (generate the variable-name
list at build time or render it into the wrapper from the same slice), and add a
test asserting the two agree. Two tables encoding one real-world fact will drift;
the test that matters pins them to each other, not more tests on either side.

## Confined Commands Still Reach a Live Credential Bus (2026-08-17)

`process/environment.rs` withholds `SSH_AUTH_SOCK` with an explicit rationale — it
is a live credential-agent socket, not a location — while forwarding
`DBUS_SESSION_BUS_ADDRESS` (`:33`) and `XAUTHORITY` (`:32`). A session bus address
reaches `org.freedesktop.secrets`, which is a live credential surface by exactly
the argument used to withhold the SSH socket.

**Root cause worth recording:** one allowlist serves two consumers with different
needs. The terminal spawner genuinely needs display and session variables to attach
a window; `spawn_confined` does not need either. **Fix:** split the list into a
common base plus a terminal-only extension, and let `spawn_confined` take only the
base. Doing that also removes the reason the second copy above exists.

See `architecture/execution-containment.md` for the honest statement of what
confinement does and does not guarantee.

## Telemetry Under-Reports the Failures It Exists to Measure (2026-08-17)

`orchestrator/core/stage_telemetry.rs:25` calls `.ok()` on
`load_deliveries`, collapsing a genuine I/O error into the same
`ContextUnavailable { reason: "no delivery record for this session" }` as a real
miss. The file exists to measure how often stages spawn without a context brief, so
folding read failures into "no record" under-reports precisely the failures it is
for. **Fix:** give the error branch its own reason string.

Related, and deliberate rather than broken: `telemetry::read_events` has **no
production caller**, and `.work/` is removed when a plan finishes, so every event
written today goes unread. The intended reader is a future `loom status`/`loom map`
diagnostic. Keep it or delete it consciously — do not assume the data is being used.

## Source-Graph Overlay Cannot Express a Deletion (2026-08-17)

`GraphStore::resolved` computes `overlay ∪ base`, so a file a stage DELETED keeps
its base entry and a view over the resolved graph — `loom map --outline
<deleted-file>` — still prints the stale outline. Fixing it needs a tombstone
concept in `graph_store`, which `refresh/source_graph.rs` explicitly does not own
(its docstring records the gap at `:10-16`).

## `Channel::Source` Is Accepted Everywhere and Consulted Nowhere (2026-08-17)

`--scope source` parses, is advertised in `--help`, is threaded into
`PackRequest.scope`, is serialized into every `ContextPack`, and is included by
`Channel::all()` in the DEFAULT path — while `rank_channels` ranks it over an empty
slice. Every emitted pack therefore names a scope it never searched. The
verification stage added an honest stderr notice rather than removing the value,
because rejecting a currently-accepted flag is a breaking change.

The trail of individually-defensible dead shapes left by shipping the store without
the consumer — `ItemKind::SourceNode` never constructed, `ResolvedGraph::node()`
with zero callers, `ContextItem.excerpt`'s unreachable `None` arm — is catalogued in
`mistakes/store-without-consumer.md`. Cheap detection for each new public item:
**name the production caller**, rather than asking whether the compiler warns.

## Uncalled Path-Escape Validators Read As Protection (2026-08-17)

`sandbox/config.rs` ships three `pub` path-escape validators that **nothing in production
calls**: `detect_path_escape` (`:192`), `validate_paths` (`:276`) and
`is_legitimate_work_access` (`:297`). They are re-exported from `sandbox/mod.rs:10-11`, and
`rg` over `loom/src` finds their only callers are their own tests at `:636-730`.

`test_validate_paths_detects_escape_in_allow_write` (`:711`) asserts `validate_paths` flags an
escape in `allow_write` — but `loom init` runs `validate_sandbox` and `validate_emittable` per
stage, never `validate_paths`, so that escape was in fact emitted unchecked. A green test
standing in for an absent control (`mistakes/tests-that-cannot-fail.md`).

Two things make it invisible: the items are `pub` and re-exported, so `dead_code` never fires
(a fresh build emits ZERO warnings), and the test name reads like coverage of a live
guarantee.

Deliberately not wired in when found: turning `validate_paths` on at plan-validation time
would start REJECTING plans that load fine today, a behaviour change a verification stage
should not make. The hole it describes is now closed **at the point of use** by the
parent-traversal filter in `sandbox/settings.rs`.

**Owner should pick one:** wire it into `loom init` and `plan verify` as a fail-fast check
(preferred — a clear error beats a silently dropped entry), or delete all three and their
tests. **Leaving `pub`-but-uncalled validators is the worst of the three, because it reads as
protection.**

## Sandbox-Widening Fields Need No Author Acknowledgement (2026-08-17)

`plan/schema/validation.rs:45-69` `unsafe_plan_reasons` gates only `enabled: false` and
`allow_unsandboxed_escape`. So `allow_write`, `allow_all_unix_sockets`,
`allow_local_binding` and `linux.enable_weaker_nested` each widen the sandbox with no
acknowledgement required from the plan author. Reviewer-reported and not independently
confirmed — read the code before acting.

## Duplicated Extension-to-Language Table (2026-08-17)

The same real-world fact is encoded twice and nothing pins the copies together:
`language.rs:117-129` maps a `str` to `Option<DetectedLanguage>`, and
`context/extract/lexical.rs:22-36` maps a `Path` to `NodeLanguage`. Both cover `rs`;
`ts,tsx,mts,cts`; `py,pyi`; `go`.

**Failure mode is a silently narrowing capability, not a crash.** Add `.jsx` to one and not
the other and the stage skill recommender classifies a file as TypeScript while the
source-graph tagger labels it `Other` and gives it lexical-only coverage — so
`loom map --outline` shows no symbols for a language loom otherwise claims to support, with no
error anywhere.

**Fix:** have `extract::lexical::language_for_path` delegate to `language::language_for_path`
(make it `pub(crate)`) rather than re-encoding the mapping. The test that matters asserts the
two agree over a shared fixture list — not more tests on either side.

## Low-Severity Cleanups Deferred From the Verification Gate (2026-08-17)

- `refresh/source_graph.rs:111` is a bare `let _ = mark_semantic_stale(..)` while that
  function's own doc (`:341-343`) says callers log and continue. The sibling caller in
  `merge_lifecycle` does log; this one does not.
- `delivery::plan_key` falls back to a `default` namespace when a stage file lacks `plan_id`,
  while `commands/context/record_edit::active_plan` and `MergeLifecycle::plan_id` both read
  `config.plan_id()`. They agree today, but a legacy stage file without `plan_id` would file
  delivery records under a different namespace than the graph overlay and dirty paths. Route
  all three through one derivation.
- `print_freshness_line` is duplicated between `commands/knowledge/status.rs:154-164` and
  `commands/knowledge/context.rs:126-136`, differing only by a two-space indent. Both live in
  the same module; hoist one.

## `loom knowledge` Cannot Rename a Section Heading (2026-08-17, duplicate-heading half fixed 2026-08-19)

`loom knowledge replace-section <file> <heading> [content]` replaces a section's **body** and
keeps the existing heading line. The half of this concern about a DUPLICATE heading is now
fixed: `commands/knowledge/mod.rs::strip_repeated_heading` drops a `## <heading>` line repeated
at the top of the caller's content before splicing, so passing content with its own copy of the
heading no longer double-writes it.

**Still true:** `splice_section` (`fs/knowledge/dir.rs:278`) matches the EXISTING heading and
always re-emits that same heading text — there is no way to change the heading itself through
the CLI, so marking an entry resolved in the repo's `~~strikethrough~~ (RESOLVED date)`
convention still requires a direct file edit for the heading line, even though the body can now
be corrected in place.

**Fix:** either accept a `--heading <new>` flag, or match the OLD heading and re-emit whatever
heading line the content passed in.

## Potential Concerns

- **18 TODO comments** found in source files
- **7 FIXME comments** found in source files

## Open After PLAN-automatic-knowledge-and-source-graph (2026-08-18)

Five smaller open items from this plan: an unbounded whole-file read ahead of the
extraction size cap, four production-dead `KnowledgeDir` methods kept alive only by
each other's tests, a writer/reader plan-key normalisation mismatch, a permission
deny that now reaches the `loom` binary's own child processes, and a fossilized
`LOOM_PERMISSIONS_WORKTREE` grant with no real consumers.

Full detail: [automatic-knowledge-source-graph-followups.md](concerns/automatic-knowledge-source-graph-followups.md).

## Three Hook Files Exceed Rule 17's 400-Line Cap

Over CLAUDE.md Rule 17's 400-line file limit (`wc -l`, 2026-08-26):

| File                             | Lines |
| -------------------------------- | ----- |
| `hooks/_common.sh`               | 1197  |
| `hooks/commit-filter.sh`         | 489   |
| `hooks/subagent-verify-guard.sh` | 416   |

`_common.sh` roughly doubled when the token-scanning helpers landed; it was already over the cap
before that.

Splitting any of them needs a new hook file, and a new hook file needs FOUR registrations: an
`include_str!` const plus a `LOOM_HOOKS` entry in `loom/src/fs/permissions/constants.rs`, the
hooks config builder, BOTH copies of the `all_hooks` array in `install.sh`, and the exact-length
assertion in `loom/src/fs/permissions/tests/hooks_tests.rs`. Miss any one and the hook is
**silently dead** rather than broken — it simply never runs, and nothing reports it. That is why
this is a deliberate, separate change and not a drive-by trim.

## Two Fenced-Code-Block Models Disagree in `fs/knowledge/` (2026-08-26)

`splice.rs`'s `fence_mask` (the level-agnostic `replace-section` splicer) requires a closing
fence's run length to be at least as long as the opener's — CommonMark-correct. `chunker.rs`'s
`fence_marker` (used for retrieval chunking, `~:153-222`) closes on ANY line whose first
non-whitespace characters start with the same delimiter, regardless of run length. They disagree
on, e.g., a ` ``` ` line inside a ```` ```` ```` fence: `chunker.rs` treats it as closing the
fence early (so a `##` line just past it can be lexed as a heading) while `splice.rs` keeps
reading through to the real closer. Consequence: the span `loom knowledge context` reads a
heading from is not necessarily the span `replace-section` will overwrite. Should become one
shared scanner.

## `replace-section` Silently Converts CRLF to LF and Can Drop Trailing Blank Lines (2026-08-26)

`splice.rs::assemble_replacement` rebuilds the file from `base.lines()` (which strips both `\n`
and any preceding `\r`) joined back with plain `\n`, so a CRLF knowledge file is silently
rewritten to LF on any `replace-section` write. When the replaced section runs to EOF, any
trailing blank lines that followed the old section body are also dropped, since the tail beyond
the match is not copied forward. Harmless today — this tree's knowledge files are LF — but worth
knowing so a surprising whitespace-only diff after a `replace-section` call is explainable rather
than alarming.

## `loom knowledge update` Trims Stdin Content But Not Inline Content (2026-08-26)

`commands/knowledge/mod.rs::resolve_content` trims stdin input (`read_content_from_stdin` calls
`buffer.trim()`) but passes an inline CLI argument through untrimmed (`Some(c) => c`). An inline
`loom knowledge update <file> "<content with trailing blank lines>"` call therefore widens the gap
before the next appended section, while the same content piped via stdin would not. Minor, but the
two paths should agree.

## `loom attach <stage-id>` Dies With Its Stage (2026-08-26)

Investigated after a report that the overview never removed dead panes or added new stages. The
report was a ghost — re-tested the same day, the reconciler both adds and kills correctly on tmux
3.6a. The defects the investigation surfaced (debug-only/absent reconciler logging, relative daemon
`work_dir`, `TMUX_TMPDIR` divergence, no retile after kills, no real-tmux test, the never-compiled
`reconcile/steps.rs` copy) were fixed the same day — see `architecture/terminal-backends.md`
§ "Live Overview Reconciliation". What remains is a design property, not a bug:

**Direct attach dies with its stage by design.** `loom attach <stage-id>` `exec`s into the stage's
OWN server (`commands/attach/mod.rs`), whose lifetime is the stage's (default `exit-empty`;
`completion_handler.rs` `kill_session` is only a backstop). The client gets `[exited]` on
completion; no follow/loop exists. Candidate design if this matters: make direct attach a
`select-pane` + `resize-pane -Z` focus on the long-lived overview viewer, so the operator is attached
to the viewer's lifetime, not the stage's (note `split-window` unzooms, and the build should reuse a
healthy viewer instead of `kill-session`-ing it). Daemon log for any attach/reconcile question:
`.work/orchestrator.log`, level fixed by `RUST_LOG` in the shell BEFORE `loom run`.

## `git/worktree/settings.rs` Is Still 637 Lines After Its Tests Moved Out (2026-08-27)

Splitting the inline test module into `tests_settings.rs` / `tests_settings_env.rs` took the file
from 1263 to 637 lines and its ledger entry from 1119 to 637 — a large win, but the production
half is still well over the 400-line guidance and remains a recorded violation in
`maintainability-baseline.txt`.

It is genuinely multi-purpose: worktree `.work`/`.claude`/`CLAUDE.md` scaffold planting, settings
generation and permission merging, env scrubbing, and the git-exclude writer. Those are separable
— the exclude writer in particular (`add_to_gitignore_exclude`,
`add_worktree_exclude_patterns`, and the two `add_settings_local_to_*_gitignore` entry points)
is self-contained and has its own tests.

Not done as part of the sandbox bug fixes because a structural split is not a surgical change and
would have collided with four agents working the same tree. Worth a dedicated stage; note that
any file at or near its ledger cap must be refactored in the same change that grows it (see
`mistakes/sandbox-and-settings.md`).

## iTerm2 Windows Survive Stage Completion — Spawn Never Names the Window (GitHub #7, 2026-08-29)

Teardown closes an iTerm2 window by title, but the iTerm2 spawn arm never names the
window (`git log -S 'set name of'` is empty), so the close query matches nothing and
falls through to killing just the `claude` process — the shell (and window) survive.
A second, independent defect in the same path: teardown addresses `tell application
"iTerm2"` while iTerm2's real scriptable name is `iTerm`. Naming the window alone is
necessary but not sufficient. Terminal.app is unverified (needs a macOS host).

Full detail: [iterm2-window-teardown.md](concerns/iterm2-window-teardown.md).

## Orphan Adoption Only Runs at Daemon Startup (2026-08-29)

`adopt_orphaned_agents` is called from `recover_orphaned_sessions`, which
`orchestrator.rs` invokes exactly ONCE, at daemon startup — not on the poll loop, despite the
"every tick" phrasing that appeared in the brief that requested it. An agent orphaned mid-run
therefore stays invisible until the next `loom run`. That is enough for the incident it was written
for (a killed daemon is restarted by definition), and the spawn-time guard in `start_stage` closes
the duplicate-spawn hole independently, so this is a narrower reach rather than a hole. Making it
per-tick is a one-line addition to the scheduler loop; the pass is already idempotent and pinned as
such by a test, so the only question is cost — it scans `.work/pids/` per Executing stage.

## `get_work_dir()` Trusts Any Path Containing `.worktrees/` (2026-08-29)

`find_worktree_root_from_cwd` (`git/worktree/paths.rs`) is a pure substring match on `.worktrees/`
in the cwd string — it never checks that the directory is a loom-managed worktree. `get_work_dir`'s
first branch then adopts `<that root>/.work` if one merely exists. A user working in any directory
they happen to name `.worktrees/<anything>` that contains a leftover `.work` would silently read
another project's memory and stage state.

Lower severity than the creation-path bug fixed alongside it (`mistakes/ambient-filesystem-trust.md`):
both of `get_work_dir`'s branches only ever RETURN a `.work` that already exists, so this
misattributes reads rather than manufacturing stray directories. Left alone deliberately, because
changing it would alter the reuse and read-only degrade paths that `loom memory list` depends on
during post-compaction recovery (Rule 3b). Fix shape if it is ever worth doing: confirm the
candidate root is a real worktree — a `.git` FILE containing a `gitdir:` pointer — before trusting
its `.work`.

## `commands::memory` Tests Mutate the Process-Global Working Directory (2026-08-29)

Those tests call `env::set_current_dir` and rely on `#[serial]`, which only serializes a test
against OTHER `#[serial]` tests — anything unmarked in the same binary runs concurrently with a
foreign cwd in effect. In each test the `TempDir` is also declared AFTER its `EnvGuard`, so it is
deleted FIRST and there is a window where the process cwd points at a removed directory.

Neither of these caused the 77-failure incident (an ambient impostor `.git` did, see above), which
is why they were not changed. They remain a live hazard: any future test in that binary that
resolves a relative path while a sibling holds a foreign cwd will fail in a way that looks
unrelated to its own subject. Fix shape: inject the working directory instead of mutating the
process's, as `orchestrator/merge_lifecycle/tests.rs:321` already does deliberately for this exact
reason.

## Guard Hooks: Four Design Questions Deliberately Left Open (2026-08-30)

The adversarial review of `read-guard.sh`/`poll-guard.sh`/`_read_discipline.sh` surfaced four
behaviours kept AS SPECIFIED rather than patched, because each needs a spec decision, not a
one-line fix:

1. **The deny threshold may not fit CLAUDE.md's own workflow.** Five identical `git status`
   invocations in one session trigger a deny, yet CLAUDE.md's own commit workflow runs `git
   status` before staging, again after committing, and again before completion — a real session
   following the documented workflow can plausibly hit the threshold.
2. **Ledger TSV keys can interleave across processes.** `_loom_ledger_append` uses the whole
   normalised command line as the TSV key with no atomicity guarantee beyond `PIPE_BUF`; a key
   longer than `PIPE_BUF` can interleave between two in-process subagents that both sanitise to
   `agent_id=main`.
3. **`loom_deny_enabled` is a line-oriented `grep`-style check**, so a TOML multi-line string
   VALUE that happens to contain the literal lines `[hooks]` and `deny_enabled = true` would
   enable the switch even though no real config intended it.
4. **`poll-guard`'s rule-2 `cat` branch is unreachable for any pre-existing `.work` file**,
   because rule 3 (repeat-read escalation) fires first for files the ledger already has an entry
   for — effectively dead code on the common path.

None of these are fixed; each is a live behaviour a future stage should either ratify explicitly
or change with an accompanying spec decision, not patch as an incidental side effect of unrelated
hook work.

## `is_ancestor("1")` Cannot Distinguish "Not an Ancestor" From "Walked Off the Top of a Container"

`hooks/_common.sh`'s `is_ancestor()` exits its walk-up-the-process-tree loop as soon as the
current pid becomes `"1"` or `"0"`, WITHOUT checking whether that final value equals the target
pid — so `is_ancestor(target="1")` is a guaranteed, deterministic `false` regardless of the real
process tree, even inside a container where PID 1 genuinely is an ancestor of everything. This is
useful as a test fixture (a non-ancestor `LOOM_MAIN_AGENT_PID` of `"1"` can never flake true), but
it is also a real edge-case correctness gap for any deployment where the loom main agent's PID
could legitimately be 1. Not fixed — recorded because the test-fixture use depends on the same
behaviour that makes it a latent bug elsewhere.

## Tier-1 Knowledge Housekeeping Backlog

`loom knowledge check --strict` enforces 250 lines per tier-1 file, 40 lines per tier-1
section, and 12 KB for `INDEX.md` (`fs/knowledge/catalog/size.rs`). The remaining work is a
dedicated knowledge-reorganization project, not part of token-governor correctness.

- **All six tier-1 files remain oversized.** Their individual sections are compact; the
  overage is cumulative volume. Moving roughly 80-120 sections safely into tier-2 topics
  should be done file-by-file, preserving links and checking for duplicate headings.
- **`MissingSourceRef` remains the dominant finding.** Resolution needs the FULL path relative to a
  package's src root (e.g. commands/status/data/collector.rs, hooks/spawn-guard.sh) — a bare
  filename (collector.rs) or a partial suffix (ledger/legend.rs, ui/tui/app.rs) fails even when
  that suffix is unique in the tree; only the fully-qualified relative path resolves. The residual is
  mostly bare filenames, hook filenames written without their `hooks/` prefix, and genuinely stale
  citations that cannot be assigned to one package root safely. Canonicalize them to the full
  src-relative form; ambiguity must continue to fail closed.
- **Tier-2 topics with generic blurbs are unfixable from inside a stage session.** A stage session's
  `hooks/worktree-file-guard.sh` hook denies Edit/Write on any path under `doc/loom/knowledge/`, and
  there is no `loom knowledge` CLI verb for the blurb line specifically (only `update`, which appends,
  and `replace-section`, which needs an existing `#{2,6}` heading — the blurb is a bare `>` line under
  the H1). A knowledge-distill stage that creates a new tier-2 topic via
  `loom knowledge update <category>/<slug>` therefore cannot repair its own auto-scaffolded "Topic
  notes for the `<category>` knowledge area." blurb; that repair needs either a `--blurb` flag on
  `update`/a dedicated verb, or a direct file edit from an interactive (non-stage) session.
- **The generated index remains oversized.** This is low-priority navigation cleanup.
- **2026-09-04 reconfirmation (web-dashboard plan's knowledge-distill stage):** the backlog is at
  728 `MissingSourceRef`/oversized-file issues under `./doc/loom/knowledge` on a tree with none of
  this plan's own additions applied (verified against the unmodified HEAD tree via a temporary
  stash), essentially unchanged from prior counts. This plan's own new/edited knowledge content adds
  ZERO net new issues (verified: every bare filename this stage introduced was corrected to its
  fully-qualified src-relative path before completion). `loom knowledge check --strict` — the
  canonical knowledge-distill acceptance criterion per `skills/loom-plan-writer/SKILL.md` — therefore
  still fails on this tree for reasons entirely predating this plan; fixing the backlog itself needs
  the dedicated reorganization project named above, not a per-plan knowledge-distill stage. A stage
  hitting this should confirm (as here) that its own additions are clean, then treat the residual
  count as this pre-existing, already-tracked concern rather than attempting to clear it inline.

## Retrieval Cannot Distinguish 'No Source Graph' From 'Healthy' (2026-09-01)

`degraded_reason` (`context/retrieve/graph.rs:116-124`) returns `None` when
`semantic_revision` is empty — the never-built case — which is the same value it returns for a
healthy graph. In a checkout with no `.work/` (so no context store, so no graph), the Knowledge
Brief therefore prints `Structural: current` with no `DEGRADED` marker while serving
knowledge-only results.

Observed 2026-09-01 in the loom repo itself: the hook was given the query _how does
`ensure_work_symlink` plant the worktree symlink and what calls it_ — written to need the source
lane — and returned five knowledge chunks, 130 omitted, and zero `Channel::Source` items, with a
clean status line. `loom map --outline/--find-all/--impact` all failed with
`.work directory does not exist` at the same time.

This is the shape recorded in [visibility-and-reachability.md](mistakes/visibility-and-reachability.md):
an `Option` that is `None` for two reasons cannot gate a claim about either.

**Deliberately NOT fixed on the spot.** The doc comment at `graph.rs:96-115` shows the predicate
was tuned carefully: it is a live input to
`commands::hook::reconcile_graph::spawn_if_needed`, which fires a detached full-repository
tree-sitter rebuild on `stale OR degraded`. Widening it to report the never-built case as degraded
would make every prompt in an uninitialised checkout start an unbounded background rebuild,
throttled only by the reconcile debounce lock — the exact failure the current shape exists to
avoid.

**Fix shape if taken up:** carry the never-built case as its OWN field rather than folding it into
`degraded`, so the status line can say it without feeding the rebuild trigger. Resolve the question
at its own source, and default in the fail-safe direction (do not claim currency).

## Claude Code Sandbox Protects the Repo's hooks/ Directory (2026-09-02)

The project-root `hooks/` directory is write-protected by Claude Code's sandbox as part of its bare-git-repo rule, so shell writes there fail even when permission config would allow them. [Sandbox Protected hooks/ Directory](concerns/sandbox-protected-hooks-dir.md)

## No `Read(...)` Deny Rule May Exist in Any Settings File (2026-09-04)

Claude Code (verified against 2.1.259) runs two checks on `rg`, `grep`, `egrep`, `fgrep`, `diff`,
`git`, `cp` and `mv`. Both return `ask` with `circuitBreaker: deniedPathInsideDirectory`,
bypass-immune and not classifier-approvable, so auto mode stalls on an operator prompt:

1. **Location check.** Each path argument (`rg` with no path means `.`) is compared with every
   `Read(...)` deny rule's location, the rule's path up to its first wildcard. A location inside or
   equal to the searched directory prompts, so a concrete token rule such as
   `Read(//home/you/src/app/.loom/work/admin.token)` prompts on every search rooted at the project.
2. **`cd` check.** If the compound command contains a `cd` anywhere, even `cd /absolute/path`, and
   the path argument is relative, the location is treated as unknowable and the check prompts
   whenever ANY `Read` or `Read(...)` entry exists under `permissions.deny` in ANY settings source
   (user, project, local, worktree). Shape and location are irrelevant. The predicate is
   `Object.values(alwaysDenyRules).flat().some(r => r === "Read" || r.startsWith("Read("))`.

The 2026-09-03 mitigation, token rules with the project directory globbed out
(`Read(//home/you/src/*/.loom/work/admin.token)`), defeated only check 1 and added a worse defect: on
Linux every `Read(...)` deny is fed to the OS sandbox, whose glob expander takes the wildcard-free
prefix (`/home/you/src`), runs `readdirSync(prefix, {recursive: true})` synchronously on the main
thread and regex-tests every entry, per sandboxed Bash command. Only a prefix of exactly `/` is
refused as too broad. On a `~/src` holding 2.7 million inodes that froze the TUI for long stretches.

Loom therefore never writes a `Read(` entry under `permissions.deny`, anywhere.
`sandbox::settings::generate_settings_json`, `write_settings` and `git::worktree::settings` emit
none; `carry_forward_denies` and the worktree refresh drop every `Read(` deny on shape alone;
`fs::permissions::sync` never promotes one; `fs/permissions/write_rules.rs::prune_loom_read_denies`
strips loom-written ones (token denies in any spelling, and mirrors of
`state_root::CREDENTIAL_DENY_READ_PATHS`) from both `.claude/settings.json` and
`.claude/settings.local.json` on `loom init`; `loom repair` check 14 (`check_read_denies`) strips
them from every loom-written file and reports, warn-only, an operator-authored `Read(...)` deny it
will not remove. `generated_settings_carry_no_read_deny_rules`
(`sandbox/settings/tests_token_rules.rs`) pins the property.

The boundary those rules described is kept by two other layers. `sandbox.filesystem.denyRead` is an
OS list, not a permission rule: it triggers neither check and keeps Bash out of the credential
directories and both tokens (`policy::MANDATORY_DENY_READ` now carries all five credential paths, so
a plan's `deny_read` cannot drop them). `hooks/credential-guard.sh` is a PreToolUse guard on Read,
Glob, Grep, Edit, MultiEdit, Write and NotebookEdit that blocks `admin.token`/`user.token` under any
state root unconditionally and applies the project's `denyRead` list to the file tools. A hook can be
switched off by `disableAllHooks` and shares the check-then-open race noted under "PreToolUse File
Guards Cannot Eliminate Path-Swap Races"; that is the accepted trade for a prompt-free auto mode.
Never reintroduce a `Read(...)` deny of any shape, and never emit a `denyRead` glob whose
wildcard-free prefix lies above the project or above a small home subdirectory.

## Ledger TUI: Tech Debt From the Live-Ledger-Dashboard Plan (2026-09-04)

- **`StageSummary.session_backend` has no reader.** Populated by the collector
  (`commands/status/data/collector.rs:225`, from `session.backend`) and delivered to spec by the payload-parity stage, but
  no ledger column, static renderer line, or attention entry ever reads it — every other occurrence in
  the tree is a test-fixture struct literal. Either give the ledger a BACKEND cell or drop the field
  from `StageSummary`.
- **`render_context_bar` (`commands/status/render/progress.rs:56`, re-exported at `commands/status/render/mod.rs:18`) is dead**, with
  no caller anywhere in the tree — `commands/status/ui/tui/ledger/cells.rs:100-116` independently reimplements the
  same five-cell bar with the same characters. Also apparently unused: `pub use theme::{StatusColors,
  Theme}` (`commands/status/ui/mod.rs:4`, every consumer imports `ui::theme::Theme` directly) and `pub use
  app::TuiApp` (`commands/status/ui/tui/mod.rs:25`, used only by `run_tui` in the same file).
- **The `.`/`..` stage-id path-component check exists in four places at three different strengths**
  (`commands/status/data/execution_models.rs:38`, `commands/memory/handlers/work_dir.rs:99`, `hooks/codex-forward.sh:43`,
  `hooks/spawn-guard.sh:309`) — see mistakes.md for the resulting divergence. Candidate for one shared
  helper.
- **Model-name display normalization (strip `claude-` prefix, strip trailing `-YYYYMMDD`) is
  implemented twice**: `commands/subagents/table.rs::display_model`/`strip_date_suffix` and
  `commands/status/data/execution_models.rs::normalize_model`. The status/ copy was written
  independently because `commands/subagents/table.rs` sits outside the payload-parity stage's declared files. Worth
  collapsing into one shared helper in a stage that owns both paths.
- **`commands/status/ui/tui/app.rs` is pinned at exactly the 400-line file cap** after this plan's fixes (doc comment,
  reconnect retry loop + constants). Any future addition must trim an equal number of lines elsewhere
  or move content into `commands/status/ui/tui/app_tests.rs`. Relatedly, `TuiApp::reconnect_after_read_error` cannot get a
  narrow unit test without mocking a live Unix socket plus a real
  `Terminal<CrosstermBackend<Stdout>>` — `TuiApp` owns both directly with no trait seam — so it is
  covered only by the full build.
- **The ledger footer's error branch has zero test coverage.** `TuiApp.last_error` is wired end to end
  from daemon exit / `Response::Error` through to `panels::render_footer`, but no ledger test ever
  sets it non-`None` (`commands/status/ui/tui/ledger/tests.rs:189`, `commands/status/ui/tui/ledger/layout.rs:295` both pass `None`).

## Web Dashboard Latent Issues

Four issues reviewed and deliberately left unchanged in `loom/src/commands/status/web/`: a mutex-poisoning cascade risk, a cosmetic `GET /ws` status-code mismatch, an inherited partial-frame truncation risk shared with the TUI, and a left-in-place bundle-size warning. The former unused-`DEFAULT_PORT` concern was resolved when bare `--web` gained automatic port fallback. Detail: [concerns/web-dashboard-latent-issues.md](concerns/web-dashboard-latent-issues.md).

## Merge Path Follow-Ups After the Silent-Unmerged Fix (2026-09-06)

Found while fixing the silent `Completed + !merged` outcome (`mistakes/phantom-merges.md`, last entry). Each is a separate change and was left as is.

- `spawn_merge_resolution_sessions` (`orchestrator/core/merge_handler.rs`) still exempts probe failures from `MAX_MERGE_RESOLVER_ATTEMPTS`. A non-final stage now reaches `MergeBlocked` after a failed auto-merge, so a permanently dirty main checkout produces a "Failed to spawn merge resolution session" warning every 5 s until the daemon exits. The 2026-08-17 entry in phantom-merges.md already asks for a cap or an escalation path.
- `loom stage merge` requires the cwd to be inside `.worktrees/` (`commands/stage/merge/preflight.rs::resolve_worktree_paths`). A stage whose worktree is gone but whose branch is unmerged has no loom command that merges it, and the hints printed by the daemon and `loom status` do not say to cd first.
- `verify_merged_true_or_revert` (`orchestrator/core/recovery.rs`) treats a git error from `verify_merge_succeeded` as "not verified" (`unwrap_or(false)`) and reverts `merged` to false, so a transient git failure can flip a merged stage to unmerged.
- `merge_stage` (`git/merge/mod.rs`) checks out the target branch in the operator's main checkout and, on success, leaves it there; only the failure paths restore the original branch.
- `try_auto_merge` is 228 lines against the 50-line function cap and is ledgered at that size.

## Stopwording drops the words a natural-language source-graph question is asked in (2026-09-06)

**Observed:** `loom knowledge context --query "how is the source graph refreshed after a commit"` (`--explain`) drops `source`, `graph`, `commit` as corpus-ubiquitous and returns three unrelated low-confidence chunks; `architecture/source-graph.md#lifecycle-who-builds-it-and-when` — the section that answers it — is not in the pack. The same question phrased with a symbol (`reconcile_base`) ranks the right function first. `loom knowledge eval` still passes at precision@5 = 1.00 because its cases are symbol- or path-shaped.

**Why it matters:** the knowledge-first doctrine now tells every session to pull a question instead of reading; a pull that misses on the project's own vocabulary sends the reader back to paging files.

**Where to look:** `context/rank/corpus/stopwords.rs` (the corpus-derived stopword threshold and its rescue floor, described in `architecture/context-retrieval.md#corpus-derived-query-stopwording-with-a-rescue-floor`), and `loom/eval/retrieval-cases.yaml`, which has no natural-language lifecycle case. A first step is adding that case so the gap is measured before the threshold is tuned.
