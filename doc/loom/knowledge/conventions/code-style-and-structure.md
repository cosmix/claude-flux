---
---
# Code Style And Structure

> Rust naming, error handling, size limits, splitting, and docstring conventions

## File & Branch Naming

| Type           | Pattern                                                    | Location          |
| -------------- | ----------------------------------------------------------- | ----------------- |
| Stage files    | `{depth:02}-{stage-id}.md` (depth 0 = `01-` prefix)        | `.work/stages/`   |
| Session files  | `{session-id}.md` (ID: `session-{uuid_short}-{timestamp}`) | `.work/sessions/` |
| Signal files   | `{session-id}.md`                                          | `.work/signals/`  |
| Handoff files  | `{stage-id}-handoff-{NNN:03d}.md`                          | `.work/handoffs/` |
| Plan files     | `PLAN-*` -> `IN_PROGRESS-PLAN-*` -> `DONE-PLAN-*`          | `doc/plans/`      |
| Stage branches | `loom/{stage-id}`                                          |                   |
| Base branches  | `loom/_base/{stage-id}` (multi-dep merges)                 |                   |

## Error Handling

- Application and orchestration code returns `anyhow::Result<T>` and adds actionable context when
  crossing a layer or performing an operation whose raw error does not identify the target.
- Use a typed error only when callers must distinguish domain outcomes by variant; do not erase that
  structure merely for visual uniformity.
- Preserve native adapter errors at their natural boundary, including `io::Result`, serde errors,
  `FromStr::Err`, and Clap validator strings. Convert them when application code consumes them.
- Git errors must include the command, directory, exit code, stdout, and stderr.
- Do not add a second general error framework without a concrete caller-facing API need.

## Serialization

- State files use markdown with YAML frontmatter (`---` delimited)
- Serde: `#[serde(rename_all = "snake_case")]` on structs
- Use `#[serde(default)]`, `#[serde(skip_serializing_if = "Option::is_none")]`, `#[serde(alias = "...")]` as needed
- All timestamps: `DateTime<Utc>` from chrono

## Module Organization & Re-exports

Standard module layout: `mod.rs` (exports), `types.rs`, `methods.rs`, `transitions.rs` (if state machine), `tests.rs`

Re-export rules: `pub use` explicit items (never wildcards). Only export public API. `pub use` NOT `pub mod`.

## Testing

- Filesystem tests: `tempfile::TempDir` for isolation
- `#[serial]` from `serial_test` for tests needing exclusive access
- Naming: `test_<action>_<condition>`
- Inline `#[cfg(test)] mod tests {}` for simple cases; separate `tests.rs` for complex suites
- Integration tests in `loom/tests/integration/`, shared helpers in `helpers.rs`

## ID and Input Validation

| Field               | Rules                                                               |
| ------------------- | --------------------------------------------------------------------- |
| Stage ID            | Max 128 chars, `[a-zA-Z0-9_-]`, no `/\.`, no reserved OS names      |
| Fact Key            | Max 64 chars, `[a-zA-Z0-9_-]`                                       |
| Acceptance criteria | Max 1024 chars, no control chars (except tab/newline/CR), non-empty |

## Constants

```rust
// Context thresholds (models/constants.rs)
DEFAULT_CONTEXT_CEILING_TOKENS: u32 = 150_000;
DEFAULT_SUBAGENT_CEILING_TOKENS: u32 = 120_000;
MIN_CONTEXT_CEILING_TOKENS: u32 = 60_000;
DAEMON_CEILING_MULTIPLIER: f32 = 1.25;

// Timeouts
DEFAULT_COMMAND_TIMEOUT = 300s;
DEFAULT_VERIFICATION_TIMEOUT = 30s;
HUNG_SESSION_TIMEOUT = 300s;
POLL_INTERVAL = 5s;

// Retries
DEFAULT_MAX_RETRIES: u32 = 3;
BACKOFF_BASE_SECONDS: u64 = 30;
BACKOFF_MAX_SECONDS: u64 = 300;
```

`DEFAULT_CONTEXT_LIMIT`, `CONTEXT_WARNING_THRESHOLD`, `CONTEXT_CRITICAL_THRESHOLD`, `DEFAULT_CONTEXT_BUDGET`, `CONTEXT_ABSOLUTE_MAX` and the `display::CONTEXT_*_PCT` module were all deleted with the move from a percentage context budget to an absolute token ceiling — do not reintroduce them.

## Display Conventions

Status icons: Completed=`✓` Executing=`●` Queued=`▶` WaitingForDeps=`○` Blocked=`✗` NeedsHandoff=`⟳` MergeConflict=`⚡` WaitingForInput=`?` Skipped=`⊘` CompletedWithFailures=`⚠` MergeBlocked=`⊗`

