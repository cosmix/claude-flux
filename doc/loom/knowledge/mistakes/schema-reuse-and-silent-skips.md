# Schema Reuse And Silent Skips

> deny_unknown_fields breaking a type with two deserialization sources, warn-and-continue masking total failure, and an overloaded exit code.

## `deny_unknown_fields` on a Type With TWO Deserialization Sources Broke the Second One (2026-08-17)

**What happened:** every `.work/stages/*.md` file failed to parse. `loom run` printed one
`Warning: Could not parse ... Failed to parse StageDefinition from frontmatter` per stage
and carried on. `StageDefinition` is `#[serde(deny_unknown_fields)]` — added in
`2d5a4679` to reject typo'd keys in PLAN YAML, which is a good guarantee for that caller.
But the same type was also being deserialized from a second, entirely different source:
stage files, whose frontmatter is a full serialized `Stage` carrying runtime-only keys
(`status`, `merged`, `fix_attempts`, `resolved_base`, `session`, `worktree`, …). Strictness
that is correct for the plan is fatal for the stage file, and nobody noticed the type had
two readers.

**The stale comment was the tell.** `fs/stage_loading.rs`'s module doc still asserted the
premise the attribute had invalidated — "serde ignores the runtime-only keys (`status`,
`created_at`, `merged`, …)". A doc comment stating a serde behaviour is load-bearing
documentation: when an attribute changes that behaviour, the comment becomes a false claim
that reads as reassurance.

**Prevention:** before adding `deny_unknown_fields` (or any strictness attribute) to a
type, enumerate every `from_*`/`parse` call site that deserializes INTO it. If there is
more than one source and they carry different key sets, the attribute belongs on a
per-source type, not the shared one. The fix here parses the frontmatter as the type the
file actually holds (`Stage`) and projects down via `definition_from_stage`, so
`StageDefinition` keeps its plan-YAML guarantee untouched.

**Keep the projection compiler-enforced.** `definition_from_stage` is a struct literal with
NO `..Default::default()`, so adding a field to `StageDefinition` fails the build until the
projection is updated. This is deliberate: an earlier hand-rolled partial struct silently
dropped `stage_type`, `auto_merge`, `sandbox`, `context_budget` and
`before_stage`/`after_stage` on every daemon restart. Never reintroduce a spread or a
hand-maintained key allowlist here — both fail silently, which is the whole bug.

## Warn-and-Continue Turns Total Failure Into an Empty Result

