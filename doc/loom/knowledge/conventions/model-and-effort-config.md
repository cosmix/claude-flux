# Model And Effort Config

> `[pressure]` and `[models]` config sections, the four-tier precedence chain, defaults, and the KEY-level vs SECTION-level fallback split.

## Two Configurable Sections

`[pressure]` (`~/.loom/config.toml` / `.loom/work/config.toml`) sets the model and effort for
the three `loom pressure` steps. `[models]` sets the model and effort each stage type's
main-agent session launches with. Both are opt-in: `loom init` never writes either section into
a project config.

## Precedence Chain (per key, model and effort resolve independently)

1. Explicit per-invocation value — a plan stage's `model` / `reasoning_effort` field for stage
   sessions, a CLI flag (`--claude-model`, `--claude-effort`, etc.) for `loom pressure`.
2. Project config `<repo>/.loom/work/config.toml`.
3. User config `~/.loom/config.toml`.
4. Built-in default.

## `[pressure]` Keys and Defaults

`pressure.claude_model` opus, `pressure.claude_effort` xhigh, `pressure.codex_model`
gpt-5.6-sol, `pressure.codex_effort` xhigh, `pressure.address_model` opus,
`pressure.address_effort` high.

## `[models]` Keys and Defaults

`models.standard_model` opus / `models.standard_effort` high (`standard`);
`models.knowledge_model` opus / `models.knowledge_effort` medium (`knowledge`);
`models.knowledge_distill_model` sonnet / `models.knowledge_distill_effort` high
(`knowledge-distill`); `models.integration_verify_model` opus /
`models.integration_verify_effort` xhigh (`integration-verify`). Merge and base-conflict
sessions stay pinned at opus/high; adjudication keeps its own `[adjudication] model`; neither
is configurable through `[models]`.

## KEY-Level vs SECTION-Level Fallback

`[pressure]` and `[models]` resolve **per key**: a project section present but missing a key
still lets that one key fall through to the user config. `[terminal]` and `[context]` keep the
older **SECTION-level** behavior: a present section wins whole, with no partial fallthrough even
for keys it omits. The split exists because `[pressure]`/`[models]` are registries of
independent single-value knobs an operator plausibly wants to override one at a time, while
`[terminal]`/`[context]` are small, tightly-coupled sections — `loom init` writes
`context.ceiling_tokens` alongside `context.subagent_ceiling_tokens` as a pair meant to move
together — so a project override there replaces the whole tuned section atomically.

## Resolution Code

Stage sessions resolve through `crate::fs::work_dir::resolve_stage_model_effort`; the three
`loom pressure` slots resolve through `commands::pressure`'s own resolver; the user-tier lookup
for both goes through `crate::user_config`.

## Plan Doctrine

A plan stage omits `model` and `reasoning_effort` by default, so the stage type's configured
default applies. Set either field only as a deliberate override, and state why in the stage
description.