Colors (`colored` crate): Executing=blue.bold, Completed=green, Blocked=red.bold, Pending=dimmed, Queued=cyan, Warning=yellow

Context bar: renders absolute `tokens`/`ceiling` and colours off `context_health(tokens, ceiling)` (`commands/status/render/progress.rs:56-72`) — Green `<60%`, Yellow `60-90%`, Red `>=90%` of the resolved ceiling, not a fixed percentage of a 200k window.

## Enum Conventions

- Derive: `Debug, Clone, Serialize, Deserialize, PartialEq`
- Serde: `#[serde(rename_all = "kebab-case")]` for status enums
- Implement `Display` matching serde representation (e.g., `WaitingForDeps` -> `"waiting-for-deps"`)

## Builder Pattern

Used for complex struct construction: `fn builder() -> Self { Self::default() }` with `fn with_field(mut self, val) -> Self` chainable methods.

## Comment Style

- Module docs: `//!` at top of file
- Function docs: `///` with `# Arguments`, `# Returns` sections
- Inline comments: sparingly, only for non-obvious logic
- **Naming an item in doc prose: `` [`name`] `` ONLY if `name` is `pub` and resolves from that module; otherwise a plain code span `` `name` ``.** The brackets are an intra-doc link, and a link to a private `fn`, a `const`, a field, or a local fails `RUSTDOCFLAGS="-D warnings" cargo doc` — a CI job and a step in both git hooks, evaluated by no other local check. This has blocked pushes four times; since 2026-09-05 `pre-commit` runs the rustdoc gate whenever a staged `.rs` diff touches a `///` or `//!` line, so the failure now surfaces at commit time. See "A `[`link`]` to a Private Item Fails the Docs Build" in [Testing & Lint](../mistakes/testing-and-lint.md).

## Code Size Limits

File: 400 lines | Function: 50 lines | Struct impl: 300 lines | Exceed = refactor immediately

`cargo test --test maintainability` enforces the file and function limits in CI. Legacy production
exceptions are recorded in `loom/maintainability-baseline.txt` and may only shrink: a new exception
or an increase above the recorded size fails the gate.

## Dependency Management

Never hand-edit manifests. Use: `cargo add`, `bun add`, `uv add`, `go get`

## Import Deduplication

When a pattern appears 3+ times, extract to a canonical location:

- `parse_stage_from_markdown` -> `verify::transitions::serialization`
- `branch_name_for_stage` -> `git::branch::naming` (never inline `format!("loom/{}", id)`)

## Map Module Conventions

Detectors skip: .git, .work, .worktrees, node_modules, target, .venv, __pycache__. Deep=3-level depth + concerns, Normal=2-level. Source extensions: .rs, .ts, .js, .py, .go, .java, .rb.

## Dependency Pins for Native-Grammar Crates

Exact-pin (`=x.y.z`) any dependency whose generated output is cached, and collapse a family
of optional deps behind ONE feature rather than letting `cargo add` mint an implicit feature
per dep — otherwise a host can disable half a family and leave a registry inconsistent. See
`architecture/source-graph.md` for the worked example.

**After any `cargo add` that pulls a new crate, run `cargo fetch` ONCE with the sandbox
disabled.** The Bash sandbox makes `~/.cargo/registry/cache` read-only, so a later
`cargo build` dies with `failed to open .../<crate>.crate: Read-only file system (os error 30)`
— which reads like a corrupt registry rather than a permissions problem. Every build after
that fetch works inside the sandbox because the `.crate` files are present.

## Splitting a File

Use the edition-2021 layout `<name>.rs` plus a `<name>/` subdirectory (as
`context/rank.rs` + `context/rank/{corpus.rs,rungs.rs}` do). **Never `<name>/mod.rs`**
— it deletes the path that a stage's artifacts and wiring lists pin. Check the ledger
and the wiring patterns first; see `mistakes/pinned-literals-ledgers-and-wiring.md`.

