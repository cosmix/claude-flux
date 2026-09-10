---
sources:
- loom/src/context/refresh/source_graph.rs
- loom/src/context/graph_store/mod.rs
verified: 054528e508d51ede343e254590cdb73ae00f7df6
---
# Source Graph

> What the source graph is and is not, its honesty contract, extractor trait, node/edge and cache identity, and lifecycle.

## What It Is, and What It Is Not

`loom/src/context/source_graph/` plus `context/extract/` hold a derived,
tree-sitter-backed graph of the repository's own source: file and symbol nodes
plus edges between them. It has **two** production consumers, and both are live:

| Consumer | Route in | What it reads |
| --- | --- | --- |
| `loom map` (`--outline`, `--find-all`, `--impact`) | `context::graph_store` | the resolved layer, rendered as read-only views |
| the `Source` retrieval channel | `context::rank_source` → `fuse` → `pack` | symbol nodes, scored and fused with knowledge chunks into one `ContextPack` |

The second consumer is new. Before it existed the graph was built, persisted and
given a CLI while `Channel::Source` was ranked over nothing at all — the failure
class is in `mistakes/store-without-consumer.md`, and the ranking design that
closed it is in `architecture/context-retrieval.md`.

Nobody builds this graph by hand any more either. `loom init` and `loom run`
publish a base layer through `advisory_source_graph_preflight`, and every stage's
overlay is reconciled just before its signal is written — see *Lifecycle* below.

Types live in `context/source_graph/` (not `context/schema.rs`) because the graph
is a distinct domain from the knowledge corpus; `schema.rs` re-exports the public
names, so callers may reach them through either path
(`context/source_graph/mod.rs:8-12`). A plan that names `schema.rs` as the home
of `SourceNode`/`SourceEdge` is naming the re-export — run
`rg -n "pub struct <Type>" loom/src/` before opening the file a plan points at.

## The Honesty Contract

**This graph is never claimed to be exhaustive** (`source_graph/mod.rs:14-21`).
Every `SourceEdge` carries an `EdgeProvenance` and an explicit confidence, and a
call whose target cannot be resolved is emitted as an *inferred* edge or as
`UNRESOLVED_TARGET` (`"<unresolved>"`) — never as an authoritative parser edge,
and never silently dropped or given an invented destination. Consumers that
render or traverse the graph **must surface that confidence rather than
flattening it away**.

| Provenance | Meaning | Confidence ceiling |
| --- | --- | --- |
| `Parser` | the grammar resolved both endpoints syntactically within ONE file | `1.0` — reserved for this alone |
| `Lsp` | a language server resolved it | reserved; **nothing emits this today** |
| `Inferred` | heuristically matched across files, or unresolved | `MAX_INFERRED_CONFIDENCE = 0.5` at extraction |

`context::resolve` may raise a uniquely-matched inferred edge to at most
`MAX_RESOLVED_INFERRED_CONFIDENCE = 0.9` — deliberately below `1.0`, because
cross-file uniqueness is real evidence an extractor never had, but a unique
*name* match is still not a parse: two unrelated crates can define one name, and
a graph that omits a file omits its definitions too, so "the only match I can
see" is not "the only match" (`source_graph/mod.rs:42-51`). Resolution **only
ever raises confidence with evidence**, and never promotes `Inferred` to
`Parser`.

That ceiling discipline is the reusable idea: when a component's view of the
world is structurally narrower than the claim it is asked to make, encode the gap
as a numeric ceiling in a named constant with the reasoning in its docstring —
not as a comment at the call site.

## The Extractor Trait

`context/extract/mod.rs` — bytes in, `FileExtraction` out. Each language
implements `SourceGraphExtractor` over a pinned grammar and a tree-sitter query
embedded in that language's module. **The registry (`extract::registry()`) is the
only thing the rest of loom sees; callers never name a grammar directly.** Without
the `source-graph` feature the registry is empty.

```rust
pub trait SourceGraphExtractor {
    fn language(&self) -> DetectedLanguage;
    fn cache_identity(&self) -> ExtractorIdentity;
    fn supports(&self, path: &Path) -> bool;
    fn extract(&self, path: &Path, bytes: &[u8]) -> Result<FileExtraction>;
}
```

What an extractor promises: every node it emits corresponds to a real
declaration in the bytes it was handed, and every edge carries honest
provenance. What it does **not** promise: an exhaustive call graph. Extraction is
per-file, so a call to a symbol defined in another file is inferred or
unresolved, never a parser edge (the `context::extract` module doc). A syntax
error is data (`FileCoverage::ParseError`), never an `Err`. Cross-file resolution
is `context::resolve`'s job.

