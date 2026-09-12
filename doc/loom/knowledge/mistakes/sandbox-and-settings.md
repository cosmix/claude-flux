# Sandbox And Settings

> Sandbox path rules, permission sync, excludedCommands matching, and settings env leaking between main repo and worktrees.

## Sandbox: Contradictory Path Rules

**Mistake:** `merge_config()` added `doc/loom/knowledge/**` to both `allow_write` and `deny_write`.
**Fix:** Removed auto-add. Knowledge writes go through `loom` CLI (outside sandbox). Same path must never appear in both.

## Permission Sync: Three Related Bugs

**Mistake:** (1) `copy_file_with_shared_lock` overwrote worktree permissions instead of merging. (2) Permissions with parent-relative or worktree paths filtered out. (3) Sync skipped when acceptance failed.
**Fix:** (1) Merge both sets before writing. (2) Transform to portable relative paths. (3) Sync unconditionally before checking acceptance.

## Knowledge Prose Staleness After Sandbox/Permission-Mode Changes (2026-05-14)

**What happened:** After changing `default_mode_for()` in `sandbox/config.rs` to return `AcceptEdits` for Standard and IntegrationVerify stages (previously `Auto`), three knowledge file locations still referenced the old `auto` default:

1. `architecture.md` — Security Model section said `Standard/IntegrationVerify → auto`
2. `entry-points.md` — Remote Control §1 table said `Standard / IntegrationVerify → Auto`
3. `patterns.md` — Sandbox permission_mode Resolution table showed `auto` for both types

These stale entries would have misled future agents into using `permission_mode: auto` when the default at the time was `accept-edits`. (Note: the default was later reverted back to `auto` for all four stage types on 2026-07-01 — see architecture.md / patterns.md — so this entry stands only as a staleness lesson, not a statement of the current default.)

**Why:** The implementation stage correctly updated Rust source + tests, but did not search knowledge files for old values. Knowledge files are not compiled, so no tool catches the mismatch.

**Prevention:** After changing any `default_mode_for()`-style constant or sandbox default:

1. `rg -l "auto\|Auto" doc/loom/knowledge/` — find knowledge files with the old value
2. Correct each stale entry in place with `loom knowledge replace-section <file> "<heading>"
   "<body>"` — `loom knowledge update` only appends and would leave the old value above the new
   one. Inside a worktree stage the knowledge files stay closed to Write/Edit
   (`hooks/worktree-file-guard.sh`), but the CLI writes through: the sandbox grants
   `doc/loom/knowledge/**` unconditionally (`sandbox::config::apply_knowledge_write_grant`). A
   non-knowledge stage records the find as a `stale-knowledge:` memory instead, for the
   knowledge-distill stage to apply (CLAUDE.md Rule 12)
3. Verify with `rg "permission.mode" doc/loom/knowledge/` that all entries agree

**Generalization:** Any plan that changes an enumerated default (permission modes, stage-type behavior, config field defaults) MUST include a step that searches `doc/loom/knowledge/` for old values and corrects them. This applies even when the code change is a single-line constant update.

## Sandbox excludedCommands: Bare Names Are Matched Exactly, Not as Prefixes (2026-05-26)

**What happened:** Every worktree stage failed at `loom stage complete` with `Read-only file system (os error 30)` writing to `.work/sessions/`, `.work/signals/`, and `.work/stages/`. `.work` is a symlink resolving to the main repo (outside the worktree), so the OS sandbox treats it as read-only. The loom CLI was supposed to be exempt because `default_excluded_commands()` returns `["loom", "git"]`, but the exemption never applied.

**Why:** Claude Code's sandbox matcher (`pK8`/`XR_` in the binary) classifies each `excludedCommands` entry:

- `"loom:*"` → **prefix** → matches `loom` AND `loom <anything>`
- `"loom *"` → **wildcard** → matches `loom <anything>` (NOT bare `loom`)
- `"loom"` → **exact** → matches ONLY the literal command line `loom` with zero args

`generate_settings_json` emitted bare `"loom"`, classified as **exact**, so `loom stage complete <id>` never matched and ran _inside_ the sandbox → EROFS. This regression surfaced on Linux once Claude Code (v2.1.150) enforced the native bubblewrap sandbox; the code's macOS-era comment misattributed it to "excludedCommands does NOT bypass OS-level filesystem restrictions."

**Prevention:** Never repair this by broadening an entry to `"<cmd>:*"`: prefix-wide exclusions move extensible CLI, VCS, interpreter, and build behavior outside the host sandbox. Treat every matcher assumption as a security boundary and verify it against the actual runtime.

**Current resolution (2026-08-08):** Plan-configurable `excluded_commands` are rejected and generated stage settings do not emit broad exemptions. Required orchestration operations need a narrow, structured control-plane boundary; they must not regain access by excluding the Loom CLI. `permissions.allow` entries control prompting only and do not provide an OS-sandbox escape.

## Worktree-Isolation Hooks Gated on LOOM_STAGE_ID, Which Leaks Into Plain Sessions (2026-05-28)

