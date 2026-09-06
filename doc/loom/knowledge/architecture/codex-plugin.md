# Codex Plugin

> Codex plugin install and identity, the codex-rescue subagent, and the loom-codex-forwarder lane.

## Install and identity

- Marketplace `openai-codex` from GitHub `openai/codex-plugin-cc`; plugin name `codex`.
- Observed installed version **1.0.6** (`claude plugin list --json`, 2026-08-06), install path
  `~/.claude/plugins/cache/openai-codex/codex/<version>/`.
- Needs the `codex` CLI on PATH (observed `codex-cli 0.146.0`) and an authenticated `~/.codex/auth.json`.
  Codex initialises a sqlite state runtime under `~/.codex`; a sandbox that denies writes there kills
  every run with `failed to initialize state runtime` (exit 1) before any model call is made — and
  some wrapper paths still exit 0, so read stderr, not the exit code.

Non-interactive CLI:

```bash
claude plugin marketplace add openai/codex-plugin-cc    # add <source>
claude plugin install codex@openai-codex --scope user   # user|project|local, default user
claude plugin list [--json]
```

## The codex-rescue subagent

**Loom stages no longer spawn this agent directly** — see "The loom-codex-forwarder lane"
below. The plugin wrapper's contract is documented here because the forwarder wraps the same
companion runtime and the hook pins BOTH agent types.

Spawn via the Agent tool with `subagent_type: "codex:codex-rescue"`.

