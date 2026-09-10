# Sandbox And Confinement Gaps

> Sandbox gaps: no E2E canary, diverging env allowlists, uncalled validators

## Sandbox Denial Has No End-to-End CI Canary

The generated sandbox policy is covered by unit and flow tests, but nothing proves denial actually
holds against a live Claude runtime: CI has no callable credentialed Claude sandbox runtime, so
Bash, interpreter, build-script, symlink, and file-tool denial cannot be exercised end to end.
That verification is manual release validation.

(Residual of a resolved concern: the fail-open defect itself — generated settings not carrying
sensitive reads into `denyRead`, and `failIfUnavailable` unset — was fixed 2026-08-08.)

## ReDoS Potential in Plan Pattern Regex

User-provided regex patterns in plan files (failure_patterns, wiring patterns) are compiled and executed without complexity checks. While mitigated by trust model (plan authors = trusted), consider adding regex timeout or complexity limits for defense in depth.

Files: src/verify/baseline/capture.rs:76-79, src/verify/baseline/compare.rs:155-158

## Bootstrap Settings Backup Risk

`bootstrap.rs:write_bootstrap_sandbox()` keeps the settings.local.json backup in memory only (`Option<String>`). If the process is killed between writing sandbox settings and restoring the original, user settings are permanently lost. Low probability since bootstrap is interactive, but a disk-based temp backup would be more robust.

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

## Sandbox-Widening Fields Need No Author Acknowledgement (2026-08-17)

`plan/schema/validation.rs:45-69` `unsafe_plan_reasons` gates only `enabled: false` and
`allow_unsandboxed_escape`. So `allow_write`, `allow_all_unix_sockets`,
`allow_local_binding` and `linux.enable_weaker_nested` each widen the sandbox with no
acknowledgement required from the plan author. Reviewer-reported and not independently
confirmed — read the code before acting.

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
