# Visibility And Reachability

> pub(crate) is not nameable by itself - visibility is capped by path reachability - plus sibling traps around wrapper types and field names.

## `pub(crate)` Does Not Make an Item Nameable — Visibility Is Capped by PATH Reachability

`orchestrator/signals/mod.rs` declared `mod cache;` and `mod format;` without `pub`, so
`cache::stable_prefix_for` and `format::brief::format_knowledge_brief` were **E0603-unreachable
outside `signals/`** even though each was marked `pub(crate)` at its definition.

**This bit TWO independent workers in one round, and each failed DIFFERENTLY and quietly:**

- one shipped an always-`None` resolver, so its feature was structurally inert while its
  acceptance greps still passed;
- the other wrote a SECOND copy of a renderer it could not reach, duplicating an
  untrusted-content fencing rule that must not drift between surfaces.

Neither reported a blocker. Both diffs looked reasonable in isolation. That is the shape to
watch for: an unreachable dependency does not stop a capable worker, it makes it invent a
local substitute.

**The one loud signal in the whole class:** an unused `pub(crate) use brief::format_knowledge_brief;`
at the definition site fires an unused-import warning, which under `-D warnings` is the only
place the problem surfaces automatically.

**Rules:**

1. On `E0603 module X is private`, add a re-export in the parent `mod.rs`
   (`pub(crate) use X::item;`) — **do not route around it.**
2. When a brief tells a worker to CALL something in another module, verify the module
   DECLARATION is `pub`/`pub(crate)`, not just the item. Check both halves before fanning out.
3. Treat "I wrote my own because I could not reach yours" in a worker report as a duplication
   defect to fix at integration (see `mistakes/subagent-orchestration.md`).

## Sweep for the Struct Itself, Not Just Its Wrapper

Adding a field to `StageSandboxConfig` was swept with `rg 'MergedSandboxConfig \{'` only, so the
exhaustive `sandbox: StageSandboxConfig {` literal at `models/stage/methods.rs:668` was missed and
broke the whole crate (exit 101) — which then surfaced as a bogus "test failure" in a sibling
subagent's unrelated run.

**Prevention:** when adding a field to struct `T`, sweep `rg -n "T \{" src tests` for **T itself**,
not just the wrapper struct you were thinking about. A nested literal in a test fixture is the usual
survivor. And note the second-order effect: in a shared worktree, a crate-wide break presents as an
unrelated sibling's test failure, so check whether the crate even compiles before believing a
sibling's red result.

## A Field Name Can Name the Wrong Domain Object

`RankQuery.stage_dependency_ids` was seeded from a dependency's STAGE ID. The field is documented
and matched as a CHUNK id (`<relative-path>#<heading>#<occurrence>`), so passing `"source-graph"`
never matched any candidate and the `StageDependency` boost never fired. Ranking still succeeded,
output still looked plausible, and no test failed — which is exactly why it survived review.

**Rule:** when a field name names one domain object but its doc comment and its matcher name
another, **believe the doc comment and the matcher**. Then prove the boost fires by asserting the
`SelectionReason` appears, never just that the call succeeded. (Correct source here: the dependency
stage's own delivery records under `.work/context/<plan>/<dep>/session-retrieval/`, whose
`delivered[].node_id` values ARE chunk ids.)

## An `Option` That Is `None` For Two Reasons Cannot Gate a Claim About Either

`signals`' `context_pack` was `None` both when retrieval selected nothing AND when retrieval
degraded (unreadable cache, `resolve_roots` error), yet `sections.rs` gated the
`CRITICAL: KNOWLEDGE BASE IS EMPTY` box on it — so a fully documented project was told its
knowledge base was empty.

**Detection rule:** when a boolean question is answered by `is_none()`/`is_empty()` on data fetched
for a DIFFERENT purpose, enumerate every way that data can be absent. **Fix:** resolve the second
question at its own source (`KnowledgeDir::has_content`) and carry it as its own field, defaulting
in the fail-safe direction (do not claim).

The same shape appears in telemetry, where an I/O error and a genuine miss collapse into one reason
string — see `concerns.md`.

## An Infallible Predicate Hides Every Failure As "No"

`git::branch::branch_exists` wraps `run_git_bool`, which swallows spawn failures and non-zero exits
into `false` (`git/runner.rs:112`). So any guard reading "branch absent" as "nothing to lose"
**proceeds on a timeout, a locked repo, or a missing git.** Use
`git::cleanup::branch_exists_strict`, which separates exit 1 from exit 128, whenever the answer is
load-bearing.

Generalisation: a predicate that cannot express "I do not know" must not gate a destructive
operation. Ambiguity has to resolve to the fail-safe branch, and a `bool` return cannot do that.

## Do Not Bypass a Constructor's Clamp — Widen It

A resolver worker wrote `edge.to = ...; edge.confidence = ...` directly, with a comment explaining
why it was bypassing `SourceEdge::inferred`'s confidence clamp. **The bypass was semantically
correct** — whole-graph uniqueness genuinely beats the single-file 0.5 ceiling — so the review
question "is this value right?" answers yes, and the real defect goes unasked: nothing now prevents
the next author writing `1.0`, or overwriting a `Parser` edge.

**Prevention:** when a subagent's comment explains why it is going around a constructor, treat that
as a **design escalation, not a justification**. The fix is a second constructor encoding the wider
bound (`SourceEdge::resolve_to`, clamped to 0.9, refusing `Parser` and already-resolved edges),
never a raw field write.

## Daemon Module Visibility

**Mistake:** Used `crate::daemon::server::DaemonServer` but `server` module is private.
**Fix:** Use re-export path: `crate::daemon::DaemonServer`.

## `pub(crate)` Is Invisible to `tests/` (2026-08-08)

`tests/e2e/*` is an **external test crate**, so it can only reach `pub` items. A stage description that
names a helper both `pub(crate)` _and_ "the e2e test seam" is self-contradictory. Before marking a
helper `pub(crate)`, check whether anything under `tests/` calls it — `src/` unit tests can reach it,
integration targets cannot.

Related: when changing a fn signature under `src/commands/`, the call-site inventory must include the
sibling `#[cfg(test)]` module. `src/commands/init/tests.rs` held a fifth `cleanup_orphaned_sessions()`
call site beyond the four an `rg` for the primary feature symbol surfaced. `rg` the **exact fn name**
across `src/` _and_ `tests/` before writing a subagent's step list.
