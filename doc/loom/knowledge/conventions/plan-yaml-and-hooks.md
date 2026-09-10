---
---
# Plan Yaml And Hooks

> Plan YAML schema, hook stdin/stdout contract, skill format, additive fields

## Plan YAML Schema

Required fields per stage: `id`, `name`, `working_dir` ("." or subdir), `dependencies` (list), `acceptance` (list)

Optional: `description`, `parallel_group`, `setup`, `files`, `auto_merge`, `stage_type` ("standard"|"knowledge"|"integration-verify"|"knowledge-distill")

Only `version: 1` supported.

## Hook Conventions

- Location: `~/.claude/hooks/loom/`
- Naming: `<event>-<action>.sh` (e.g., `session-start.sh`, `post-tool-use.sh`)

## Skill File Format

Directory: `skills/<skill-name>/SKILL.md`

Frontmatter fields: `name` (kebab-case, required), `description` (required; one inline scalar, never a `|`/`>` block, and the whole `description:` line at most 160 bytes because core-skill descriptions sit resident in every request; enforced over every `skills/loom-*/SKILL.md` by `no_skill_description_exceeds_the_resident_cost_cap` in `loom/src/skills/index_catalog.rs`; keywords go in `triggers`, description prose is not a trigger source), `triggers` (YAML array, highest priority), `trigger-keywords` (CSV string, fallback), `allowed-tools` (optional CSV).

Trigger priority: (1) triggers YAML array, (2) trigger-keywords CSV, (3) "TRIGGERS:"/"Trigger keywords:" in description text. Matching: phrase=2pts, word=1pt, threshold 2.0, max 5 per signal.

Body sections: Overview, When to Use, Instructions.

## Knowledge Files

Seven files: architecture, entry-points, patterns, conventions, mistakes, stack (aliases: deps, tech), concerns (aliases: debt, issues)

## Signal File Format

Signal files at .work/signals/{session-id}.md use markdown with structured sections. Knowledge/merge/recovery signals have distinct formats. All share .work/signals/ directory.

## Permission Mode YAML Values

`permission_mode` YAML values are kebab-case: `"auto"`, `"accept-edits"`, `"plan"`, `"default"`

## Plan YAML Schema: Acceptance Field

The `acceptance` field in stage definitions uses `Vec<AcceptanceCriterion>` (not `Vec<String>`).
Two forms in YAML:

- Simple: `- "cargo test"` (plain string)
- Extended: `- command: "loom --help"\n  stdout_contains: ["Usage:"]` (object with TruthCheck fields)

`has_any_goal_checks()` checks ONLY: artifacts, wiring, wiring_tests, dead_code_check.
Validation requires: acceptance OR goal-backward checks for standard/integration-verify stages.

Old `truths`/`truth_checks` fields were removed from `StageDefinition` and are now rejected by strict deserialization. `before_stage`/`after_stage` remain supported and still use `TruthCheck`.

Plan deserialization is strict at every policy-bearing layer: the metadata root, `LoomConfig`, `StageDefinition`, and nested sandbox, filesystem, network, Linux, adjudication, code-review, truth-check, wiring-test, and dead-code structures use `deny_unknown_fields`. A typo or retired field must fail parsing with an actionable unknown-field error; it must never disappear before validation. In particular, top-level `truths` is rejected.

**`loom knowledge sync` (and anything that reaches `context::retrieve::resolve_roots` → `ContextStore::open` → a `refresh` write) can never sit in a worktree stage's acceptance list.** `ContextStore::open` (`context/store.rs:49`) resolves the context cache under `WorkDir::main_project_root().join(".loom/cache/context-v1")` — deliberately OUT of the worktree, through the `.work` symlink, to the MAIN repository, so parallel stages share one cache instead of each growing an immediately-stale private copy. `sync`'s `refresh` step (`context/refresh.rs:218`) then WRITES there via `ContextStore::save_catalog`, and both settings emitters strip `.loom` from `allow_write`, so that write always trips the sandbox from inside a worktree. `loom knowledge context` (retrieval) also opens the same store but is safe: its refresh failure downgrades to a warning and it builds the catalog in memory instead (`context/retrieve.rs:147-149`), which is why the signal footer tells agents to run it directly. `loom knowledge check` was written specifically to be safe as an acceptance criterion by NEVER opening the context store at all — it resolves only the knowledge root and calls the pure, read-only `catalog::build` (`commands/knowledge/check.rs:1-21`); do not "simplify" it back into `context::resolve()`.