**One shared harness, not four implementations.** The tree-sitter walk lives in
the directory module `context/extract/treesitter/mod.rs` (`run_query`, with
`treesitter/build.rs` and `treesitter/collect.rs`), parameterized by the
per-language `QueryHarness` trait. A language module supplies only a grammar, a
query using the `@definition.<kind>` / `@name` / `@import.path` / `@call.name`
capture protocol, and a capture-to-kind mapping (`QueryHarness::kind_for_capture`).
A `@definition.*` match with no `@name` counts toward `FileCoverage::Partial`
instead of becoming an anonymous node. This was deliberate: the honesty constraint
(provenance, the 0.5 ceiling) is a property four separate `extract()`
implementations would each have to remember and any one could silently break.
Centralizing makes it structural. It also made the four language workers
genuinely disjoint and parallelizable.

## Node and Edge Identity

- File node id: the relative path, forward-slashed (`file_node_id`).
- Symbol node id: `<relative-path>#<kind>:<scope-joined-by-::>`, scope
  outermost-first, joined with `::` regardless of language so ids are comparable
  across extractors. Empty scope is invalid (`source_graph/mod.rs:68-89`).
- **The kind is part of the id because scope alone is not unique.** Rust's
  `struct Widget` and `impl Widget` share a name, as do a TypeScript `interface
  Foo` and a `const Foo`. Keying on scope alone let an implementation node
  silently shadow the type it implements, collapsing two distinct nodes into one
  and making their `Contains` edges indistinguishable, so a traversal could not
  tell which parent a method belonged to. The id was scope-only when first
  written and the collision was caught inside the same stage; the docstring now
  carries the reasoning so it is not "simplified" back.
- `SourceNodeKind`: `File`, `Function`, `Type`, `Interface`, `Module`,
  `Constant`, `Implementation`. `SourceEdgeKind`: `Contains`, `Imports`, `Calls`,
  `References`, `Implements`, `Extends`. Both have `as_str()` giving the stable
  lowercase name used in ids, CLI output and fixture JSON — so renaming a variant
  breaks golden fixtures and node ids at once.

## Cache Identity

`ExtractorIdentity` (`context/extract/mod.rs`) is what stops a cached extraction
from an older build being silently reused:

| Field | Source |
| --- | --- |
| `grammar_version` | version of the pinned tree-sitter grammar crate |
| `query_digest` | `sha256:<hex>` over the embedded query source |
| `extractor_version` | `u32`, **bumped by hand** whenever the walking logic changes shape |

**Any change to the pinned grammar, the embedded query, or the walking logic must
change this.** The grammar and query halves are automatic; `extractor_version` is
not — changing how the walk builds nodes without bumping it serves stale cached
extractions with no error anywhere. `to_parser_version()` renders a compact form —
grammar version, the first 12 hex digits of the digest and `v<extractor_version>`,
joined by `+` — stored on every node as `SourceNode::parser_version`, small enough
to repeat per node.

Content identity is `body_hash(bytes)` = `sha256:<hex>`, the one definition
(`context/source_graph/mod.rs`). `build_layer` (`refresh/source_graph/layer.rs`)
tries two reuses before it parses a file, each against the previous layer and the
base:

1. **By git blob id** (`reuse_by_oid`): the path is not dirty and its current blob
   id equals the one that layer's `blob_index` recorded — no read, no hash.
2. **By content hash** (`reuse_by_hash`): after reading and hashing the bytes, the
   recorded `content_hash` matches.

Both also require `parser_version_matches` — the entry's first node was produced
by the extractor identity that is current now — so bumping an identity forces a
re-parse. An unreadable file's entry and an overlay tombstone both carry an empty
`content_hash`, which can never equal a real `body_hash`, and `reuse_by_oid` skips
empty hashes; that is what makes keeping those entries safe.

## Coverage: Nothing Ever Vanishes

`FileCoverage` (`context/source_graph/node.rs`) records why a file got less than
full treatment. No path silently disappears (the `context::extract` module doc):

| Situation | Result |
| --- | --- |
| no grammar for the language | file node, `FileCoverage::LexicalOnly` |
| file over `MAX_EXTRACTED_FILE_BYTES` (512 KiB) | file node, `FileCoverage::Oversized` |
| grammar reports a syntax error | file node, `FileCoverage::ParseError` |
| a `@definition.*` match had no `@name` | named definitions still emitted, `FileCoverage::Partial` |
| `source-graph` cargo feature disabled | file node, `FileCoverage::LexicalOnly` |
| unreadable file | entry with no nodes and an empty `content_hash`, `FileCoverage::LexicalOnly` ("unreadable: …"), see `unreadable_entry` |
| file deleted in the working tree | overlay tombstone with no nodes, `FileCoverage::Deleted` |