(Correction 2026-08-21: this section previously cited `context/graph_store.rs` + `graph_store/` as the worked example, but `context/graph_store.rs` does not exist in the tree — that module has always been `context/graph_store/mod.rs`, the OTHER layout this section says to avoid. `context/rank.rs` is a verified, currently accurate example of the sibling-style split with no `mod.rs`; other equally valid ones include `context/lexical.rs` + `context/lexical/`, `context/refresh.rs` + `context/refresh/`, and `context/retrieve.rs` + `context/retrieve/`. The `<name>/mod.rs` layout is not wrong everywhere — `commands/hook/mod.rs` and `context/graph_store/mod.rs` are both legitimate _directory modules_ built that way from the start. The rule this section states applies specifically to _splitting an existing top-level `<name>.rs` file_: converting it to `<name>/mod.rs` mid-split changes the file's own path, which breaks anything pinning `<name>.rs` as a literal — acceptance criteria, artifacts lists, wiring checks.)

A file that must ADD a wired submodule without editing a read-only parent (the file that
would otherwise gain the new `mod` declaration is owned by another stage or subagent) can
route around that instead of waiting — for example, `#[path = "sibling_file.rs"] mod name;` inside the
file you DO own declares a flat sibling file as a child module, without any edit to the
directory's own `mod.rs`/parent declaration. `commands/hook/user_prompt.rs`'s
`#[path = "tests_user_prompt_e2e.rs"] mod e2e;` is the established precedent for this,
used for splitting a same-directory test file the same way.

Two visibility details that bite when splitting:

- Re-export across a module boundary explicitly: an item marked `pub(crate)` is unreachable
  if any module on its path is declared without `pub`.
- Moving a `#[cfg(test)]` fixture DEEPER breaks an existing `pub(super)` re-export (E0364).
  Give the moved item `pub(in crate::path::to::original::scope)` to match its original
  effective visibility exactly, rather than `pub(super)`.

## Docstring Honesty

State the current wiring, not the intended one. If a consumer is unbuilt, say so — the house
style to copy is `commands/context/record_edit.rs:12-14`, which states outright that it is
consumed by nothing and is pure input for a consumer that has not been built. Prefer intra-doc
links (`` [`crate::context::refresh`] ``) over plain backticks for module cross-references, so
a wrong path becomes a rustdoc warning instead of a permanent lie that survives
`clippy -D warnings`.

## Deliberately-Invalid Test Fixtures

`tests/maintainability/scanner.rs` parses EVERY `.rs` file under the crate, `tests/fixtures/`
included, and errors on unbalanced braces. An intentionally-unparseable fixture must therefore
**not carry a real `.rs` extension** — name it `<name>.rs.broken` and pass a virtual `.rs`
dispatch path to whatever must still treat it as that language.

## Bump `INDEX_VERSION` Whenever `lexical::tokenize` Changes

`context/lexical_index.rs::INDEX_VERSION` (currently `1`) has no compile-time
protection tying it to the tokenizer. The persisted index file already hashes
the `WEIGHT_*` scoring constants (`derivation()`, `lexical_index.rs:85-98`) and
is rejected on a mismatch, so retuning a weight cannot leave a warm cache
scoring at the old value. `lexical::tokenize` (`context/lexical.rs`) is the one
document input to that same index with **no** constant hashed into it: a
tokenizer change that keeps every source byte identical still changes what
each document's `(term, weight)` pairs are, and the index has no way to detect
that on its own.

The failure mode is a divergence visible only on a cache HIT: a warm index
built under the old tokenizer keeps serving old-tokenization postings, while a
cold miss rebuilds under the new tokenizer and scores differently — same code,
same corpus revision, two different answers depending on nothing but whether a
cache file happened to survive. See [Context Retrieval](../architecture/context-retrieval.md)
for the rest of the index's invalidation contract (why `average_length` and
the document-frequency map are recomputed rather than stored, and why weights
are persisted as IEEE-754 bits).

**Rule:** any change to `lexical::tokenize` (the split rules, the emitted
casing, what counts as a token boundary) MUST bump `INDEX_VERSION` in the same
commit. A file at the wrong version is treated as a miss, not an error — the
reader falls back to the scan and rewrites the file — so bumping the version
costs nothing but a few extra scans on the next prompt per revision, while
forgetting it costs a silent, hit-only scoring divergence that no test
currently pins.

## Version and Release Identity

The product version is the git tag, and nothing else. `loom/Cargo.toml`'s `version = "0.0.0-dev"`
is a deliberate placeholder, so **`env!("CARGO_PKG_VERSION")` evaluates to the literal string
`0.0.0-dev` in every build, released ones included** — never use it for anything a user or an
external service sees. Use `crate::version::VERSION` (or `LABEL` for a `v`-prefixed display
string), which `loom/build.rs` derives from `git describe --tags --exact-match` via
`derive_version` (`loom/src/version/derive.rs`) and emits as the `LOOM_VERSION` env var.

The chain that keeps it honest: a release is cut by pushing a `v*.*.*` tag;
`.github/workflows/release.yml` checks out with `fetch-depth: 0` so the tag is present at build
time, and its `verify-version` job runs `loom -v` on the built binary and fails the release unless
the reported version equals the tag minus its leading `v`. A build with no tags reachable degrades
to `0.0.0-dev+<sha>` rather than lying.