Its frontmatter (`agents/codex-rescue.md`) declares `model: sonnet`, `tools: Bash`. **That sonnet is the
THIN FORWARDING WRAPPER, not the implementing model** — the real work runs in Codex behind the companion
script. Do not read that `sonnet` as the quality tier of the result. **The `tools: Bash` line is
inert:** plugin agents' `tools:` field is ignored by design
(code.claude.com/docs/en/sub-agents#available-tools), so this wrapper actually runs with a full
toolset — the mechanism behind the rogue-wrapper incident.

Wrapper contract (all from `agents/codex-rescue.md`):

- Exactly ONE Bash call to `node "${CLAUDE_PLUGIN_ROOT}/scripts/codex-companion.mjs" task ...`, and it
  returns that stdout **verbatim**, with no commentary before or after.
- It forwards `--model` / `--effort` **ONLY when the request names them** ("Leave `--effort` unset unless
  the user explicitly requests a specific reasoning effort"; "Leave model unset by default").
  **Therefore model and effort MUST be written into the prompt text you hand the subagent.**
- Valid `--effort` values: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`
  (`VALID_REASONING_EFFORTS`, `scripts/codex-companion.mjs:71`).
- It adds `--write` by DEFAULT unless read-only / review-only behaviour is requested.
- `spark` maps to `--model gpt-5.3-codex-spark`; `--resume` adds `--resume-last`, `--fresh` does not.
- Left to itself it picks **background** for open-ended tasks — so say foreground explicitly.
  See [Codex Concurrency](codex-concurrency.md): background fan-out is forbidden by doctrine.

`codex` echoes its own configuration banner (workdir / model / provider / approval / **sandbox** /
reasoning effort / session id) to **stderr** on every healthy run. Loom's silent-failure detector
flags that `sandbox` substring as a possible block; it is benign. Judge a run by the echoed model
line and the actual reply, not by the presence of `sandbox` in stderr.

## The loom-codex-forwarder lane (2026-08-07)

Loom's doctrine now spawns `subagent_type: "loom-codex-forwarder"` for all codex implementation
work, never the plugin agent directly. The rogue-wrapper incident (see
[Codex Lane Rogue Wrapper](../mistakes/codex-lane-rogue-wrapper.md)) showed the plugin agent's
`tools: Bash` frontmatter is not enforced for in-process subagents: a wrapper holding a codex
prompt implemented all the edits itself on sonnet and nothing surfaced it. Three shipped layers:

1. **`agents/loom-codex-forwarder.md`** — loom-owned forwarding shim (sonnet, `tools: Bash`),
   installed by `install.sh` with the other `loom-*` agents. Unlike the plugin wrapper, its
   `tools:` allowlist IS hard-enforced — user-scope agents enforce the field, plugin agents
   ignore it by design (code.claude.com/docs/en/sub-agents#available-tools) — so Edit/Read are
   blocked at the harness level and the hook only has to police Bash-shaped escapes. Contract: resolve the companion by
   glob (`~/.claude/plugins/cache/openai-codex/codex/*/scripts/codex-companion.mjs`, highest
   version — `${CLAUDE_PLUGIN_ROOT}` is unset outside plugin agents), ONE Bash call, `--write`
   plus `--model`/`--effort` exactly as the prompt states, stdout verbatim followed by a
   `--- LOOM-CODEX-EVIDENCE ---` trailer (`exit:`, `mode:`, then either the newest
   `state/*/jobs/*.json` paths in companion mode or the `session:` rollout path in direct mode —
   see the macOS section below).
   On failure: report verbatim prefixed `LOOM-CODEX-FORWARD-ERROR`, never implement.
2. **`hooks/codex-forward-guard.sh`** — PreToolUse on Bash/Edit/Write/Read/Task/Agent. Primary
   gate: payload `agent_type` ∈ {`loom-codex-forwarder`, `codex:codex-rescue`} → only a Bash
   command containing `codex-companion.mjs` passes; everything else exits 2 with forwarding
   doctrine on stderr. Fallback gate: `transcript_path` under `*/subagents/agent-*.jsonl` whose
   opening bytes carry `LOOM-CODEX-FORWARD-ONLY`. Fail-open everywhere else. `loom_is_subagent`
   was unusable here — it is process-tree based and returns false for in-process subagents.
3. **Signal doctrine** (`format_codex_implementers_section`, also emitted on the recovery path):
   spawn the forwarder by type, put `CODEX_FORWARD_SENTINEL` (`loom/src/codex.rs`) as the codex
   prompt's FIRST line, and accept a codex report only with the evidence trailer — verify the
   newest listed job record for the worktree exists with `phase: done`, or, for `mode: direct`,
   that the listed session rollout exists and postdates the spawn. A report without the
   trailer, or edits with no matching record, is a failed delegation: revert and respawn, or
   keep only after a full review.

Cross-surface pins: `tests_doctrine.rs::codex_forward_sentinel_agrees_across_surfaces` ties the
sentinel constant to the hook script and the agent definition, and requires CLAUDE.md.template
and the plan-writer SKILL to name the forwarder; `tests_cache.rs` pins the generated section to
carry the sentinel, the trailer name, and the `loom-codex-forwarder` spawn line.

## The sandbox must grant codex its state dirs (2026-08-10)

Codex is a subprocess, so the Bash sandbox — not Claude Code's tool permissions — decides whether it
can run at all. Its write set is the working directory plus the session temp dir, and codex needs two
directories outside both:

| Path                                       | Written by            | Failure when denied                            |
| ------------------------------------------ | --------------------- | ---------------------------------------------- |
| `~/.codex`                                 | codex CLI (sqlite state runtime, sessions, logs) | `failed to initialize state runtime`, `Read-only file system (os error 30)` |
| `~/.claude/plugins/data/codex-openai-codex` | codex-companion (per-job state) | `ENOENT: no such file or directory, mkdir '.../state/<cwd>-<hash>/jobs'` |

Both are declared once as `CODEX_SANDBOX_WRITE_PATHS` (`loom/src/codex.rs`) and emitted as
`sandbox.filesystem.allowWrite` — additive, OS-enforced for child processes, and the mechanism the
settings schema names for exactly this. `CODEX_SANDBOX_DOMAINS` does the same for the hosts codex
reaches (no domain is pre-allowed by default, and an unlisted host raises a mid-run permission
decision). Emitted from two places, deliberately: `sandbox/settings/policy.rs` (every stage worktree
and `loom repair --fix`) and `fs/permissions/settings.rs::ensure_loom_hooks_local` (`loom init`,
which never runs the sandbox generator). Sessions outside a loom repo need the same block in
`~/.claude/settings.json` — loom does not own that file.

**This is Linux-specific in origin, not in fix.** macOS Seatbelt let these writes through, so the
lane looked healthy there; the native bubblewrap sandbox enforces the allowlist and broke it.

**Never route around it with `dangerouslyDisableSandbox`.** That retry re-enters the permission gate
and the auto-mode classifier denies it, which is how the lane went from "sandboxed" to "unusable" —
see [Codex Lane Rogue Wrapper](../mistakes/codex-lane-rogue-wrapper.md).

## Codex's own sandbox must exclude /tmp (2026-08-11)

`allowWrite` was necessary but not sufficient: codex (0.147+) wraps every exec in its OWN nested
bubblewrap sandbox. In `workspace-write` mode its default writable roots are the cwd, `/tmp`, and
`$TMPDIR`, and it masks `.git` under every writable root (`codex-rs/linux-sandbox/src/bwrap.rs`).
`/tmp/.git` does not exist, so bwrap must create that mountpoint — and the outer stage sandbox
keeps `/tmp` read-only (only `/tmp/claude` and `$TMPDIR` are writable), so every codex exec dies at
namespace setup with `bwrap: Can't mkdir /tmp/.git: Read-only file system` before the model runs a
single command. Reproduce and verify without a model call:

```bash
codex sandbox -c sandbox_mode="workspace-write" -- echo hi            # bwrap: Can't mkdir /tmp/.git
codex sandbox -c sandbox_mode="workspace-write" \
  -c sandbox_workspace_write.exclude_slash_tmp=true -- echo hi        # hi
```

The fix is `[sandbox_workspace_write] exclude_slash_tmp = true` in `~/.codex/config.toml` — the only
channel loom controls, because the companion spawns `codex app-server` itself and no `-c` override
can be threaded through a forward. `loom/src/codex.rs` owns detection
(`codex_config_excludes_slash_tmp`) and the comment-preserving edit
(`ensure_codex_config_excludes_slash_tmp`, `toml_edit`); `loom repair --fix` applies it and
`advisory_codex_lane_preflight` (`commands/run/checks.rs`) warns at `loom run` startup when the lane
is licensed but the key is missing. Both are Linux-gated: on macOS the question never arises,
because inside the stage sandbox codex's Seatbelt cannot be applied at all (next section) and
outside it the default roots are harmless.

Do NOT "fix" this by adding `/tmp` to the outer sandbox's `allowWrite` instead: bwrap's mountpoint
mkdir writes through to the host, and a stray `/tmp/.git` makes git discovery under any `/tmp`
directory find a phantom repository.

## macOS: the stage sandbox refuses a nested Seatbelt, so the wrapper runs codex exec directly (2026-09-02)

**Symptom.** Five `loom-codex-forwarder` spawns in a stage session each reached gpt-5.6-terra and the
companion exited 0, but zero files were written: every shell command codex ran, even `pwd`, died with
`sandbox-exec: sandbox_apply: Operation not permitted`. The 67b97114 state-root redirect (previous
section) had worked; this failure sits one layer below it.

**Cause.** Stage Bash calls already run inside Claude Code's own Seatbelt sandbox on macOS. Codex's
`workspace-write` and `read-only` modes wrap each command it runs in `sandbox-exec` too, and macOS
refuses a second profile on an already-sandboxed process. Codex still exits 0 when the model's turn
ends, whatever its tools did.

```bash
sandbox-exec -p '(version 1)(allow default)' /bin/pwd                # sandbox_apply: Operation not permitted, rc 71
codex sandbox -- /bin/pwd                                            # same error
codex sandbox -c sandbox_mode="danger-full-access" -- /bin/pwd       # prints the cwd
```

**Why no config knob helps.** The companion hardcodes
`sandbox: request.write ? "workspace-write" : "read-only"` (`codex-companion.mjs:491`) into
`thread/start`, overriding `~/.codex/config.toml`'s `sandbox_mode` with no flag or env override, and
`read-only` seatbelts too. `dangerouslyDisableSandbox` is refused by the auto-mode classifier; there is
no macOS equivalent of Linux's `exclude_slash_tmp` — the nesting itself is refused.

**Fix.** `hooks/codex-forward.sh` probes `sandbox-exec -p '(version 1)(allow default)' /usr/bin/true`
(PATH lookup, so tests can stub it) and, only when refused, bypasses the companion and runs `codex exec
--sandbox danger-full-access --skip-git-repo-check --model <model> -c
model_reasoning_effort="<effort>" -- "<preamble + task>" </dev/null`. The `</dev/null` is required;
see "Direct-lane runs" below. The outer stage sandbox — worktree plus granted write paths,
domain allowlist, credential read denies — remains the boundary, same as a sonnet subagent's Bash call.

The evidence trailer now always carries `exit:` and `mode:`. `mode: companion` lists the newest
`state/*/jobs/*.json` records, globbed from the state root the companion actually used (including the
redirected `~/.codex/plugin-data` — the earlier wrapper globbed the original root and printed `jobs:
none found` on redirected machines). `mode: direct` lists the `session:` rollout path
(`~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`) that `codex exec` writes.
`signals/format/codex.rs` accepts either.

| Platform | Where | Lane | Inner sandbox |
| --- | --- | --- | --- |
| Linux | inside a stage sandbox | companion | bubblewrap, nested; needs `exclude_slash_tmp` |
| macOS | inside a stage sandbox | direct `codex exec` | none |
| macOS | outside any sandbox | companion | codex's own Seatbelt applies |

**Given up, deliberately.** No companion job record, so `/codex:status`/`/codex:result` cannot see
direct runs. Codex's Seatbelt no longer masks `.git` on macOS, so the preamble's no-git rule and the
orchestrator's post-run `git status --short` are the remaining guards. Codex's inner network cut-off is
gone, but the stage sandbox's domain allowlist still applies.

See [Codex Lane Rogue Wrapper](../mistakes/codex-lane-rogue-wrapper.md) for the verification gap that
let this ship.

## Availability fallback: codex CLI/plugin not installed (2026-08-08)

Stage licensing (`implementers` listing `codex`) says a stage MAY use the lane; it says nothing
about whether the lane is actually reachable on this machine. `loom/src/codex.rs` adds a second,
independent gate for that: `codex_lane_status()` does the real check (codex CLI resolvable via
`find_codex_path()`, and the plugin's companion runtime present at
`~/.claude/plugins/cache/openai-codex/codex/*/scripts/codex-companion.mjs`), and a memoized
`codex_lane_available()` wraps it as a cheap per-spawn boolean — the same
capability/preflight/resolve shape as [Remote Control](remote-control.md)'s `preflight()` /
`resolve()`.

- **`commands/run/`** runs the check once at startup and prints an advisory warning when the lane
  is unavailable, mirroring `run_startup_preflight` for Remote Control: informative, never a hard
  failure — a missing codex install must not block a run that never intended to use codex on most
  of its stages.
- **Signal doctrine.** When the lane is unavailable, `format_codex_implementers_section` emits a
  fallback note in place of the normal codex doctrine block: terra-/luna-tier work routes to
  sonnet for the run, and the orchestrator must NOT spawn `loom-codex-forwarder` — spawning it
  against a missing companion runtime would just fail the one Bash call the forwarder is allowed
  to make.
- **Scope.** The check does not mutate `implementers` or stage state; it only changes what the
  signal tells the orchestrator to do at spawn time, the same way Remote Control's `resolve()`
  gates the `--remote-control` flag without touching `.work/config.toml`.

## Hooks (hooks/hooks.json)

| Event        | Script                       | Timeout |
| ------------ | ---------------------------- | ------- |
| SessionStart | `session-lifecycle-hook.mjs` | 5s      |
| SessionEnd   | `session-lifecycle-hook.mjs` | 5s      |
| Stop         | `stop-review-gate-hook.mjs`  | 900s    |

The Stop gate is **OPT-IN**: `main()` returns early unless `config.stopReviewGate` is set
(`stop-review-gate-hook.mjs:154-156`; `defaultState()` sets it `false`), and
`STOP_REVIEW_TIMEOUT_MS = 15 * 60 * 1000` (`:16`). When it does run and fails it emits
`{"decision":"block", ...}` (`:169`). loom also binds Stop via `commit-guard.sh` — **both run**.

## What loom shipped for this lane

The per-stage `implementers` field names which agent lanes a stage may spawn subagents from.
Three moving parts:

1. **The field.** `Implementer` enum (`models/stage/types.rs:135`), `claude` | `codex`,
   `#[serde(rename_all = "kebab-case")]`, closed — an unknown value is a parse error. It is held by
   `Implementers` (`models/stage/types.rs`), a `#[serde(transparent)]` newtype over
   `Vec<Implementer>` whose `Default` is `vec![Claude]`. Carried on `StageDefinition`
   (`plan/schema/types.rs`) and `Stage` (`models/stage/types.rs`), both `#[serde(default)]`; copied
   at `commands/init/plan_setup.rs` and onto the signal's `EmbeddedContext` at
   `orchestrator/signals/generate.rs`. Query it through `includes_codex()` / `includes_claude()` /
   `preferred()` / `is_mixed()` — never by comparing the whole list, which is what made the original
   scalar design wrong.
2. **ORDER IS THE PREFERENCE, MEMBERSHIP IS THE LICENSE.** These are two different questions and
   the code must not conflate them:
   - `preferred()` (first element) = the lane routine implementation reaches for.
   - `includes_codex()` = whether the codex safety doctrine must be emitted.

   Gating doctrine on the _preference_ rather than on _membership_ is a real bug: a stage listing
   `["claude", "codex"]` may still spawn a codex subagent, and would then run with none of the
   blast-radius rules. That is exactly the hole the original `implementer == Codex` equality check
   left, and `tests_cache.rs` now pins both the mixed-signal and mixed-recovery cases against it.

   Validation (`plan/schema/validation.rs`) REJECTS an empty list and a repeated lane (order would
   be ambiguous), and WARNS when codex is listed on a knowledge, knowledge-distill, or
   integration-verify stage — on membership, not preference, for the same reason.

3. **The gated signal block.** `format_codex_implementers_section(&Implementers)`
   (`orchestrator/signals/format/sections.rs`) emits `## Codex Implementers`, gated on
   `includes_codex()`. It names every licensed lane, and on a mixed stage tells the orchestrator to
   choose the lane PER SUBAGENT and to keep ONE file-ownership table across lanes (cross-lane
   collisions lose work exactly as same-lane ones do). It lives in the **semi-stable** section
   (regenerated per stage), NOT the stable prefix — the stable prefix only forward-references it
   (`cache.rs`). The recovery path emits it too (`recovery_format.rs`), which was a real gap fixed
   by `d1530e0c`; without it a recovered or retried codex stage loses its whole doctrine block.
   Model/effort are interpolated from `CODEX_IMPLEMENTER_MODEL_TERRA = "gpt-5.6-terra"`,
   `CODEX_IMPLEMENTER_MODEL_LUNA = "gpt-5.6-luna"`, and `CODEX_IMPLEMENTER_EFFORT = "xhigh"`
   (`loom/src/codex.rs`) rather than hardcoded, and `tests_doctrine.rs` asserts BLOCK-B contains
   all three — so changing `codex.rs` without updating the prose surfaces fails the build. Terra is
   the tier for common implementation and integration tests; luna is for boilerplate, scaffolding,
   and simple unit tests.
4. **Settings carry-forward.** `PRESERVED_SETTINGS_KEYS` / `preserve_unowned_keys`
   (`sandbox/settings.rs:580,587`) — see the scope section below.

Do NOT confuse the two implementer-lane tiers — `gpt-5.6-terra` and `gpt-5.6-luna` (both
`codex.rs`) — with `gpt-5.6-sol` (`commands/pressure/mod.rs:245`, the `loom pressure` review
driver). Three models, three purposes; a grep for `gpt-5` returns all three.

## Install scope: user or project, not local

Scope decides the file: project → `.claude/settings.json`, local → `.claude/settings.local.json`,
user → `~/.claude/settings.json`. The relevant keys are `enabledPlugins` and `extraKnownMarketplaces`.

`sandbox::write_settings` (`sandbox/settings.rs:77`) REBUILDS `.claude/settings.local.json` from
scratch on every stage spawn, in worktrees and in the main repo root alike. **Since `df7d1060`
(2026-08-07) that no longer erases plugin installs:** `preserve_unowned_keys` (`:587`) carries a
two-key allowlist forward —

```rust
const PRESERVED_SETTINGS_KEYS: [&str; 2] = ["enabledPlugins", "extraKnownMarketplaces"];
```

— so a local-scope install now survives regeneration (verified by driving the real `write_settings`
over a seeded file twice). **Prefer user or project scope anyway**: the carve-out is exactly two
keys, so local scope is safe only for plugins and only by special case, while every other top-level
key in that file is still dropped. `/loom-plan-writer` states the user/project rule as the
requirement. See [Sandbox & Settings](../mistakes/sandbox-and-settings.md) for the rebuild mechanics.

## What Codex Actually Reads (verified 2026-08-29)

Codex loads `AGENTS.md` from its working directory and never reads `CLAUDE.md`. Probed both
directions with `codex exec -m gpt-5.6-luna` in a scratch repo: a passphrase planted in
`AGENTS.md` came back verbatim; the identical file renamed `CLAUDE.md` produced `UNKNOWN`.
`loom install-assets` now writes `~/.codex/AGENTS.md` from `AGENTS.md.template`
(`loom/src/assets/install.rs:110-127`), capped at 12,288 bytes (`loom/src/assets/mod.rs:22`) —
well under codex's own 32,768-byte `project_doc_max_bytes` truncation point.

Two consequences:

- Codex still starts every run with no doctrine beyond what a project's `~/.codex/AGENTS.md` and
  the prompt carry, since a scratch repo or one that never ran `loom install-assets` has neither.
  `hooks/codex-forward.sh` still prepends its own per-task stage contract on every forwarded task —
  the one channel an orchestrator writing a prompt cannot forget, and the standing `AGENTS.md` does
  not make it redundant.
- The old signal doctrine's claim that codex "inherits CLAUDE.md's knowledge-first rule" named the
  wrong mechanism. The rule reached codex because orchestrators pasted CLAUDE.md Rule 5's Claude
  preamble ("READ CLAUDE.md IMMEDIATELY AND FOLLOW ALL ITS RULES") into codex prompts. Codex obeyed,
  read the file, hit KNOWLEDGE-FIRST, and paged a ~200k-token corpus through shell reads. Never send
  that preamble to a codex prompt.

## The Navigation Kit

Codex's harness is shell-based, so the cure for slow reading is a shell command that returns
digested facts rather than bytes. Loom's source graph already ships four commands, all sub-second,
all usable from any directory in the tree:

| Command | Answers |
| --- | --- |
| `loom map --find-all <symbol>` | every definition of a name: path, line, kind |
| `loom map --outline <file>` | a file's symbols with line ranges and signatures |
| `loom map --impact <symbol\|path>` | what reaches it, with path confidence |
| `loom knowledge context --query "<q>" --budget-tokens <n>` | ranked knowledge plus matching source |

Verified: given only the first two, `gpt-5.6-luna` answered a two-part structural question - where a
constant is defined, and how many functions its file holds - in two commands and ~10k tokens, with
no file reads at all.

**Not read-only: they try to refresh a cache outside the worktree's sandbox.** Both commands never
write anything inside the worktree itself, but they DO try to refresh a derived-artifact cache under
the canonical MAIN repo's `.loom/cache/context-v1/` (`context/store.rs:44-57` resolves it via
`main_project_root`, following `.work`'s symlink out of the worktree on purpose so every parallel
stage shares one cache rather than growing an immediately-stale copy of its own). That path is not
in a stage worktree's `allowWrite` set (`sandbox/settings/policy.rs:137-158`), so inside a worktree
the refresh is denied, the command prints `warning: could not refresh the working-tree source graph
...; reading the layers already on disk` (`commands/map.rs:116`), and it answers from the last
successfully published layer instead — the BASE the stage branched from, not the current working
tree. Consequence: inside a worktree the navigation kit does not show edits made during the same
session, including a sibling unit's earlier changes in that stage. See
[Parallel Worktree Shared State](../mistakes/parallel-worktree-shared-state.md) for the general rule
this instance follows.

## Direct-lane runs: benign stderr, closed stdin, and what the report must be (2026-09-02)

A healthy `mode: direct` run logs these repeatedly on stderr while the model call succeeds and the
files land:

```text
codex_models_manager::manager: failed to refresh available models: Connection failed: error sending request
rmcp::transport::worker: worker quit with fatal: Transport channel closed ... https://chatgpt.com/backend-api/ps/mcp
```

Both are benign: codex 0.152's background model-list refresh, and a ChatGPT-hosted MCP endpoint. Neither
is a missing sandbox grant. `chatgpt.com`, `*.chatgpt.com`, `api.openai.com` and `auth.openai.com` are
already in `CODEX_SANDBOX_DOMAINS` (`loom/src/codex.rs:42-47`) and in the generated settings, so adding
hostnames or editing `~/.codex/config.toml` fixes nothing.

**Detection rule.** Judge a forwarded run by the evidence trailer and the files, never by stderr: `exit:
0` in the `--- LOOM-CODEX-EVIDENCE ---` trailer, a `session:` rollout newer than the spawn, and the
task's files present and correct. Two spawns on 2026-09-02 (gpt-5.6-luna and gpt-5.6-terra, effort
xhigh) printed the pair throughout and wrote both files byte-exact.

**Stdin.** The same runs printed `Reading additional input from stdin...`. `codex exec` treats an open
stdin as extra prompt input and blocks until EOF; under the Bash tool stdin is already at EOF, so only
a caller that keeps it open hangs. The wrapper passes `</dev/null` on the direct-lane invocation, and
`hooks/tests/codex-forward-wrapper.sh` stubs `codex` to exit 66 unless `/dev/stdin -ef /dev/null` while
running the wrapper with stdin on a regular file, so dropping the redirect turns that case red. The
companion lane needs none: it takes the prompt positionally and only falls back to stdin when that is
empty (`readTaskPrompt`, `codex-companion.mjs:643-649`), and gives its child pipes rather than this
process's stdin (`lib/app-server.mjs:193`).

**The report is the forwarder's final message.** Both spawns relayed the wrapper output through
`SendMessage` and ended their turn with a one-line summary, so the harvested report carried no trailer
at all. `codex-forward-guard.sh` is now registered for `SendMessage` too
(`loom/src/fs/permissions/hooks/config.rs`, pinned by `assert_codex_forward_guard_matchers`), and
`agents/loom-codex-forwarder.md` states that the final message is the report.