`Deleted` is the one variant that records a deletion rather than a degraded
extraction, and only an overlay carries it — see *Building and Persisting*. That is
the coverage contract: a degraded file is *reported as degraded*, never omitted.
`context::coverage::CoverageReport` aggregates it. When you add an extractor, the
degraded paths are the ones to test — the happy path fails loudly, the degraded
paths fail silently.

## Building and Persisting

`refresh::source_graph::reconcile_source_graph(store, graph_store, project_root, scope)`
is the builder: inspect the working tree, enumerate files through git, reuse or
re-extract each one, and persist the resulting `GraphLayer` through `GraphStore`.
`ensure_snapshot` (see *Lifecycle*) is the policy layer most callers go through; it
drives the same builder.

- **Working tree** (`refresh/source_graph/generation.rs::working_tree`): `HEAD` plus
  every dirty path from `git status --porcelain=v1 -z --untracked-files=all` (a
  rename records both paths). Its `generation` is a sha256 over `HEAD` and one
  `<status> <path> <content-hash|deleted>` line per dirty path; a clean tree's
  generation is `clean_generation(HEAD)`. Comparing generations is how an overlay
  proves it is current without re-walking anything.
- **Enumeration** (`refresh/source_graph/enumerate.rs`), path → git blob id:
  - `SourceGraphScope::Base { revision }` lists **committed** content
    (`git ls-tree -r -z HEAD`), and `build_layer` reads a dirty path's bytes with
    `git show HEAD:<path>` rather than from disk. A base therefore always describes
    committed `HEAD` and can be published from a dirty checkout; the earlier rule
    that a dirty tree refused a base publish is gone.
  - `SourceGraphScope::Overlay { plan, stage }` lists the index
    (`git ls-files -s -z`) plus **untracked** files
    (`git ls-files --others --exclude-standard -z`, existing and not excluded), and
    records index paths missing from disk as deleted.
- **Tombstones.** In an overlay, every deleted path — an index path missing from
  disk, or a dirty path absent from disk — becomes `FileEntry::tombstone()`: no
  nodes, empty hash, `FileCoverage::Deleted`. `persist_layer` keeps a tombstone only
  when the base has that path, and keeps any other entry only when it differs from
  the base's; `GraphStore::save_overlay` applies the same tombstone filter.
  `GraphStore::resolved` REMOVES a tombstoned path from the resolved view instead of
  shadowing it, so a file a stage deleted no longer shows up in `loom map` or the
  source channel.
- **`GraphLayer`**: `revision` (a base's commit; for an overlay, the `HEAD` it was
  cut from), `generation` (the tree generation for an overlay, empty for a base),
  `built_at`, `files` (path → `FileEntry`), and `blob_index` (path → the git blob id
  whose bytes produced that entry). An overlay write is skipped when revision,
  generation, files and `blob_index` all equal the previous overlay's;
  `GraphStore::publish_base` never overwrites a revision already published.
- **`SourceGraphOutcome { nodes, edges, freshness, counters }`** describes the layer
  as built by THIS call; `counters.bytes_serialized` is 0 when nothing was written.
  A working-tree or enumeration failure is **data, not a crash**: the outcome is
  degraded, with `Freshness::never_built(detail)` and zero counts, and the stored
  semantic layer is marked stale.
- **`SourceGraphCounters`**: `files_enumerated`, `files_hashed`, `files_parsed`,
  `files_reused`, `files_deleted`, `files_untracked`, `bytes_serialized`, and
  `enumerate_ms` / `hash_ms` / `parse_ms` / `persist_ms`.
  `SnapshotOutcome::describe` prints the parsed, reused and deleted counts on its
  advisory line.
- `EXCLUDED_ROOTS` = `.loom`, `.work`, `.worktrees`, `target`, `node_modules`, `.git`
  (`refresh/source_graph.rs`), applied to enumerated, untracked and dirty paths
  alike.
- `context` reaches `git` only through `git::runner::run_git_checked`, from
  `enumerate.rs`, `generation.rs` and `layer.rs` under `refresh/source_graph/`. That
  is a deliberate downward edge, not a layering violation.

## Stack

Six dependencies, all `optional = true`, all behind ONE default-on cargo feature
`source-graph` (`loom/Cargo.toml:41-46`, `:63-77`), exact-pinned with `=`:
`tree-sitter =0.27.0`, `tree-sitter-rust =0.24.2`,
`tree-sitter-typescript =0.23.2`, `tree-sitter-python =0.25.0`,
`tree-sitter-go =0.25.0`, `streaming-iterator =0.1.9`.

