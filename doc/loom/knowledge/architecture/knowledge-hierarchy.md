---
sources:
- loom/src/fs/knowledge/catalog.rs
- loom/src/commands/knowledge/check.rs
verified: 054528e508d51ede343e254590cdb73ae00f7df6
---
# Knowledge Hierarchy

> Read before touching fs/knowledge: targets, INDEX.md, checks, size limits

## Module Layout (`fs/knowledge/`)

Split by concern when the tiering work pushed `dir.rs` past the 400-line cap; every
public method signature was kept stable so no caller changed.

| File | Owns |
| --- | --- |
| `types.rs` | `KnowledgeFile`, `KnowledgeTarget`, `KnowledgeLayout`, `INDEX_FILENAME`, the tier-1 alias table |
| `dir.rs` | `KnowledgeDir` — `initialize`, `append_target`, `replace_section_target`, `layout` detection, index read/write, `refresh_index_if_hierarchical` |
| `index.rs` | `scan_topics`, `generate_index`, `write_index`, `MAX_BLURB_CHARS` |
| `catalog.rs` | `catalog::build` — deterministic chunk list plus `CatalogIssue` diagnostics over the curated tree (see *Audit Rules* below) |
| `catalog/issue.rs` | the `CatalogIssue` enum and `is_review_only` |
| `catalog/size.rs` | the three size limits and their checks |
| `fs/knowledge/catalog/evidence.rs` | `changed_since_verified` — the `EvidenceChanged` check against frontmatter `sources` and `verified` |
| `catalog/source_roots.rs` | resolving backticked source paths: project root, cargo package source roots, unique path suffix, or basename |
| `catalog/order.rs` | deterministic issue ordering |
| `catalog/prose.rs` | indexing the configured prose roots into the same catalog, with lifecycle derived from the path |
| `chunker.rs` (+ `chunker/references.rs`) | `chunk_file` / `chunk_sections` — heading-anchored `KnowledgeChunk`s, links, and typed backticked source references |
| `frontmatter.rs` | leading YAML frontmatter (`id`, `aliases`, `state`, `sources`, `verified`), read by the chunker and written by `loom knowledge annotate` |
| `splice.rs` | `splice_section` — in-place `#{2,6}` heading section replace/append, backing `replace_section_target` |
| `scaffold.rs` | tier-2 stub-header detection/healing helpers used when a new topic file is created |
| `templates.rs` | tier-1 and tier-2 file scaffolds, and `scaffold_blurb` (what `GenericBlurb` compares against) |

There is no `gc.rs` or `summary.rs` in this module — an earlier version of this doc
invented both.

The alias table lives in `types.rs`, not the CLI layer:
`commands/knowledge/mod.rs::update`/`replace_section` resolve their `file` argument
through `KnowledgeTarget::parse`, which matches the no-slash case against
`KnowledgeFile::parse`, so the data layer and the CLI cannot drift. There is no
`parse_file_type` function anywhere in the tree — a name an earlier version of this
doc invented.

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
`KnowledgeDir::refresh_index_if_hierarchical` calls `index::write_index` (which
renders through `generate_index`) after every `loom knowledge update`,
`replace-section` and `annotate`, and `loom knowledge sync` regenerates it on every
run and is the one command that creates it for a flat directory. `write_index`
skips the write when the rendered bytes already match the file on disk.

The generated file opens with `GENERATED_MARKER` (an HTML comment saying not to edit
by hand), a `# Knowledge Index` title, and a header blockquote telling the reader to
read the index first, then only what it points to. Next comes a **Tier 1** table
(`| File | Description | Lines |`) with one row per tier-1 file that exists, then a
**Tier 2** table grouped by `### <category>` with header `| Topic | Blurb | Lines |`
and one row per topic: the slug linked to its topic file, the blurb, and the line
count. There is no Title column — the slug link plus the blurb already carry what it
repeated. The Tier 2 section is omitted entirely when no topics exist.

Blurbs are capped at `MAX_BLURB_CHARS = 80` characters (`index.rs`): a longer blurb
is truncated on a word boundary, trailing punctuation trimmed and an ellipsis
appended, and `loom knowledge annotate --blurb` refuses anything over 80 outright.
Table cells escape `|`.

