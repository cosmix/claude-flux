# D1 — Prose authority by path, typed source evidence, verified claims, `loom knowledge annotate`, honest index limits

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `delivery-and-catalog` · Single wave (parallel with C1, C2, C3, D2).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F7 ingestion, F8, F16 diagnostics, F11 limits)

- **F7.** `fs/knowledge/catalog/prose.rs::push_prose_file` excludes only `DONE-*` under a `plans` segment. Proposals, reports, `REVIEW-*` documents, archived plans (`doc/plans/ archive/`, note the leading space), not-started `PLAN-*` files and every task brief under `doc/plans/briefs/` are indexed as `LifecycleState::Active` (`ProseSources::chunks` forces it), so a superseded design competes with the current one on rare words.
- **F16.** `chunker.rs::references_in` turns every backticked path-like span into a source reference regardless of its role; `loom knowledge check` reports 111 `MissingSourceRef` issues, most of them prose explaining that a file does NOT exist, placeholders (`category/slug.md`, `foo.md`), generated runtime paths (`.work/config.toml`), or files in another project. Those are not stale claims.
- **F8.** No claim carries a verification point. A `sources:` list exists in frontmatter but only feeds the same missing-ref check; nothing says "verified at revision X" or flags "the code this rests on changed since".
- **F11.** `MAX_INDEX_BYTES = 12_288` is already exceeded at 77 files (12,611 bytes) and the hierarchy repair stage adds topics; `MAX_BLURB_CHARS = 100`.

Merged before you: `retrieval-contracts` (`LifecycleState::Historical`, `LifecyclePolicy`), `source-graph-snapshot` (`index.rs::write_index` skips identical bytes — keep that). Codex's `loom map` will not show them; read `context/schema.rs` and `fs/knowledge/index.rs` directly.

## Files you own (write)

- `loom/src/fs/knowledge/catalog/prose.rs`, `catalog/tests_prose.rs`
- `loom/src/fs/knowledge/chunker.rs` (split `chunker/references.rs` out; each file under 400 lines), `fs/knowledge/tests/chunker.rs`
- `loom/src/fs/knowledge/catalog.rs`, `catalog/size.rs`, `catalog/source_roots.rs`, `fs/knowledge/tests/catalog.rs`, `tests/catalog/source_refs.rs`
- `loom/src/fs/knowledge/index.rs` — `MAX_BLURB_CHARS` only
- `loom/src/fs/knowledge/frontmatter.rs` — NEW (read/write the YAML block; used by chunker and annotate); `fs/knowledge/mod.rs` (declaration)
- `loom/src/commands/knowledge/check.rs`, `tests_check.rs`, `commands/knowledge/annotate.rs` — NEW, `tests_annotate.rs` — NEW, `commands/knowledge/mod.rs` (declare `annotate` AND `telemetry` — C2 creates `telemetry.rs` in parallel; declare it now so the crate links)
- `loom/src/cli/types_memory.rs` (`KnowledgeCommands` gains `Annotate` and `Telemetry`), `loom/src/cli/dispatch.rs` (both arms)
- `loom/src/context/ingest.rs` only if it matches `CatalogIssue` exhaustively (grep first)

Read-only: `commands/knowledge/{context,eval,telemetry}.rs` (C2/D2), `context/**`, `commands/hook/**`.

## Contract

### Prose lifecycle by path (`prose.rs`)

```rust
/// None → not indexed at all.
pub(crate) fn prose_lifecycle(relative: &Path, explicit: Option<LifecycleState>) -> Option<LifecycleState>
```

