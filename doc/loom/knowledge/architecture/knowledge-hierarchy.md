# Knowledge Hierarchy

> Tier-1/tier-2 knowledge mechanics: layout predicate, target parsing, INDEX.md generation, audit link rules, coverage blast radius, opt-in migration, lock ordering.

## Module Layout (`fs/knowledge/`)

Split by concern when the tiering work pushed `dir.rs` past the 400-line cap; every public
method signature was kept stable so no caller changed.

| File                                | Owns                                                                                                                                                    |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `types.rs`                          | `KnowledgeFile`, `KnowledgeTarget`, `KnowledgeLayout`, `INDEX_FILENAME`, the tier-1 alias table                                                          |
| `dir.rs`                            | `KnowledgeDir` — `initialize`, `append_target`, `replace_section_target`, `layout` detection, index read/write                                          |
| `index.rs`                          | `scan_topics`, `generate_index`, `write_index`                                                                                                           |
| `catalog.rs` (+ `catalog/prose.rs`) | `catalog::build` — deterministic chunk list plus `CatalogIssue` diagnostics (duplicate heading, generic blurb, broken link, missing source ref) over the curated tree; `prose.rs` extends the same catalog with the project's configured prose roots |
| `chunker.rs`                        | `chunk_file` — splits one file into heading-anchored `KnowledgeChunk`s, extracts links and backticked source-path references                             |
| `splice.rs`                         | `splice_section` — in-place `#{2,6}` heading section replace/append, backing `replace_section_target`                                                    |
| `scaffold.rs`                       | tier-2 stub-header detection/healing helpers used when a new topic file is created                                                                       |
| `templates.rs`                      | tier-1 and tier-2 file scaffolds                                                                                                                          |

There is no `gc.rs` or `summary.rs` in this module — an earlier version of this doc invented both.

The alias table lives in `types.rs`, not the CLI layer: `commands/knowledge/mod.rs::update`/`replace_section`
resolve their `file` argument through `KnowledgeTarget::parse` (`types.rs:146-170`), which matches the
no-slash case against `KnowledgeFile::parse` (`types.rs:167`), so the data layer and the CLI cannot drift.
There is no `parse_file_type` function anywhere in the tree — a name an earlier version of this doc invented.

## Layout Predicate

A knowledge directory is **`Hierarchical` iff `INDEX.md` exists**, otherwise `Legacy`
(`dir.rs::layout`). Nothing else is consulted — not topic directories, not content, not links.
Creating `INDEX.md` flips the layout; deleting it downgrades instantly.

## Targets: Tier-1 vs Tier-2

`KnowledgeTarget::parse` splits on the first `/`. No slash → a tier-1 file (resolved through the
alias table: `arch`/`map`/`overview` → `architecture.md`, `lessons` → `mistakes.md`, and so on).
One slash → a tier-2 topic at `<category-dir>/<slug>.md`. A second slash is rejected: topics are
exactly one level deep.

Slugs go through `validation::validate_id` — ASCII alphanumeric plus `-` and `_` only (no dots),
128 chars max, no reserved device names, and **no leading two-digit `NN-` prefix**. A trailing
`.md` is stripped, so `architecture/foo` and `architecture/foo.md` are the same target. The
category directory is created automatically on first write.

## INDEX.md Generation

There is no dedicated index-generation verb. `INDEX.md` regenerates automatically —
`KnowledgeDir::refresh_index_if_hierarchical` (`dir.rs:286`) calls `generate_index`
(`index.rs:160`) after every `loom knowledge update`, and `loom knowledge sync` forces
it structurally too. The generated file opens with a generated-file marker and a header
blockquote telling the reader to read the index first, then only what it points to
(`index.rs:164-171`). Next comes a **Tier 1** table (`| File | Description | Lines |`) and a
**Tier 2** table grouped by `### <category>`, header `| Topic | Blurb | Lines |`, rows
`| [<slug>](<category>/<slug>.md) | <blurb> | <lines> |` — the Title column is gone, since the
slug link plus blurb already carry what it repeated. The Tier 2 section is omitted entirely when
no topics exist. Blurbs are capped at `MAX_BLURB_CHARS = 100` characters (`index.rs:65`),
truncated on a word boundary with an ellipsis.

`scan_topics` is **non-recursive** — it reads `<root>/<category>/*.md` only, skipping dotfiles
and non-`.md` entries. Nested subdirectories under a category are ignored completely. Title is
the first `#` line, blurb the first `>` line (`index.rs:90-101 extract_title_and_blurb`),
falling back to the slug and an empty string.

Regeneration is idempotent and does a full atomic overwrite, so hand edits to `INDEX.md` are
silently destroyed. Every `loom knowledge update` refreshes the index — but **only once the
directory is already hierarchical**. The generated index is itself capped at
`MAX_INDEX_BYTES = 12_288` bytes (`catalog/size.rs:18`); exceeding it surfaces as an
`OversizedIndex` catalog issue (`loom knowledge check`) rather than being repaired
automatically.