`scan_topics` is **non-recursive** — it reads each category directory's direct
`*.md` children only, skipping dotfiles and non-`.md` entries. Nested
subdirectories under a category are ignored completely. Title is the first `#`
line, blurb the first `>` line (`extract_title_and_blurb`), falling back to the
slug and an empty string.

Regeneration is idempotent and does a full atomic overwrite, so hand edits to
`INDEX.md` are silently destroyed. Every knowledge write refreshes the index — but
**only once the directory is already hierarchical**. The generated index is capped
at `MAX_INDEX_BYTES = 16_384` bytes (`catalog/size.rs`, roughly 4k tokens for the
first read of every session); exceeding it surfaces as an `OversizedIndex` issue
from `loom knowledge check` rather than being repaired automatically.

## Audit Rules — Nine Catalog Issue Kinds

An earlier version of this doc described a `gc`-based system with two disagreeing
link-form checks, and a later one listed four issue kinds plus an index-staleness
text check. Neither matches the tree: there is no link-form rule and no
index-staleness check.

`fs::knowledge::catalog::build` walks every curated markdown file under the
knowledge root (recursing into category directories, skipping `INDEX.md` and
dotfiles) and reports nine `CatalogIssue` kinds (`catalog/issue.rs`), sorted
deterministically by `catalog/order.rs`:

- **`DuplicateHeading`** — the same normalized H2+ anchor occurs more than once in
  one file.
- **`GenericBlurb`** — a topic's first `>` line still equals
  `templates::scaffold_blurb` for its category.
- **`BrokenLink`** — a markdown link target does not resolve to a real file, by
  lexical path resolution (`.` and `..` folded relative to the linking file,
  `contained_link_target`). An absolute target, or one that folds outside the
  knowledge root, is reported as broken without being probed on disk — an earlier
  version of this section said such targets were skipped.
- **`MissingSourceRef`** — a backticked source path classified live does not
  resolve (see *Reference classification* below).
- **`EvidenceChanged`** — a file's frontmatter declares `sources` and a `verified`
  revision, and `git diff --name-only <verified>..HEAD -- <sources>` lists one of
  them (`fs/knowledge/catalog/evidence.rs`; one git call per file, and any git
  failure skips the check for that file).
- **`UnverifiableReference`** — a backticked path classified example, runtime,
  external or historical that does not resolve; a note, since such a path is not
  expected to exist.
- **`OversizedSection`**, **`OversizedFile`**, **`OversizedIndex`** — the size
  limits under *Thresholds* below.

`EvidenceChanged` and `UnverifiableReference` are review-only
(`CatalogIssue::is_review_only`): they print as `review:` / `note:` lines, land in
the JSON `review` array instead of `issues`, and never count toward `--strict`.

**Reference classification** (`chunker/references.rs::classify_reference`, applied
to each backticked span ending in a source extension — rs, tsx, ts, py, go, sh, md,
toml, yaml, yml — outside fenced blocks, using the text around it): *runtime* when
the path starts with `.loom/`, `.work/`, `target/`, `node_modules/`, `~`, `/tmp` or
`$`, or contains an angle bracket; else *example* when the path or its sentence
carries a placeholder marker (foo, bar, baz, an angle bracket, an ellipsis,
path/to, slug, the words example or placeholder, and a few more); else
*historical* when the sentence says the path does not exist, no longer, was
removed or deleted, was renamed, used to, or "there is no" directly before it;
else *external* on markers such as upstream or another project; otherwise *live*.
A live path must exist at the project root, under a cargo package source root, as
the unique project file with that path suffix, or — bare basename — as any file
with that name (`catalog/source_roots.rs::repository_source_path_exists`). A live
path containing a slash whose first component names nothing in this project
becomes an external note instead of a `MissingSourceRef`
(`push_source_ref_issue`).

`catalog::build` never repairs anything (`context/ingest.rs` states it as a hard
constraint) and reports on the curated tree only — chunks indexed from the
configured prose roots contribute no issues.

**Surfaces.** `loom knowledge check` (`commands/knowledge/check.rs`) resolves only
the knowledge root — never the context store, so it writes nothing and is safe as a
stage acceptance criterion — and prints one line per issue. It exits 0 unless
`--strict` is set and at least one non-review issue exists, in which case it exits
1 after printing. `--json` prints `root`, `issues`, `review` and `count`, where
`count` is the strict count. `loom knowledge sync` also runs the build and prints
the issue count, but gates nothing.