- `streaming-iterator` is not incidental: `QueryCursor::matches` returns a
  `StreamingIterator`, not a plain `Iterator`.
- **Why one feature and not six.** `cargo add` generates one implicit feature per
  optional dep, which would let a host disable half the grammars and leave the
  extractor registry inconsistent. Collapsing them makes
  `--no-default-features` the only supported degraded mode, and that mode falls
  back to file-level lexical nodes rather than failing to build — the point is
  that a host without a C toolchain can still build loom.
- Core-crate upgrades carry API breakage: 0.26 → 0.27 turned
  `QueryMatch::captures` from a public field into a method
  (`context/extract/treesitter/collect.rs:108`). The grammar crates were
  unaffected — they bind through `tree-sitter-language`, not the core ABI.

## Lifecycle: Who Builds It, and When

Nothing in the normal path asks a human to build the graph, and every entry point
is **advisory** — it reports failure and continues, because a missing graph must
degrade retrieval, never block a run.

**One decision path: `ensure_snapshot(store, graph_store, project_root, policy)`**
(`context/refresh/snapshot.rs`). It inspects the working tree once, then:

| `SnapshotPolicy` | Does |
| --- | --- |
| `BaseOnly` | ensure the base for `HEAD` |
| `LocalCurrent` | ensure the base for `HEAD`; when the tree is dirty (its generation differs from `clean_generation(HEAD)`), also bring the checkout's `_local` overlay (`local_overlay_key`) current |
| `StageOverlay { plan, stage }` | bring that stage's overlay current; no base publish |

A base for `HEAD` is reused when it exists and is `layer_is_current` (every file's
first node carries the current extractor's parser version); a base built by an
older extractor identity is deleted and rebuilt. An overlay is reused when its
`generation` equals the tree's and it is `layer_is_current`. The result is a
`SnapshotOutcome { action, reason, revision, generation, overlay, counters, elapsed }`
whose `SnapshotAction` is `Reused`, `Updated` (some files reused), `Rebuilt` or
`Unavailable`; `describe()` renders the single `source graph: …` advisory line
every surface prints.

| When | Call site | Policy or scope |
| --- | --- | --- |
| `loom init` | `commands/init/execute.rs`, via `advisory_source_graph_preflight` | `BaseOnly` |
| `loom run` (daemon) | `commands/run/mod.rs::prepare_background_run` | `BaseOnly` |
| `loom run --foreground` | `commands/run/foreground.rs` | `BaseOnly` |
| `loom map`, `loom knowledge sync` | `commands/map.rs`; `refresh::semantic::try_reconcile_semantic` | `LocalCurrent` |
| prompt-hook self-heal | `loom hook reconcile-graph` (`commands/hook/reconcile_graph.rs`), spawned detached when a pack is stale or degraded | `HookTarget::snapshot_policy`: `StageOverlay` in a stage, `LocalCurrent` in a checkout |
| before a stage's signal is written, and before a merge | `MergeLifecycle::reconcile_overlay`, from `stage_executor.rs` (fresh spawn), `skip_retry.rs` (recovery), `merge_handler.rs` and `progressive_complete.rs` | `reconcile_source_graph` with `Overlay { plan, stage }` |
| after a merge | `MergeLifecycle::reconcile_base` | `reconcile_source_graph` with `Base { revision }` of the merged revision |

`advisory_source_graph_preflight(repo_root, work_dir)` (`commands/run/checks.rs`)
never returns a `Result`, so it cannot bail startup: it prints the `describe()` line
unless the base was simply reused, and one `source graph: unavailable (...)` line on
error. It is modelled on `advisory_codex_lane_preflight`. The `publish_source_graph`
helper and the `allow_overlay_fallback` parameter an earlier version of this section
described no longer exist.

**Ordering in `loom run`.** The preflight now runs AFTER
`plan_inputs::mark_plan_in_progress`, which commits the plan-file rename, so the
base is published against the committed `IN_PROGRESS-` filename and the revision
stages inherit. The earlier rule — preflight first, because the rename dirtied the
tree and a dirty tree refused a base publish — no longer applies, since bases are
built from committed content.

**Recovery signals need their own call.** Signal bytes are embedded once at write
time and `start_stage` later re-uses them verbatim from disk, so a crash/hang retry
that did not reconcile first would hand the agent a stale overlay (`skip_retry.rs`).
`start_knowledge_stage` deliberately has no reconcile call — it runs in the main
repo with no worktree, and `reconcile_overlay` returns early when the stage has no
worktree directory.