## Audit Rules — the Two Checks Disagree About Link Form

This heading is inherited from an earlier version of this doc, which described a `gc.rs`-based
system with two disagreeing link-form checks. That system does not exist anywhere in the tree.

What actually runs today is `fs::knowledge::catalog::build` (`catalog.rs:150`), which walks every
curated `*.md` file under the knowledge root (skipping `INDEX.md`) and reports four `CatalogIssue`
kinds, sorted deterministically:

- **`DuplicateHeading`** — the same normalized H2+ anchor occurs more than once in one file.
- **`GenericBlurb`** — a topic's `>` blurb still matches the unmodified scaffold text for its category.
- **`BrokenLink`** — a markdown link target does not resolve to a real file, by real lexical path
  resolution (`.`/`..` folding relative to the linking file, `catalog.rs:295-322`), not a regex on
  link syntax. An absolute target, or one that folds outside the knowledge root, is skipped rather
  than flagged. Any link form resolves the same way — there is no special-cased "only this exact
  markdown form counts" rule.
- **`MissingSourceRef`** — a backticked repository-relative path in the file does not exist on disk.

`catalog::build` never repairs anything (`context/ingest.rs:9-14` states the hard constraint) and
runs over the curated tree only — chunks pulled in from the project's configured prose roots
contribute no issues. `loom knowledge sync` is the CLI surface that runs it and prints the issue
count (`commands/knowledge/sync.rs`); nothing gates a write on the result today.

Index staleness is a separate, lighter check inside the same build path: whether the on-disk
`INDEX.md` textually contains every tier-1 filename and topic path. Line counts in the index are
not compared, so stale numbers are never flagged.

There is no per-link "form" requirement — a human title pointing to `category/slug.md` is simply
the house style (see `patterns.md`), not something an audit enforces.

## Thresholds

None of `SECTION_EXTRACT_THRESHOLD`, `DEFAULT_MAX_TIER1_LINES`, `DEFAULT_MAX_TOPIC_LINES`, or
`DEFAULT_MAX_PROMOTED_BLOCKS` exist anywhere in the tree — an earlier version of this doc invented
all four. No file-size, section-size, or promoted-block-count limit is enforced in code today.

The "tier-1 section past ~40 lines spills into a topic" rule (CLAUDE.md Rule 12) is prose-only —
a convention for authors to apply by hand, not a check `loom knowledge sync` or anything else
runs. The four `CatalogIssue` kinds `catalog::build` actually reports (duplicate heading, generic
blurb, broken link, missing source ref — see the Audit Rules section above) say nothing about size.

## Coverage Blast Radius

`architecture_coverage_text()` does not exist anywhere in the tree — an earlier version of this
doc invented it, along with the coverage-weighted-retrieval mechanism it described. No function
concatenates tier-1 `architecture.md` with tier-2 Architecture topics to weight `src/` directory
matches; nothing in `fs/knowledge/` or `context/` does that. (`context/coverage.rs` does define a
`CoverageReport`, but it reports source-graph parse coverage — full / lexical-only / parse-error
per file — which is unrelated to knowledge docs.)

`commands/knowledge/check.rs` and a `--min-coverage` gate do not exist either. The current
`loom knowledge` CLI surface is five subcommands, all dispatched from `cli/dispatch.rs:83-107`:
`update`, `replace-section`, `context`, `eval` (scores retrieval against a checked-in case file),
and `sync`.

## Migration Is Opt-In (a Deliberate Backwards-Compatibility Exception)

This project otherwise forbids compatibility shims and migration routines. The knowledge layout
is the documented exception: `KnowledgeDir::initialize()` captures `let fresh = !root.exists()`
and writes `INDEX.md` **only for a directory it just created**. An existing flat knowledge base
is never auto-migrated, and `KnowledgeLayout` keeps its `Legacy` arm indefinitely.

The reason is that a knowledge base is **user-curated prose, not code**. Silently restructuring
thousands of lines of someone's writing as a side effect of an unrelated command is destructive
and unreviewable; a wrong migration cannot be recovered by re-running a build. Upgrading is
therefore explicit: run `loom knowledge sync` to regenerate structure (`INDEX.md`) for an
already-hierarchical directory, or write a first `INDEX.md` by hand (any `loom knowledge update`
against a freshly-created directory does this too) to opt a `Legacy` directory in. The dedicated
`gc`-driven compaction verb this section used to describe is gone along with the rest of the
collapsed CLI surface.

## Locking

`fs/locking.rs` locks a file's **parent directory**, not the file. `INDEX.md` and all tier-1
files share the knowledge root, so an index refresh must run **after** `locked_read_modify_write`
returns — calling it inside the closure self-deadlocks on the same thread, because `flock` is
per open file description. Tier-2 writes lock `<root>/<category>/` and therefore never collide
with an index write. Refresh failures warn to stderr and return `Ok`, so that a successful
content write is never retried into a double append.