**What happened:** `worktree-isolation.sh` (and `worktree-file-guard.sh`) decided "are we in a loom worktree?" solely via `if [[ -z "${LOOM_STAGE_ID:-}" ]]; then exit 0; fi`. `LOOM_STAGE_ID` is exported into the worktree session's shell (pid_tracking.rs) and persists in the user's interactive shell environment afterward. A normal Claude Code session in the **main** repo on `main` then had `LOOM_STAGE_ID` still set, so the hook activated and blocked ordinary commands — e.g. any Bash command line merely _containing_ the substring `.worktrees/` (like an `rg`/`ls` that references another stage's dir) was rejected as "cross-worktree access," even though the session was nowhere near a worktree.

**Misleading signal:** `LOOM_STAGE_ID` being set _looks_ like proof you're executing a stage. It isn't — env vars outlive the process that set them. The hook even had a comment acknowledging `LOOM_STAGE_ID` "can be stale" but still used it as the activation gate.

**Why:** Worktree membership is a property of **location** (`<repo>/.worktrees/<stage-id>/`), not of an env var. Gating on an env var that leaks conflates "a loom run happened in this shell once" with "this command is running inside a worktree right now."

**Prevention:** Decide worktree membership from the working directory (cwd inside `.worktrees/<stage>/`), or from `LOOM_WORKTREE_PATH` only when it points at an existing `.worktrees/` dir. Never gate isolation enforcement on `LOOM_STAGE_ID` alone. Derive the current stage from the worktree path (`basename`), not from the possibly-stale env var.

**Fix:** Added `loom_current_worktree()` to `hooks/_common.sh` (returns the worktree root by cwd/`LOOM_WORKTREE_PATH`, else non-zero). Both `worktree-isolation.sh` and `worktree-file-guard.sh` now gate on it and derive the stage from the path. `worktree-file-guard.sh` now also sources `_common.sh`. Remember to reinstall hooks (`install.sh`) after editing — the runtime copy lives at `~/.claude/hooks/loom/`, separate from the repo source (see "Source vs Installed: Editing Wrong File").

## Worktree-Relative Escape Deny Rules Leak Into Main-Repo settings.local.json (2026-06-02)

**What happened:** The default sandbox config (`default_deny_read`/`default_deny_write` in `plan/schema/types.rs`) bakes in worktree-escape rules — `../../**` and `../.worktrees/**`. The **worktree** settings generator, `sandbox::write_settings(config, target)`, was being pointed at the **main repo root** by two callers: `commands/repair.rs::fix_sandbox_settings` (`loom repair --fix`) and `orchestrator/core/stage_executor.rs:438` (knowledge-stage spawns, which run in the main checkout). At a worktree (`.worktrees/<stage>/`) `../..` is the repo root — the intended isolation boundary — but at the repo root `../..` is the repo's **parent**, typically `$HOME`. So `Read(../../**)` denied all of `$HOME` (including `~/.gitconfig` → git lost its identity → commits failed) and `Write(../../**)` denied writes across the whole home dir.

**Misleading signal:** The bug is invisible inside worktrees, because there the exact same string is _correct_. It only bites when Claude runs at the repo root (interactive sessions, knowledge stages). A prior partial fix made `generate_settings_json` filter `Read(../…)` (because it also leaks into the macOS OS sandbox), which silenced the git-read symptom — but the **Write side was never filtered** (a comment called it "harmless," true only in a worktree), so `Write(../../**)` survived and kept denying `$HOME` writes. Three fossils prove an old file was written by an older binary: tilde-_expanded_ creds (`Read(/home/u/.ssh/**)`), the un-filtered `Read(../../**)`, and bare `excludedCommands: ["loom"]`.

**Why:** Path-traversal rules are _relative to wherever `settings.local.json` lives_. They are meaningful only in a worktree. Reusing the worktree-shaped generator for the main repo writes rules that resolve to a completely different (and dangerous) location. Worktree-ness is a property of **location**, not something the generator should assume — same root lesson as the `LOOM_STAGE_ID` hook bug above.

**Prevention:** Before writing path-traversal deny/allow rules, ask "relative to _which_ directory will Claude Code resolve these?" Never emit `../`-based rules into a settings file that can live at the repo root. A worktree never _depends_ on inheriting these from main — it regenerates them relative to itself at spawn (`write_settings(worktree.path)`), the create-time copy + refresh union only _adds_, and the worktree hooks enforce isolation independently. So stripping them from the main repo is safe.

**Fix:** `sandbox/settings.rs::write_settings` now computes `target_is_worktree(path)` (a `.worktrees` path component, or a symlinked `.work`) and calls `strip_worktree_escape_denies(&mut config)` for non-worktree targets, so the rules are emitted _only_ where `../..` means the repo root. This guards every main-repo caller at once. `merge_existing_permissions(.., is_worktree)` also scrubs stale `Write(../…)`/`.worktrees` entries from an already-polluted main file (the Read-side filter was already unconditional). The fold-back path (`fs/permissions/sync.rs`) already drops `../`/`.worktrees` via `transform_worktree_path`, so it needed no change.

## settings.local.json `defaultMode: "auto"` is silently ignored — must pass `--permission-mode` on the CLI (2026-07-01)

**What happened:** Every loom stage was supposed to start in `auto` permission mode (the default for all four stage types), but sessions actually started in `default` mode and prompted for every action — defeating autonomous execution. Loom set the mode ONLY by writing `permissions.defaultMode: "auto"` into each worktree's `.claude/settings.local.json` (via `generate_settings_json` / `apply_default_mode`, and again via the hooks generator). Nothing passed `--permission-mode` on the `claude` command line.

**Misleading signal:** The value `"auto"` is correct — `claude --help` lists it among `--permission-mode` choices (`acceptEdits`, `auto`, `bypassPermissions`, `default`, `dontAsk`, `plan`), and `apply_default_mode` emitted the right camelCase string. Loom's own tests asserted `defaultMode: "auto"` was present in the generated JSON, so the settings file _looked_ correct. The bug was the DELIVERY MECHANISM, not the value.

**Why it broke:** Claude Code v2.1.142+ **deliberately ignores `permissions.defaultMode: "auto"` when it comes from project or local settings** (`.claude/settings.json` / `.claude/settings.local.json`) — a security measure so a checked-in repo cannot grant itself auto mode. Auto from those files is dropped silently (no error), and the session falls back to `default`. `auto` is honored ONLY from the `--permission-mode` CLI startup flag, `~/.claude/settings.json` (user settings), or managed settings. (This gating is specific to `auto`; `acceptEdits`/`plan`/`default` ARE honored from local settings, which is why the bug hid — only auto was affected.) Confirmed against the installed binary (v2.1.197) and the official docs (code.claude.com/docs/en/permission-modes).

**Prevention:** To make a loom-spawned session START in a given permission mode, pass `--permission-mode <mode>` on the `claude` CLI (done in `build_claude_command`, resolved in the unified `spawn()` from `merge_config(read_plan_sandbox, stage.sandbox, stage.stage_type)`). Do NOT rely on `settings.local.json` `defaultMode` for `auto`. When a Claude Code setting "isn't taking effect," check the docs for file-scope gating (project/local vs user/managed) before assuming loom emits it wrong — the value can be right while the _source file_ is ignored. Note `auto` also has account/model/provider requirements (Opus 4.6+/Sonnet 4.6+, enabled on the account); an unsupported account falls back regardless of how the mode is requested.

**Fix:** `build_claude_command` now emits `--permission-mode {mode}` (before the positional prompt) using the resolved mode; `settings.local.json` still carries `defaultMode` (harmless, honored for non-auto modes). Unit test `build_claude_command_passes_permission_mode_before_prompt`.

## Claude Code Applies the MAIN Repo's settings.local.json env to Worktree Sessions (2026-07-23)

**What happened:** After the 2026-07-22 identity-scrub fix shipped, worktree sessions on kairos still ran with `LOOM_STAGE_ID=knowledge-bootstrap`: all 1,476 tool events and every lifecycle hook event across the whole 5-stage run carried the FIRST stage's identity, and the only heartbeat file written was `knowledge-bootstrap.json`. Verified live on the `knowledge-distill` session: the claude process env (via `/proc/<pid>/environ`) had the CORRECT wrapper-exported IDs, and the worktree's own settings files were clean (env = `LOOM_WORK_DIR` only) — yet its SessionStart hook logged the stale pair, which existed in exactly one file on the machine: the MAIN repo's `.claude/settings.local.json`.

**Why (two compounding causes):**

1. Claude Code (observed on v2.1.217) applies the **main repository's** `.claude/settings.local.json` `env` block to sessions running in **linked worktrees**. Settings env overrides process env, so the wrapper's correct exports are shadowed by whatever the main-repo file carries. Scrubbing the worktree-side settings files (the whole thrust of the 2026-07-22 fix) is therefore necessary but NOT sufficient. The per-repo values prove the source: the loom repo's sessions get loom's stale pair, kairos sessions get kairos's — a user/managed file can't produce repo-specific values.
2. Nothing in the run path heals a previously polluted main file: `ensure_loom_hooks_local` self-heals only on `loom init`/`loom repair`, and the permission fold-back (`fs/permissions/sync.rs`) rewrites the main `.claude/settings.local.json` mid-run (observed mtime seconds before a spawn) while leaving the stale `env` block intact. Pre-fix pollution therefore persists indefinitely. The main repo's committed-scope `.claude/settings.json` can carry the same pollution from even older loom versions and is never scrubbed by any path.

**Misleading signals:** clean worktree settings + correct wrapper exports made all spawn-side code look exonerated. `rg` over `.claude/` silently skipped `settings.local.json` because it is gitignored — use `rg -uu` when searching ignored config files. Sandboxed diagnosis shells run in a PID namespace, so `ps -p <host-pid>` / `/proc/<pid>/...` false-negatives made live processes look dead.

**Prevention:** Treat the MAIN repo's `.claude/settings.json` and `.claude/settings.local.json` as env sources for ALL sessions, including worktree ones. Per-session identity must be scrubbed from the main-repo settings files in the RUN path — at daemon startup and in every code path that rewrites those files (the sync fold-back especially) — not only on `loom init`/`repair`.

**Fix:** three-site run-path healing. (1) `scrub_main_repo_settings_identity(repo_root)` (`fs/permissions/settings.rs`) scrubs BOTH main-repo settings files, called from `prepare_repo_for_run` (`commands/run/checks.rs`) so every `loom run` — background and foreground — heals before spawning; (2) `merge_permissions_with_lock` (`fs/permissions/sync.rs`) scrubs while holding the fold-back lock, so every stage completion re-heals mid-run; (3) `migrate_hooks_to_local` (`ensure_loom_permissions`) drops identity keys from `settings.json` on init/repair. `LOOM_WORK_DIR` is stable per-repo and deliberately survives.

## Worktree Settings Are a Whole-Object Rebuild — Unemitted Keys Vanish (2026-08-07)

**What happened:** `.claude/settings.local.json` is not merged, it is REBUILT.
`generate_settings_json` (`sandbox/settings.rs:246`) starts from `json!({})` and assigns exactly
three top-level keys — `sandbox` (`:367`), `permissions` (`:435`, always present via
`apply_default_mode` at `:440`), and `worktree` (`:452`) — then `write_settings` overwrites the
whole file (`:197`). Only two things survive from the previous contents: `permissions.allow`/`deny`
(`merge_existing_permissions`, `:187`) and the two-key allowlist
`PRESERVED_SETTINGS_KEYS = ["enabledPlugins", "extraKnownMarketplaces"]` (`:580`, applied at `:191`).
Every other top-level key — `env`, user-authored `hooks`, `hasTrustDialogAccepted`, anything another
Claude Code feature wrote there — is silently dropped. This happens in worktrees AND in the main
repo root (`stage_executor.rs:373` and `:584`, `commands/repair.rs:879`).

**Why:** the rebuild is deliberate — loom owns the sandbox and permission blocks and must not
inherit drift from a previous run. But it makes the file hostile to every _other_ writer, and it
fails silently: no warning, no diff, the key is simply gone on the next stage spawn.

**Prevention (detection rule):** when a Claude Code feature configured through settings works in the
main repo but NOT inside a loom worktree, do not debug the feature — check whether
`generate_settings_json` emits that key at all:

```bash
rg -n '"<yourKey>"' loom/src/sandbox/settings.rs      # emitted anywhere?
rg -n "PRESERVED_SETTINGS_KEYS" loom/src/sandbox/settings.rs   # or carried forward?
```

Neither hit means the key is dropped every spawn, and no amount of re-configuring the feature will
survive.

**Fix:** either add the key to `PRESERVED_SETTINGS_KEYS` (a foreign key loom carries forward) or
emit it from `generate_settings_json` (a key loom owns). `preserve_unowned_keys` skips any key the
generated object already contains (`:595`), so generated always wins — the allowlist can never be
used to smuggle privileges past loom's own sandbox/permission blocks. Negative-control tests at
`settings.rs:1695-1742` pin exactly that: `enabledPlugins` and `extraKnownMarketplaces` carry
forward, a seeded `env` key and an arbitrary unknown key are both dropped.

**Related trap:** `git/worktree/settings.rs:97-104` copies the main repo's `.claude/settings.local.json`
wholesale into a new worktree _before_ the rebuild, so a main-repo local-scope key appears to
propagate — then loses everything outside the allowlist on the first stage spawn. The copy is not
evidence that the key survives.

## A Plan's `allow_write` Cannot Grant a Subprocess OS-Level Write Access (2026-08-08)

**What happened:** the tmux work needed `tmux` to `mkdir` its own socket directory, and reached for a
plan-level sandbox `filesystem.allow_write` entry to permit it. It has no effect on a subprocess, by
design — and the reason is not obvious from the plan schema.

**Why — it is inert on both layers:**

1. **OS layer.** `sandbox/settings/policy.rs` emits `sandbox.filesystem.allowWrite`, but only ever
   with loom's own fixed entries (`CODEX_SANDBOX_WRITE_PATHS`) — a plan's `allow_write` is never
   copied into it.
2. **Tool layer.** Per `concerns.md` ("Per-Stage Sandbox `Write(path)` Rules Are Inert"), Claude Code's
   file permission check consults **only** `Edit(path)`; a `Write(path)` rule parses, prints a startup
   warning, and is then ignored.

So `allow_write` is expressed in the one tool-permission form Claude Code ignores, and never reaches
the OS sandbox.

**Amended 2026-08-10 — the reason given here for withholding `allowWrite` was wrong.** This note
claimed emitting `allowWrite` makes macOS `sandbox-exec` over-restrictive about **reads**, blocking
`~/.gitconfig` and `~/.claude/shell-snapshots/`. That is the signature of the _deny-leak_ bug
documented above ("A Worktree-Only Escape Rule Applied at the Repo Root"), which was fixed
separately by filtering `Read(../…)`/`Write(../…)`; the blame was misattributed. The settings schema
is explicit that `allowWrite` means "additional paths to allow writing within the sandbox, merged
with paths from `Edit(...)` allow rules" — additive, and OS-enforced for child processes. It is the
documented, recommended lever for exactly this, preferred over `excludedCommands`.

**Prevention (corrected 2026-08-26 — the claim below was stale):** to give a **subprocess** (codex,
tmux, any non-Claude-tool binary) write access to a path outside the worktree,
`sandbox.filesystem.allowWrite` is the lever. Plan `allow_write` DOES now reach it: `filesystem_settings`
in `sandbox/settings/policy.rs` copies every plan `allow_write` entry straight into the OS-enforced
`allowWrite` grant (pinned by a test near `sandbox/settings.rs:719`) — this note previously claimed
"nothing copies it there" and that wiring it through was a "live follow-up"; both were wrong by the
time this correction was written. The remaining reasons a subprocess write can still fail are: (1) the
path escapes the worktree with `../` — filtered out by both emitters; (2) the path did not exist at
session start — the sandbox skips a listed path that is not there; (3) a deny entry shadows it (deny
beats allow). Package-manager caches (bun, npm, cargo, uv, go, ...) no longer need a plan entry at
all — they are granted to every stage automatically; see `sandbox/package_caches.rs` and the
"Package-Manager Caches Are Granted To Every Stage" section of `architecture/execution-containment.md`.
Note the deny-leak asymmetry documented above still holds: `denyWrite` leaks into the OS sandbox from
`permissions.deny` as well.

## `excludedCommands` Does Not Reliably Bypass the OS Sandbox for Compound Commands (2026-08-08)

`excludedCommands` entries (`tmux:*`, `cargo:*`) in `.claude/settings.local.json` only take effect when
the command **literally starts with** the excluded token. A script beginning with a variable
assignment before `tmux ...` still runs sandboxed, and tmux's own `mkdir` for its socket dir then fails
with `Operation not permitted` outside the `allowOnly` paths. Full `cargo test ...` invocations do
bypass it reliably.

**Consequence for tmux work specifically:** you cannot smoke-test tmux from a Bash tool call inside a
loom worktree without `dangerouslyDisableSandbox`. The sandbox allows unix sockets only under
`/tmp/tmux-*/**` and writes only under `/tmp/claude`, `$TMPDIR` and the worktree — but `/tmp/tmux-<uid>`
does not exist and `mkdir` on `/tmp` is denied, so every socket dir you _can_ create is one tmux
_cannot_ bind in. Validate tmux behaviour through `cargo test` (the e2e suite works around it per-test
via `tests/e2e/tmux_backend.rs`'s `TmuxTmpDirGuard`), not raw shell tmux.

**Detection:** `couldn't create directory /private/tmp/tmux-<uid> (Operation not permitted)` means
sandbox, not a tmux bug.

## A Credential That Must Be Read Cannot Express a Narrow Capability (2026-08-11)

**What happened:** no worktree stage could complete through its only sanctioned path. `hooks/loom-control-complete.sh` runs the completion broker, which called `read_user_token()`, which `sandbox/settings.rs` denies to every worktree agent (S-1). The failure surfaced as `trusted completion broker could not read .work/user.token`, and the stage simply could not finish.

**Why:** both halves were individually correct and nobody reconciled them. `.work/user.token` authorizes EVERY User-capability RPC, so handing it to a stage agent is privilege escalation and denying it is right. But completing its own stage is the one RPC that agent is supposed to make. One global secret cannot say "this caller may complete its own stage, and nothing else" — the capability it grants is fixed by the token, not by who presents it.

**Prevention:** when a deny rule and a required read are generated by the same tool, they are one decision, not two — check both directions before shipping either. More generally: if a capability needs to be scoped to a caller, a shared secret is the wrong instrument. Ask whether identity can carry it instead.

**Fix:** `daemon/server/peer_identity.rs`. `SO_PEERCRED` (`LOCAL_PEERPID` on macOS) gives the connecting pid from the kernel at `connect(2)`, which no request body can forge; the caller must be that session's recorded process or a descendant, with the same start-time verification the backends use against pid reuse. `authorize_preface` returns `PendingPeerIdentity` instead of a flat refusal for a User request with no valid token, `handle_client_connection` admits that outcome for `CompleteStage` alone, and the handler finishes the check once the body names a session. The S-1 deny stays exactly as it was.

**Two properties worth preserving if this is ever touched:** the request-type gate lives in `handle_client_connection`, not inside each arm, so a NEW User request added later is refused by default rather than silently inheriting the peer path. And `caller_is_inside_session` fails closed on `Unverifiable`, unlike the liveness helpers that deliberately fail open — liveness errs toward "still running" so nothing is reaped on unread evidence, while authorization must err the other way.

## A Credential Tied to the Daemon Locks the Operator Out When the Daemon Is Down (2026-08-11)

**What happened:** `loom stage complete <stage> --no-verify` failed with "the daemon credential could not be read" on a project whose daemon was stopped — exactly when an operator most needs to force-complete a stuck stage.

**Why:** the proof is HMAC'd with `.work/admin.token`, which is published at daemon start and removed when the daemon is not running. No daemon ⇒ no verifier ⇒ no obtainable proof, for anyone, operator included. The gate was written as though the credential were always available.

**Prevention:** when a check depends on ephemeral state, ask what it does in that state's absence. A gate that cannot be satisfied is not a stricter gate; it is an outage.

**Fix:** privileged completion skips the proof requirement when no credential exists at all. Safe because the daemon's absence removes a stage agent's ability to ACT, not merely its credential: an agent completes through the daemon broker (which requires the daemon) or by writing `.work/stages/*.md` directly (which `denyWrite: .work/**` forbids). With no daemon it can do neither, whatever the authorization returns — while an unsandboxed operator can still write, which is precisely the asymmetry the proof was standing in for. `refuse_operator_inside_a_session` still turns an agent away by name first.

## A `--settings` Path Must Survive the Wrapper's `cd`, and the Existence Guard Must Resolve Where the CONSUMER Will (2026-08-17)

**What happened:** every spawned stage session died within ~15s with no log output,
surfacing only as `Process no longer running` in the crash report, then retry, then
`Blocked`. The wrapper script `cd`s into the worktree before `exec`ing claude, but
`native/capsule.rs` built `--settings` from a `cwd` that arrives RELATIVE
(`./.worktrees/<stage>`, derived from `Stage::worktree_path`). After the `cd` the flag
resolved to `<worktree>/.worktrees/<stage>/.claude/settings.local.json`, and claude exited
before startup with `Error: Settings file not found`.

**Why it hid:** the `is_file()` guard in front of the flag PASSED. A relative path is
resolved against the _daemon's_ cwd — the main repo — where the file genuinely exists. So
the guard confirmed a file the spawned process would never open: the checker and the
consumer resolved the same string against two different roots. `wrapper.rs` had an
`absolute()` helper doing exactly the right thing for the `cd` line, with a doc comment
reading "Paths are absolutized because the script may `cd` elsewhere" — the capsule simply
never used it.

**Why tests missed it:** the capsule unit tests only ever passed an already-absolute path
(`/w/.claude/settings.local.json`), so the relative case — the only case production
produces — was never exercised.

**Prevention:** any path handed to a process that will `cd` must be absolutized at the
point it is built, through the SAME helper the `cd` target uses, so both resolve to one
root. Absolutize BEFORE the existence check, never after — a guard that resolves against a
different cwd than its consumer is worse than no guard, because it reports success. When a
value crosses into a subprocess with a different cwd, unit-test the relative input
explicitly; an absolute-only fixture proves nothing about the production path.

**Detection:** a session that dies inside ~15s with an empty log is almost always the
`claude` argv itself failing, not the agent. Read the generated wrapper — it is named
`<pid-key>-wrapper.sh` under `.loom/work/wrappers/` — and run its `exec` line by hand from
the worktree; the flag error surfaces immediately, where the crash report only says
`Process no longer running`.

## `loom memory` Was Unwritable in Every Sandboxed Stage (2026-08-18)

**What happened:** `loom memory note` failed with `Read-only file system (os error 30)` in
every worktree stage, for as long as the sandbox has been enforced. Stages fell back to
writing prose into `.work/handoffs/`, so the loss looked like agents choosing not to record
rather than being unable to.

**Why:** three things compounded. `.work` in a worktree is a symlink to the main repo, so
the write target is outside the worktree boundary. `sandbox/settings.rs` grants
`Read(.work/memory/**)` but no matching `Edit` — only `.work/handoffs/**` gets a write
grant. And the loom binary is **not** exempt from the sandbox: `validate_emittable` rejects
`excluded_commands` outright. So `record.rs`'s direct `append_entry` hit the kernel and lost.

**Misleading signal:** the comment in `sandbox/settings.rs` asserted that "memory and dispute
state are daemon-owned, so direct file-tool writes must never be authorized" — describing an
RPC that **does not exist**. `daemon/protocol.rs` has `CompleteStage` and `DisputeCriteria`
and nothing for memory. The comment read as deliberate design, so the missing write grant
looked intentional rather than like a gap. A second fossil pointed the same wrong way:
`fs/permissions/constants.rs` declared `LOOM_PERMISSIONS_WORKTREE` with `Write(.work/**)` and
`Bash(loom *)`, which read like a blanket grant but has no consumers outside its own unit test
— and `Write(path)` rules are inert anyway. (That entry is now `Edit(.work/handoffs/**)`; see
[../concerns/sandbox-write-rules-inert.md](../concerns/sandbox-write-rules-inert.md).)

**Prevention:** when a comment says state is written "through the daemon", verify the RPC
exists in `daemon/protocol.rs` before treating a missing write grant as intentional. An
architecture note describing an unbuilt mechanism is indistinguishable from one describing a
real one. More generally: after removing a sandbox escape (here, the 2026-08-08
`excluded_commands` rejection), audit every operation that depended on it — `loom stage
complete` was given a broker, `loom memory` was not, and nothing failed loudly enough to
notice.

**Fix:** spool + drain. The sandboxed CLI appends to `<worktree>/.loom/memory-spool.jsonl`
(inside the worktree, no new grant needed) and the daemon drains it. Attribution is by
worktree location, not by any claim in the payload. See
[architecture/memory-spool.md](../architecture/memory-spool.md).

**Found alongside:** `validate_stage_id` rejects path separators but not a sibling stage's
id, so `loom memory note --stage <other>` could write another stage's journal — and journals
are injected into other stages' prompts (`orchestrator/signals/generate.rs`). Now refused
whenever `LOOM_STAGE_ID` is set and disagrees.

## An Agent-Facing `touch /tmp/<marker>` Handshake Can Never Fire (2026-08-25)

**What happened:** `loom pressure` auto-closes each foreground Claude session by injecting,
via `--append-system-prompt`, "your FINAL action MUST be to run exactly this shell command
… `touch /tmp/loom-pressure-claude-<pid>.done`", then polling for that file and SIGTERMing
the idle session once it appears. The marker never appeared — not once, in any round, since
the command shipped. The operator had to create the file by hand TWICE per round (after
`/pressure` and again after `/address`), and the failure looked like an agent ignoring its
instruction rather than a hard impossibility.

**Why:** the driver spawns `claude --permission-mode auto`, so the agent's Bash runs inside
the OS sandbox, which mounts `/tmp` **read-only** (`touch: cannot touch '/tmp/x':
Read-only file system`). But `std::env::temp_dir()` was evaluated in the DRIVER, which is
unsandboxed and could create, poll and delete `/tmp` paths perfectly well. Every side of the
mechanism except the agent's own write worked, so it read as sound.

**Prevention:** for any handshake file, ask **who creates it** separately from **who reads
it**. A path an AGENT must write at runtime has to live inside the repo working tree — the
sandbox's writable root is the child's cwd — regardless of where the parent process could
write. `codex_log_path()` in the same module correctly stays in the temp dir because the
DRIVER opens it. Before shipping such a handshake, run its literal command from inside a
sandboxed session; the whole bug is visible in one `touch`.

**Fix:** the marker moved to `<repo>/.work/pressure/claude-<pid>.done` (gitignored, and
loom's own hook guard covers only `.work/stages/` and `.work/sessions/`), the driver
`create_dir_all`s the parent before each spawn, and the injected instruction now names this
as the sanctioned exception to the "never write under `.work/` directly" rule so a
rule-abiding agent does not balk at it. Regression test: `claude_marker_path` must NOT
start with `std::env::temp_dir()` (`commands/pressure/tests.rs`) — mutation-verified by
reverting the path and watching only that test go red.

**Found alongside:** the fix added 31 lines to a file sitting exactly at its
`maintainability-baseline.txt` cap (596), so `cargo test --test maintainability` rejected a
5-line behavioural change. Budget for this: touching an at-cap file means refactoring it in
the same change — `pressure/mod.rs` was split into `mod.rs` / `paths.rs` / `spawn.rs`
(229/145/278 lines) and its ledger entry deleted.

## A Sandboxed Caller Cannot Reach the Daemon Socket, and `loom status` Called It Missing (2026-08-27)

**What happened:** a stage agent reported that the completion bridge had "no transport", because
`loom status` said `daemon process alive, socket missing / try loom repair`. The socket was on
disk and the daemon healthy. Reproduced directly:

```text
$ ls -la .work/orchestrator.sock
srw------- dkaponis dkaponis 0 B  .work/orchestrator.sock        # present

$ python3 -c "import socket; socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)"
PermissionError: [Errno 1] Operation not permitted                # denied at socket() creation
```

**Why:** Claude Code's Bash sandbox denies AF_UNIX socket syscalls outright — the denial lands at
`socket()`, before `connect()` is even attempted. `DaemonServer::check_status` inferred "socket
missing" from any `UnixStream::connect` failure, so every stage agent that ran `loom status`
against a healthy daemon got the alarming form. The hint then pointed at `loom repair`, whose
advice for socket trouble is to kill and restart the daemon.

**Prevention:**

- **A failed connect is not evidence about the server.** It conflates three states: no socket, a
  stale socket with no listener, and a caller that is not permitted to dial. Classify by
  `io::ErrorKind` (`PermissionDenied` → unreachable-from-here) before naming a cause in a message
  an operator or agent will act on.
- **Do not corroborate a sandbox denial with a second syscall.** A sandbox that denies `connect`
  may or may not permit `stat`, so a failed `exists()` proves nothing and must never downgrade the
  classification.
- **`is_running()` must stay true for the unreachable state.** The flock already proved a daemon
  owns the `.work/`; reporting otherwise would let a second daemon start. See
  `concerns/daemon-singleton.md` for the incident that makes this load-bearing.

**Also worth knowing — ~20 project-root paths are read-denied in a stage sandbox.** Extracted from
real transcripts, these produce `Permission denied` for any recursive read from the worktree root:

```text
./.claude/{hooks,skills,agents,commands,workflows,routines,output-styles,launch.json,scheduled_tasks.json,loop.md}
./.gitconfig ./.gitmodules ./.mcp.json ./.ripgreprc ./.vscode
./.bashrc ./.zshrc ./.profile ./.bash_profile ./.zprofile
```

Any tool loom shells out to that walks the tree will hit all of them, once per invocation. See
`mistakes/schema-reuse-and-silent-skips.md` § "An Exit Code That Also Means 'Some Files Were
Unreadable'" for what that cost.

## `loom handoff` Wrote the Document and Silently Failed the Stage Transition (2026-08-31)

**What happened:** a stage's main agent hit its context ceiling, ran the mandated `loom handoff
--stage climate-timezone-data --session session-a183358d-1788132648 --trigger ceiling`, and
stopped. The daemon did nothing for hours until an operator noticed. The stage file still read
`status: executing`. The command's own output, verbatim:

```text
Warning: could not mark stage 'climate-timezone-data' NeedsHandoff: Failed to open temp file: /home/dkaponis/src/cartolyth/.worktrees/climate-timezone-data/.work/stages/01-climate-timezone-data.md.tmp
/home/dkaponis/src/cartolyth/.worktrees/climate-timezone-data/.work/handoffs/climate-timezone-data-handoff-002.md
```

**Why:** a worktree agent's Bash sandbox write allow-list grants `.work/handoffs` but not
`.work/stages` — deliberately, since `daemon/server/control_complete.rs` exists so a sandboxed
agent cannot mutate trusted `.work` state directly. `loom handoff` wrote the document and could
not apply the `Executing -> NeedsHandoff` transition. `commands/handoff/create.rs` treated that
failure as a warning on a command that exited 0. The daemon's recovery is level-triggered on
`NeedsHandoff`, so it never armed. Two independent recovery paths existed and neither fired: the
second, `MonitorEvent::SessionHung`, was advisory and only printed.

**Prevention:** any command a sandboxed worktree agent runs to mutate trusted `.work` state
outside `.work/handoffs` must either route through the daemon broker or fail loudly — a warning
on exit 0 reads to the agent as success and it stops. When a recovery path is level-triggered on
a state field, ask what writes that field and whether the writer can fail under the sandbox. A
recovery path that only prints is not a recovery path.

**Fix:** `loom handoff` now returns an error naming the cause and the document path. The monitor
watches `.work/handoffs` for a document naming the current stage and session with
`origin: agent_ceiling` and drives the existing takedown-and-requeue from it, which is the one
signal a sandboxed session can always leave. `SessionHung` now recovers the stage, bounded at two
stall recoveries.

## A `Read(...)` Deny Reshaped to Dodge One Check Froze the TUI and Missed the Other (2026-09-04)

**What happened:** auto mode kept prompting on `rg`/`grep` from the project root because the
daemon token `Read(...)` denies sat inside the project. The 2026-09-03 fix globbed the project
directory out (`Read(//home/you/src/*/.loom/work/admin.token)`) so the rule's wildcard-free prefix
left the project. The prompts continued on every `cd X && rg pattern relative/dir`, and Claude Code
began freezing for long stretches: the operator could not type.

**Why:** two separate defects in one reading of the binary. The prompt has a second branch that
fires on the mere existence of any `Read(` deny in any settings source whenever the compound
command contains a `cd` and the path is relative, so no shape avoids it. And on Linux every
`Read(...)` deny is fed to the OS sandbox, whose glob expander does a synchronous recursive
`readdirSync` of the wildcard-free prefix, here all of `~/src` (2.7 million inodes), per Bash
command. The 09-03 analysis knew about the expansion and only ruled out `**` at `/` or `~`, not a
`*` one level below the home directory.

**Prevention:** when a harness check is bypass-immune, read the WHOLE predicate before choosing a
rule shape, and search for every branch that returns the same `circuitBreaker`. Before emitting any
glob into a sandbox list, name the directory the expander will walk and reject it if that directory
is not small and owned by the project. A fix for an operator prompt is verified only by running the
exact command that prompted, with a `cd` in front of it, not by re-reading rule text.

**Fix:** no `Read(` deny is ever written; `denyRead` plus `hooks/credential-guard.sh` keep the
boundary. See concerns.md § "No `Read(...)` Deny Rule May Exist in Any Settings File".

## The Sandbox's AF_UNIX Denial Also Kills sccache, Breaking Every Cargo Command (2026-09-04)

**What happened:** loom exports `RUSTC_WRAPPER=/usr/bin/sccache` into every stage session
and into the confined acceptance environment (`process/environment.rs` allowlists
`RUSTC_WRAPPER`) whenever sccache is found on the host. Every `cargo build`/`clippy`/`doc`/
`test` then fails before a single crate compiles: `error: process didn't exit successfully:
/usr/bin/sccache rustc -vV` / `sccache: error: Operation not permitted (os error 1)`.

**Why:** the same AF_UNIX `socket()` denial documented above — sccache's client reaches its
server over a Unix domain socket, and `sccache --start-server` cannot even bind one.
`find_sccache_path()` (`orchestrator/terminal/native/build_cache.rs`) only proves the binary
EXISTS, never that it can run where it is being exported to. NOT a cache-directory
permission issue: a writable `SCCACHE_DIR` fails identically. Note `sccache --version`
SUCCEEDS, so any probe weaker than starting the server passes and proves nothing.

**Detection:** a cargo failure whose FIRST line names sccache, before any "Compiling" line,
is this — not a build defect.

**Workaround inside a session (no daemon change):** prefix the command with
`env -u RUSTC_WRAPPER`. This also fixes `git commit`/`git push`, since `loom/.githooks/
pre-commit` and `pre-push` run cargo without that prefix and git hooks inherit the git
process's environment — `env -u RUSTC_WRAPPER git commit ...` is enough.

**Fix that needs an operator, not the stage session:** restart `loom run` with
`LOOM_SCCACHE=0` so the daemon stops exporting the wrapper at all — the acceptance criteria
themselves carry no prefix and the completion-guard hook pins the stage-completion command
to one exact invocation, so nothing inside the session can add a prefix there. Never amend a
shared plan's criteria to carry the prefix; that bakes a machine-specific workaround into the
plan.

## `cargo audit` and `cargo deny` Fail in a Stage Sandbox for Two Unrelated Reasons (2026-09-04)

**What happened:** `cargo audit -f Cargo.lock -d target/advisory-db` and `cargo deny check`
both fail verbatim in a stage session, and neither failure is the sccache/AF_UNIX one above.

**`cargo audit`:** the operator's global `~/.gitconfig` carries `url.ssh://git@github.com/
.insteadof https://github.com/`, so `cargo audit`'s https clone of `RustSec/advisory-db` is
silently rewritten to `ssh://`, and the sandbox has no ssh key or `known_hosts`:
`ssh_askpass: exec(/usr/bin/ssh-askpass): No such file or directory` / `Host key
verification failed`. Detection: an https git fetch failing with `ssh_askpass` or "Host key
verification failed" is an `insteadOf` rewrite, never a proxy/allowlist denial —
`git ls-remote https://github.com/...` succeeds once the rewrite is off. Workaround:
`GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null <command>` (`GIT_CONFIG_COUNT`/
`GIT_CONFIG_KEY_0` do NOT work — `insteadOf` picks the longest matching prefix and the
existing rule still wins).

**`cargo deny`:** not installed, and `cargo install cargo-deny --locked` fails with
`Read-only file system (os error 30)` — the sandbox allows `~/.cargo/registry` and
`~/.cargo/git` but not `~/.cargo/bin`. Install with `--root $TMPDIR/tools` (and
`CARGO_TARGET_DIR` under `$TMPDIR`), prepend to `PATH`. Once running, `cargo-deny` shells out
to `cargo metadata`, which inherits `RUSTC_WRAPPER` and dies on the same sccache EPERM —
`cargo-deny` reports that as "failed to fetch crates", which reads like a network denial and
is not one; prefix with `env -u RUSTC_WRAPPER` too.

**Prevention:** when N acceptance criteria fail together, attribute each one INDIVIDUALLY —
a shared symptom ("all cargo commands fail") is not a shared cause, and inheriting a prior
session's attribution without re-testing propagates a wrong diagnosis.

## A Bash Tool Call Is Its Own Network Namespace — a Server Started in One Call Is Unreachable From Another (2026-09-04)

**What happened:** a server started with `cargo run status --web` (or `bun run dev`) in one
Bash call is unreachable from a later Bash call, and from a Playwright MCP browser, because
each Bash invocation gets its own network namespace.

**Prevention:** for any visual/browser check of a locally-served app, start the server AND
make every request against it inside ONE shell invocation
(`(loom status --web PORT & sleep 2; curl ...; kill $!)`), or serve pre-captured content to
the browser directly — e.g. build to `$TMPDIR/dist` and have the Playwright process itself
serve fixtures via `page.route`/`page.routeWebSocket` rather than proxying to a live server
in a different Bash call. `browser_run_code_unsafe`-style sandboxes have no
`process`/`require`/`import`, only `page`.

## A Sandboxed `git merge` That Aborts Still Leaves the Branch's New Files Untracked (2026-09-10)

**What happened:** a merge-resolver session ran `git merge loom/memory-events` in the main
checkout. Git failed with `unable to unlink old '.gitignore': Device or resource busy` (and the
same for `CLAUDE.md.template`, `commands/distill.md`, `skills/loom-plan-writer/SKILL.md`, plus
`Read-only file system` for `hooks/pre-compact.sh`) and printed `Merge with strategy ort
failed.` HEAD, the index and every tracked file were unchanged, but the ten files the branch
ADDS had already been written as untracked files. The later `git merge --ff-only` aborted with
`untracked working tree files would be overwritten by merge`.

**Why:** the Bash sandbox bind-mounts each individually allow-listed path (`.gitignore`,
`CLAUDE.md.template`, the hook and skill files), so git cannot unlink and recreate them. Ort
checks out new paths before it reaches the busy ones and does not remove them on abort. The
resolver's clean-tree check filtered `??` lines, so the strays went unseen.

**Prevention:** after any failed merge or checkout in the main repo, compare
`git diff --name-only --diff-filter=A <base> <branch>` against the working tree, not just
tracked status. When the branch touches a bind-mounted path, merge in a detached temporary
worktree (`git worktree add --detach <scratch> main`), commit there, and have the user
fast-forward main from outside the sandbox. In the pre-commit hook and cargo, export
`RUSTC_WRAPPER=` (see the sccache entry above).

**Fix:** confirm each stray is byte-identical to the merge commit
(`git hash-object <f>` equals `git rev-parse <commit>:<f>`), remove it, then fast-forward.

## `loom review` wrote through the `.work` symlink into the main repo's doc/plans (2026-07-22)

**What happened:** From inside a worktree, `loom review` printed `✓ Review document written to doc/plans/REVIEW-....md` (exit 0) but the file never appeared in the worktree's `doc/plans/` — it had been written to the MAIN repo's copy, invisible from the worktree.
**Why:** The command resolved its output root via `WorkDir::main_project_root()`, which follows the worktree's `.work` symlink back to the main repo. The success message then printed the path relative to that root, making it look local.
**Prevention:** Commands that WRITE user-visible files must anchor on the current checkout (worktree root when `cwd` is inside `.worktrees/`), not on `main_project_root()` — that helper is for reaching shared `.work` state, not for output placement. Exit 0 + "written to <relative path>" is not proof the file is where the reader thinks; check which root the path was relativized against.
**Fix:** `commands/review/generate.rs::resolve_output_root()` — writes to `find_worktree_root_from_cwd(cwd)` when inside a worktree, else the main project root.

## Worktree Test Runs Resolve node_modules From the MAIN Repo When the Worktree Has None (2026-08-11)

**What happened:** in a JS-project worktree stage (cartolyth `city-detail-popup`), codex's proof
command `bunx vitest run …` failed with EROFS writing `node_modules/.vite-temp`, with no test
assertions executed — while the file edits themselves landed fine.

**Why:** a fresh git worktree has no `node_modules` (ignored files are not part of the checkout).
Node module resolution walks UP from the worktree — `.worktrees/<stage>/` → `.worktrees/` → the
MAIN repo's `node_modules/` — so test runners load dependencies from the main checkout and write
their caches there too (vite writes `node_modules/.vite-temp/` while loading config). Both the
codex nested sandbox and the stage sandbox refuse that write, correctly: it lands outside the
worktree, in shared mutable state that parallel stages and the operator's checkout depend on.
Proof it really happens: `node_modules/.vite-temp` exists in cartolyth's MAIN repo, created by a
later unsandboxed run.

**Prevention:** a plan whose stages run JS/TS tests in-session MUST provision dependencies inside
the worktree before the first test run — an explicit first task (`bun install` from the worktree
root) in the stage description. `setup:` does NOT cover this: it only prefixes acceptance
commands, which `loom check` runs on the host after the session's work, not inside the session.

**Fix:** never widen a sandbox toward the main repo's `node_modules` — the denial is the system
working. Install dependencies in the worktree, then re-run the tests.

## A Ledger Written From Inside the Bash Sandbox Silently Never Filled

**What happened:** the MODELS column of `loom status --live` never showed a codex tier on any
stage — every row read `opus›sonnet,opus` — while Claude subagents always appeared. The codex lane
was installed and licensed on the stages that ran.

**Why:** `.loom/work/subagents/<stage-id>/codex.jsonl` was appended by `hooks/codex-forward.sh`,
which the forwarder subagent invokes through the Bash tool — so it ran INSIDE the stage's Bash
sandbox. From a worktree that append resolves through the `.loom/work` symlink into the main repo,
outside the sandbox's write allow-list (handoffs and `doc/loom/knowledge`, `sandbox/settings.rs:295-311`).
`record_codex_task` was best-effort and returned 0 on every failure path, so the denial was logged
nowhere. Its sibling `spawns.jsonl` filled normally because `hooks/spawn-guard.sh` is a PreToolUse
hook, and hook processes are not under that sandbox. The display could not fall back either: the
forwarder's own `spawns.jsonl` row carries the shim's sonnet tier and is skipped on purpose, so a
codex run left NO trace at all.

**Prevention:** decide where a recorder RUNS before deciding what it writes. Anything the agent
invokes through Bash is inside the stage sandbox and reaches only the worktree, handoffs and the
knowledge dir; a hook process is not. A best-effort writer that returns 0 on every failure will
never report its own breakage, so its write path has to be checked when it is written, not when it
is missed.

**Fix:** moved the row into `hooks/codex-forward-guard.sh`, the PreToolUse hook that already parses
and validates the forwarding command, so only an authorized forward records. Widening the sandbox
was rejected: a ledger of what the agent did must not be writable by the agent.

## A Comment Described an RPC That Was Never Built

`loom memory note` failed with EROFS in every sandboxed stage: `.work` is a symlink out of
the worktree and the sandbox grants `Read(.work/memory/**)` with no matching `Edit`. It read
as intentional because the comment beside the grant said memory is "written through daemon
RPCs" — and `daemon/protocol.rs` has no such RPC. `loom stage complete` got a broker when
the `excluded_commands` escape was removed; `loom memory` did not, and nothing failed loudly.
Verify an RPC exists before treating a missing write grant as deliberate, and after removing
a sandbox escape, audit every operation that relied on it.

## A Stage Sandbox Can Deny Loopback TCP Even While the Server Reports Listening — but Not Always (2026-09-12)

**What happened:** in the settings-lanes stage, `loom status --web 7373` and
`vite --port 5173` both reported listening, but `curl --noproxy '*'` to `127.0.0.1`
returned HTTP 000 from the same Bash command, and headless Chrome rendered an empty
`<html>`. The same day, in the integration-verify stage for the same plan,
`scripts/smoke-web-dashboard.sh` against `loom/target/debug/loom` bound `127.0.0.1` and
every `curl` in it succeeded.

**Why:** loopback TCP denial is a property of that stage's sandbox network policy, not
a fixed platform behaviour — two stages in the same plan, run the same day, saw
opposite results.

**Prevention:** probe loopback with the smoke script (or a plain `curl`) at session
start, before planning work around its absence. Do not treat one stage's denial as a
standing rule for the next stage, or one stage's success as proof a sibling stage can
reach loopback too.

**Workaround when loopback IS denied and a plan step needs the dev server (e.g. a
visual review):** either add a sandbox network allowance for `127.0.0.1`, or render the
built bundle via `file://` with `fetch`/`WebSocket` stubbed — see
[patterns.md](../patterns.md#offline-file-harness-for-a-visual-review-under-a-no-network-sandbox).
`google-chrome`/headless-shell also needs `XDG_CONFIG_HOME`/`--user-data-dir` pointed at
the scratchpad (its crashpad handler writes to `~/.config` and dumps core otherwise),
and the Read tool's worktree guard only opens images inside the worktree — a
screenshot directory under `node_modules/` gets ignored by tools that read images, so
write screenshots inside the worktree proper.