Rules, first match wins, on the PROJECT-relative path (components compared trimmed and lowercased): any component equal to `archive` → `None`; `DONE-*` under a `plans` component → `None` (today's rule); explicit frontmatter `state:` → `Some(explicit)`; under `plans`: `REVIEW-*` → `Historical`, `PLAN-*` or `IN_PROGRESS-PLAN-*` → `Draft`, a `briefs` component → `Draft`; a file name starting with `REPORT-` or `PROPOSAL-` (case-insensitive) anywhere → `Historical`; everything else → `Active`. `ProseSources::chunks` uses it (no more forced `Active`) and `files()` skips `None` paths so the fingerprinter agrees with the chunker (one function feeds both — keep it that way). The explicit state comes from `frontmatter::file_state(bytes) -> Option<LifecycleState>`.

### Typed evidence (`chunker/references.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvidenceKind { Live, Example, Runtime, External, Historical }
pub(crate) fn classify_reference(span: &str, sentence: &str) -> EvidenceKind
```

- `Runtime`: path starts with `.loom/`, `.work/`, `target/`, `node_modules/`, `~`, `/tmp`, `$`, or contains `<`/`>`.
- `Example`: span or sentence contains any of `foo`, `bar`, `baz`, `<`, `...`, `…`, `path/to`, `slug`, `newcmd`, `example`, `e.g.`, `placeholder`, `NN-`, `xxx`.
- `Historical`: the sentence (the text between the previous and next `.`/newline around the span) contains `does not exist`, `do not exist`, `no longer`, `never existed`, `invented`, `earlier version`, `was deleted`, `was removed`, `removed in`, `deleted in`, `renamed to`, `used to`.
- `External`: the sentence contains `external`, `upstream`, `another project`, `plugin's`, `openai`, `codex-rs`, `claude code's`, or the path's first component is not a directory or file at the project root AND not a declared cargo source root (reuse `source_roots.rs`'s resolution: unresolved AND the sentence names a foreign project).
- otherwise `Live`.

`KnowledgeChunk::source_paths` receives ONLY `Live` references (so `MissingSourceRef` fires only for them). The other kinds are counted per file into a new `CatalogIssue::UnverifiableReference { file, source_path, kind: String }` ONLY when the path does not resolve — reported by `check` as an informational `note:` line, never counted by `--strict`.

### Verified claims

`frontmatter.rs` parses and rewrites the leading `---` block: keys `id`, `aliases`, `state`, `sources`, `verified` (a git revision), unknown keys preserved verbatim. `catalog::build` computes, per curated file with both `sources` and `verified`, `CatalogIssue::EvidenceChanged { file, source_path, verified }` for every source path that `git diff --name-only <verified>..HEAD -- <path>` lists (one git invocation per file; a `git` failure yields no issue and a `tracing::debug!`). `check` prints it as `review: <file>: <path> changed since <verified8> — re-verify or`loom knowledge annotate <target> --verified HEAD`` and `--strict` does NOT count it (it is a review trigger, not a falsehood). Add `CatalogIssue::is_review_only(&self) -> bool` (`EvidenceChanged`, `UnverifiableReference` → true) and make `--strict` fail on `issues.iter().filter(|i| !i.is_review_only()).count() > 0`. `check --json` gains `"review": [...]` alongside `"issues"`.

### `loom knowledge annotate`

```text
loom knowledge annotate <target> [--state <active|draft|deprecated|superseded|historical>] [--source <path>]... [--clear-sources] [--verified <rev|HEAD>] [--alias <name>]... [--blurb <text>]
```

`<target>` through `KnowledgeTarget::parse` (tier-1 name/alias or `<category>/<slug>`); rewrites only the frontmatter block via `frontmatter::update_file(path, |fm| ...)` under the same parent-directory lock `append_target` uses; `--verified HEAD` resolves to the full revision with `git rev-parse HEAD`; prints the resulting frontmatter. `--blurb <text>` rewrites (or inserts, directly after the `# Title` line) the single `>` blurb line `index.rs::extract_title_and_blurb` reads — the only way to fix a `GenericBlurb` issue from the CLI; the text is one line, at most `MAX_BLURB_CHARS`, rejected otherwise. `INDEX.md` is refreshed afterwards the way `replace_section` does it (so a new blurb lands in the index immediately).

### Limits

`size.rs::MAX_INDEX_BYTES = 16_384` with the doc comment updated ("~4k tokens; the first read of every session — the ceiling the hierarchy repair keeps under"); `index.rs::MAX_BLURB_CHARS = 80`.

### `Telemetry` variant (for C2)

`KnowledgeCommands::Telemetry { #[arg(long)] stage: Option<String>, #[arg(long)] json: bool }` → `knowledge::telemetry::telemetry(stage, json)`; help: "Summarise recorded context delivery, prompt briefs, abstentions and pulls per stage".

## Steps

1. `frontmatter.rs` (parse/serialize with `serde_yaml` as `chunker.rs` does; keep unknown keys); `chunker.rs` uses it; `references.rs` classification; `prose.rs` lifecycle.
2. `catalog.rs`: `EvidenceChanged`, `UnverifiableReference`, `is_review_only`; `size.rs`/`index.rs` constants.
3. `check.rs`: `review:`/`note:` lines, strict semantics, JSON `review`; `annotate.rs` + CLI wiring for BOTH new variants.
4. Tests (names are what acceptance filters on):
   - `tests_prose.rs`: `a_review_document_is_historical`, `a_report_or_proposal_is_historical`, `an_archived_plan_is_not_indexed`, `a_not_started_plan_and_its_briefs_are_drafts`, `explicit_frontmatter_state_overrides_the_path_rule`, plus the existing cases updated where they asserted forced `Active`.
   - `tests/chunker.rs`: one test per `EvidenceKind` using the report's real samples (`gc.rs` in "There is no `gc.rs`", `category/slug.md`, `.work/config.toml`, `codex-rs/linux-sandbox/src/bwrap.rs`, `doc/plans/PLAN-foo.md`, and a genuine `loom/src/context/pack.rs`); `only_live_references_become_source_paths`.
   - `tests/catalog/source_refs.rs`: `an_example_path_is_reported_as_a_note_not_a_missing_source`; `tests/catalog.rs`: `evidence_changed_fires_only_for_a_source_that_changed_since_verified` (temp git repo: commit, annotate `verified`, commit a change to one source, build → one issue) and `strict_ignores_review_only_issues` (test the counting helper).
   - `tests_annotate.rs`: `annotate_writes_frontmatter_without_touching_the_body`, `annotate_verified_head_resolves_the_full_revision`, `annotate_rejects_an_unknown_state`.

## Done means

`cargo build --manifest-path loom/Cargo.toml` warning-free (with C2's `telemetry.rs` present — if it is not yet, add a one-line stub is NOT allowed; report instead); `cargo test --lib fs::knowledge::` and `--lib commands::knowledge::` green; `loom/target/debug/loom knowledge --help` lists `annotate` and `telemetry`; `loom/target/debug/loom knowledge check --json` on this repo shows `UnverifiableReference` notes where it showed most `MissingSourceRef`s (the orchestrator runs this; expect the strict-counted total to drop well below 127 — report the number you observe, do not tune to a target).

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib fs::knowledge::tests::chunker` — skip if unsure.

## Constraints and traps

- `check` must NEVER open the `ContextStore` (its module doc and `check_never_creates_a_loom_cache_directory` pin this); `git diff` and `git rev-parse` are fine.
- Curated chunks keep knowledge-root-relative `file`; prose chunks keep project-relative `file` (documented asymmetry in `prose.rs`).
- Do not add fields to `KnowledgeChunk` (its literal sites span five modules); the evidence kinds are consumed inside `fs/knowledge` only.
- The sentence window for classification must not cross a fenced code block; reuse `chunker::fence_marker`.
- `annotate` is the ONLY writer of frontmatter; `update`/`replace-section` never touch it.
- No git commands of your own (the ones your CODE runs are the feature).

## Report back

Files changed and created; the final `CatalogIssue` enum verbatim; the strict-counted issue total on this repo after your change; anything unresolved.