## Hook Output Contract

Claude Code hooks communicate with the host process via stdin/stdout and exit codes.

**Exit codes:**

- `exit 0` — allow the operation to proceed (default, no output needed)
- `exit 2` — block the operation; stderr is shown to Claude as a `PreToolUse:` prefixed message
- Any other exit code — treated as an error (non-blocking, but logged)

**hookSpecificOutput (JSON response for warnings):**
To issue a warning without blocking (exit 0 with advisory), write a JSON object to stdout with a `hookSpecificOutput` field. Claude Code appends this to the tool result as additional context. Example:

```json
{ "hookSpecificOutput": "LOOM_HOOK_WARN: consider using rg instead of grep" }
```

The `LOOM_HOOK_WARN:` prefix is recognized by the loom hook system and surfaced as a warning in output.

**PostToolUse stdin schema:**

```json
{
  "tool_name": "Bash",
  "tool_input": {"command": "...", ...},
  "tool_result": {"output": "...", "is_error": false, "exit_code": 0},
  "session_id": "...",
  "session_info": {...}
}
```

Some fields may use `tool_response` instead of `tool_result` depending on Claude Code version — always use `(.tool_result.x // .tool_response.x)` patterns in shell hooks.

**PreToolUse stdin schema:** `tool_name` and `tool_input` fields only (no result yet).

**Stop hook (session end):** receives `{"reason": "...", "exit_code": N}`. Used by `commit-guard.sh` and `learning-validator.sh`.

## Additive Schema Fields: Prefer `#[serde(default)]` Over Bespoke Migration (2026-08-07)

For a new additive stage field, `#[serde(default)]` carries existing plan files and in-flight
`.work/stages/*.md` without a bespoke upgrade pass. This compatibility rule does not make removed or
misspelled plan fields permissive: strict plan structs still reject unknown fields. The shape used
for `implementers` and `subagent_timeout_secs`:

- Plan schema `StageDefinition` (`plan/schema/types.rs`) — `#[serde(default)]`; add
  `skip_serializing_if = "Option::is_none"` for `Option` fields so re-serialized plans stay clean.
- Runtime `Stage` (`models/stage/types.rs`) — `#[serde(default)]`, so a stage file written
  before the field existed still loads mid-run.
- Prefer a **closed enum over a string**: `Implementer` (`models/stage/types.rs:135`) derives
  `Default` + `#[serde(rename_all = "kebab-case")]`, so a typo is a parse ERROR (`unknown variant
'bogus-lane', expected 'claude' or 'codex'`), never a silent fallback. Pin `Display` to the serde
  spelling with a test (`plan/schema/tests/implementer_tests.rs`).
- Propagate along the existing chain — see [Adding New Plan Fields Checklist](../architecture.md):
  plan → canonical `Stage::from_definition` (`models/stage/methods.rs`) → signal
  `EmbeddedContext` (`orchestrator/signals/generate.rs`).

**Model a per-stage capability as a SET, not a scalar, when more than one value can be true at
once.** `implementer` shipped as a single enum, which silently asserted that every subagent in a
stage came from one lane; real stages mix codex and Claude subagents. The fix was `Implementers`, a
`#[serde(transparent)]` newtype over `Vec<Implementer>` with `Default = [Claude]`, where MEMBERSHIP
licenses a lane and ORDER picks the default for routine work. Before adding an enum-valued stage
field, ask whether a stage could legitimately want two of its values simultaneously — if yes, ship
the list on day one, and gate any safety doctrine on `contains`, never on equality with the
preferred value. Validation must then reject the two shapes a list admits and a scalar could not:
the empty list and a repeated element.

**Guard the default with the `implementer_defaults` pair** — copy these two tests verbatim in shape
for any new field (`loom/tests/integration/implementer_defaults.rs`):

1. `*_plan_yaml_without_field_*` — parse plan markdown whose YAML has NO such key; assert every
   stage gets the default.
2. `*_stage_file_without_field_*` — write `.work/stages/*.md` frontmatter with no such key, call
   `load_stage()`, assert it loads and defaults.

Schema-only tests are not enough: they never touch the state files already on disk, which is exactly
where a non-defaulted field breaks a running plan.
