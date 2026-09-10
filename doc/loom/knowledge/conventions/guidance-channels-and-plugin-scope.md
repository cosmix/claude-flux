---
---
# Guidance Channels And Plugin Scope

> Guidance-channel selection, verification-is-main-agent rule, plugin scope

## Vendored Agent Assets Live at Repo Root

Claude slash commands and Codex skills shipped by loom live in source at the repo root, NOT under `loom/`:

- `commands/*.md` — Claude slash commands (installed to `~/.claude/commands/`)
- `codex/skills/<name>/SKILL.md` — Codex skills (installed to `~/.codex/skills/<name>/`)

`install.sh` asserts the required source files exist before copying and fails the install if any are missing.

## Guidance Delivery Channels Convention

Agent guidance lives in the channel that delivers it closest to the decision point, cheapest:

- **Hooks** (`hooks/*.sh`) — rules that must never be violated (plans path, all-files staging, commit/complete, worktree isolation, subagent verification). Deterministic; the exit-2 message re-injects the rule at the exact moment of violation.
- **Stage signals** (`orchestrator/signals/`) — stage-execution mechanics (completion checklist, adversarial review dimensions). Delivered per-stage at execution time.
- **Skills** (`skills/*/SKILL.md`) — task-scoped expertise loaded on demand (`loom-plan-writer` owns ALL plan-authoring mechanics: YAML, working_dir, acceptance design, model selection, parallelization).
- **CLAUDE.md.template** — only cross-cutting rules and the 6-item hard-stop tier (stated verbatim at top AND bottom; middle of a long file is a retrieval dead zone). Do not restate what a hook, signal, or skill already delivers — duplicated guidance drifts and dilutes.

When adding new guidance, pick the channel first; the template is the channel of last resort.

**Hook versus prose — how to choose (2026-07-28).** Prose is advice an agent may reason its way
around; a hook is a wall. Escalate a rule to a hook when _all_ of these hold:

1. The violation is **cheap to detect mechanically** — a command shape, a path, a file state.
2. The violation is **expensive or irreversible** once it happens (lost work, corrupted state,
   a security relaxation granted wrongly).
3. Prose has **already failed**, or the rule contradicts a stronger instinct the agent has.

The plans-path rule is the worked example: it was the one hard rule with no hook enforcement and
it kept being violated, because "write the plan where you were told" loses to the harness's own
suggestion. Adding `plans-path-guard.sh` ended it.

Two obligations come with choosing the hook channel:

- **The prose does not go away — it must AGREE.** An enforcement layer landed without updating
  the guidance layer produces surfaces that actively instruct the blocked behaviour, and the
  agent obeys the instruction and hits a wall it was told to walk into. After adding a hook,
  sweep every prose surface for wording the hook now retires.
- **The refusal message is the guidance.** It is read at the exact moment of the mistake, so it
  must state the rule, the allowed alternative, and the carve-out — not just "blocked".

Corollary for exceptions: an exception must live in **every block that gets copied into a
subagent prompt**, not only in the prose that explains the rule.

## Verification Is the Main Agent's Job

Subagents do not verify. A subagent may run **at most one narrowly-scoped check** relevant to
the files it just changed; project-wide builds, full test suites, and repo-wide lint or
typecheck runs belong to the main agent, which is the only party that can see the whole tree.

Enforced by `hooks/subagent-verify-guard.sh` (PreToolUse:Bash), stated in the Rule 5 subagent
preamble in `CLAUDE.md.template`, and injected into stage signals by
`orchestrator/signals/cache.rs`. The three copies are pinned byte-for-byte by
`orchestrator/signals/tests_doctrine.rs`.

**The one exception:** an `integration-verify` stage exists to run the complete suite, so its
subagents are carved out. The carve-out is resolved from the stage file and **fails safe** — more
than one glob match, a non-integration-verify stage type, or a missing file all mean "no
relaxation".

## Claude Code Plugin Scope in Loom Repos (2026-08-07)

Enable Claude Code plugins at **user or project scope** — `claude plugin install codex@openai-codex --scope user`.
Scope decides the file: user → `~/.claude/settings.json`, project → `.claude/settings.json`,
local → `.claude/settings.local.json`. That last file is the one loom REBUILDS from scratch on every
stage spawn (`sandbox::write_settings`, `sandbox/settings.rs:77`, called from
`orchestrator/core/stage_executor.rs:373` and `:584`, and from `loom repair --fix`).

**Nuance that shipped 2026-08-07 — do not over-read the old "local scope vanishes" rule.**
`preserve_unowned_keys` (`sandbox/settings.rs:587`) now carries a two-key allowlist across every
regeneration:

```rust
const PRESERVED_SETTINGS_KEYS: [&str; 2] = ["enabledPlugins", "extraKnownMarketplaces"];
```

Plugin enablement at local scope therefore _survives_ — verified by driving the real `write_settings`
over a seeded file twice. The user/project rule still stands, for a different reason than before:
the carve-out is exactly two keys, so local scope is safe **only** for plugins and **only** by
special case. Everything else you put in that file (`env`, custom `hooks`, ...) is still dropped on
the next spawn — see [Sandbox & Settings](../mistakes/sandbox-and-settings.md).

Verify inside a worktree after a stage starts; never assume:

```bash
rg -n "enabledPlugins" .claude/settings.local.json
claude plugin list --json
```