`load_stages_from_work_dir` handled a parse error by `eprintln!`-ing a warning and
`continue`-ing to the next file. With every file failing, it returned `Ok(vec![])` — a
successful empty load. Its production caller is the documented recovery path in
`plan/graph/loader.rs` ("Stage files in .work/stages/ can be used instead of the plan
file"), so that fallback could never have worked; it would have produced an empty graph
rather than an error.

**Prevention:** a per-item skip is only safe when *some* items are expected to succeed. When
a loop skips EVERY item, that is a total failure wearing a success type. Either fail loudly
when the skip count equals the item count, or make the caller distinguish "no stage files"
from "no stage file parsed". A warning printed once per file is not a substitute — it
scrolls past in daemon output and nothing downstream can act on it.

**Test fixtures encode assumptions too.** Five pre-existing tests hand-wrote minimal,
plan-shaped frontmatter and asserted it parsed — the exact assumption the bug was made of.
They passed for as long as the bug existed. When a fix makes old fixtures fail, check
whether the fixture or the code was wrong before "repairing" the fixture; here the fixtures
were, and were rebuilt to serialize a real `Stage`.

## An Exit Code That Also Means "Some Files Were Unreadable" (2026-08-27)

**What happened:** duplicate detection and wiring detection had been silently reporting
"nothing found" under every sandbox, while passing. Both shell out to `grep -r … .` from the
worktree root and treated exit code 2 as fatal — dump stderr, discard stdout, return empty.

**Why:** grep exits 2 when ANY file under the search root is unreadable, **even when the search
succeeded and stdout holds real matches**. Verified on the host:

```text
grep -r -n "needle" .    →  good.txt:1:needle here   exit=2   stderr: Permission denied
grep -s -r -n "needle" . →  good.txt:1:needle here   exit=2   stderr: (empty)
```

Note the second line: `-s` suppresses the messages but **the exit code is still 2**. Silencing
the noise alone would have hidden the flood and left the false PASS in place.

Sandboxed stages have ~20 read-denied paths at the project root
(`mistakes/sandbox-and-settings.md`), so exit 2 was the normal case, not the exception. The
same handler also `eprintln!`-ed the whole stderr blob: one integration-verify stage emitted
1344 lines / 59.8KB, which pushed the completion command's output past the harness persist
threshold and cost the stage its verification marker
(`mistakes/completion-broker-credential.md`).

**Prevention:**

- **Ask what a non-zero exit actually asserts.** An exit code that conflates "I failed" with "I
  partially succeeded" cannot be used as a control-flow signal. Read the tool's documented exit
  semantics before branching on them; here the stdout was always authoritative.
- **This is the sibling of "warn-and-continue" above.** There a per-item skip turned total
  failure into an empty success; here an exit code did. Both present as a thinner result, never
  as an error, and both passed their tests for as long as the bug existed.
- **A subprocess's stderr is unbounded input.** Never `eprintln!` it verbatim into output that
  something downstream parses or truncates. Cap it by lines and bytes.
- **Test the degraded environment, not just the happy one.** The regression tests build a tree
  containing a `0o000` file and assert the match is still found — and skip loudly (rather than
  passing vacuously) where permission bits are not enforced, e.g. under root.

## Schema Root: LoomConfig vs Plan

**Mistake:** Passing the top-level YAML document (which wraps `loom:` key) where a `LoomConfig` (the inner object) is expected, or vice versa. This commonly manifests as "missing field" serde errors.

**Why:** Plan YAML has the structure `{ loom: LoomConfig }`. `parse_plan()` extracts the `loom:` block and deserializes that into `LoomMetadata` / `LoomConfig`, not the outer wrapper.

**Prevention:** The canonical deserialization root is `LoomConfig` (at `plan/schema/types.rs`), not the outer document. Nested fields (execution, stages, sandbox) live on `LoomConfig`.

## A Scalar Config Field Silently Asserts "Exactly One" (2026-08-07)

**What happened:** the codex lane shipped as `implementer: claude | codex`, a single enum per stage.
That scalar quietly encoded a claim nobody had checked: that every subagent a stage spawns comes
from ONE lane. Real stages do not work that way — an implementation stage routinely wants codex for
routine file work, sonnet for tests, opus for the architectural call, and fable on a second failure.
The doctrine text then hardened the wrong model, telling stages "Codex REPLACES sonnet/haiku."

**Why:** the field was named for the decision it was born from ("which implementer do we use here?")
rather than for the state it represents ("which lanes may this stage draw from?"). A scalar can only
answer the first. Worse, the safety doctrine was gated on `implementer == Codex`, so the mixed case
was not merely inexpressible — it was UNSAFE: a `claude` stage that spawned one codex subagent got
none of codex's blast-radius rules (`.work/` symlink escape, hooks not seeing codex's own shell), and
nothing in the system would say so.

**Prevention (detection rule):** before shipping an enum-valued stage field, ask whether a single
stage could legitimately want two of its values AT ONCE. If yes, it is a set, not a scalar — ship the
list on day one. Then check every gate that reads it: a capability's safety doctrine must be gated on
MEMBERSHIP (`includes_x()`), never on equality with the preferred value. Equality gating is the bug
that hides, because the common cases still look right.

**Fix:** `Implementers`, a `#[serde(transparent)]` newtype over `Vec<Implementer>` with
`Default = [Claude]`, where membership licenses a lane and ORDER names the routine-work preference
(`preferred()`, `includes_codex()`, `is_mixed()`). Signal doctrine gates on `includes_codex()` and
tells the orchestrator to choose per subagent. Validation rejects the two shapes only a list admits:
empty, and a repeated lane (which would make the preference ambiguous). See
[Codex Plugin](../architecture/codex-plugin.md) for the wiring.