There is no per-link form requirement — a human title pointing to a topic path is
simply the house style (see `patterns.md`), not something an audit enforces.

## Thresholds

The three size limits ARE enforced — as catalog issues, reported and never
repaired (`catalog/size.rs`, the mechanical form of CLAUDE.md Rule 12):

| Constant | Value | Issue |
| --- | --- | --- |
| `MAX_TIER_ONE_SECTION_LINES` | 40 | `OversizedSection` — a tier-1 `##` section over 40 lines, heading line included and trailing blank lines excluded; the headingless preamble is exempt |
| `MAX_TIER_ONE_FILE_LINES` | 250 | `OversizedFile` — a tier-1 file over 250 lines |
| `MAX_INDEX_BYTES` | 16 384 | `OversizedIndex` — a generated `INDEX.md` over 16 384 bytes |

Tier-1 is decided by path depth alone (`is_tier_one`: one path component under the
knowledge root), so tier-2 topic files are exempt from both line limits. All three
count toward `loom knowledge check --strict`. The blurb cap (`MAX_BLURB_CHARS = 80`,
`index.rs`) is separate: it truncates in the index and makes `annotate --blurb`
refuse, but raises no catalog issue.

None of `SECTION_EXTRACT_THRESHOLD`, `DEFAULT_MAX_TIER1_LINES`,
`DEFAULT_MAX_TOPIC_LINES` or `DEFAULT_MAX_PROMOTED_BLOCKS` exist anywhere in the
tree — an earlier version of this doc invented all four, and a later one wrongly
said no size limit was enforced at all.

## Coverage Blast Radius

`architecture_coverage_text()` does not exist anywhere in the tree — an earlier
version of this doc invented it, along with the coverage-weighted-retrieval
mechanism it described. No function concatenates the tier-1 architecture summary
with tier-2 architecture topics to weight source-directory matches.
(`context/coverage.rs` does define a `CoverageReport`, but it reports source-graph
parse coverage per file, which is unrelated to knowledge docs.) There is no
`--min-coverage` gate either.

`loom knowledge check` DOES exist (`commands/knowledge/check.rs`; see *Audit Rules*
above). The `loom knowledge` CLI has eight subcommands, all dispatched from
`cli/dispatch.rs::dispatch_knowledge`: `update`, `replace-section`, `annotate`
(frontmatter lifecycle state, `--source` evidence paths, the `--verified` revision,
aliases, and the topic blurb), `context`, `eval` (scores retrieval against a
checked-in case file), `telemetry` (summarizes delivery and retrieval events),
`sync`, and `check`.

## Migration Is Opt-In (a Deliberate Backwards-Compatibility Exception)

This project otherwise forbids compatibility shims and migration routines. The
knowledge layout is the documented exception: `KnowledgeDir::initialize()` captures
`let fresh = !root.exists()` and writes `INDEX.md` **only for a directory it just
created**. An existing flat knowledge base is never migrated as a side effect, and
`KnowledgeLayout` keeps its `Legacy` arm indefinitely.

The reason is that a knowledge base is **user-curated prose, not code**. Silently
restructuring thousands of lines of someone's writing as a side effect of an
unrelated command is destructive and unreviewable; a wrong migration cannot be
recovered by re-running a build. Upgrading is therefore explicit:
`loom knowledge sync` is the one command that migrates. On a `Legacy` directory it
writes the first `INDEX.md` (`upgrade_flat_layout`, a hard failure if that write
fails); on an already-hierarchical one it regenerates the index best-effort before
rebuilding the derived catalog. `update`, `replace-section`, `annotate` and every
retrieval path leave a flat directory flat. Writing an `INDEX.md` by hand also opts
a directory in, since the layout predicate checks only that the file exists. The
`gc`-driven compaction verb an earlier version of this section described is gone
with the rest of that CLI surface.

## Locking

`fs/locking.rs` locks a file's **parent directory**, not the file. `INDEX.md` and all tier-1
files share the knowledge root, so an index refresh must run **after** `locked_read_modify_write`
returns — calling it inside the closure self-deadlocks on the same thread, because `flock` is
per open file description. Tier-2 writes lock `<root>/<category>/` and therefore never collide
with an index write. Refresh failures warn to stderr and return `Ok`, so that a successful
content write is never retried into a double append.
