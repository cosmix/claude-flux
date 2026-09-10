# Automatic Knowledge Source Graph Followups

> Knowledge-plan followups, retrieval-degradation gap, stopwording, resolved items

## Open After PLAN-automatic-knowledge-and-source-graph (2026-08-18)

**Whole-file read ahead of the size cap.** `context/refresh/source_graph.rs:228` does
`fs::read` on every tracked file BEFORE `extract_file` applies the 512 KiB
`MAX_EXTRACTED_FILE_BYTES` cap, so the cap bounds parsing but not allocation, and the
daemon spikes to the size of the largest tracked blob on every merge reconcile.
Deliberately not fixed at the quality gate: `FileExtraction::file_level`
(`extract/mod.rs:103`) needs the BYTES to build the file node's span, so avoiding the
read means changing the oversized node's span semantics or threading a streamed line
count through the extractor API — a hot-path refactor. Peak is one file at a time and
`EXCLUDED_ROOTS` already skips `target/` and `node_modules/`, so the realistic worst
case is a transient spike, not corruption.

**Four production-dead `KnowledgeDir` methods.** Deleting `loom knowledge show`/`list`
orphaned part of the read/replace side: `read` (`dir.rs:120`), `append` (`dir.rs:127`),
`read_index` (`dir.rs:160`), and `replace_section` (`dir.rs:136`, the
`KnowledgeFile`-keyed variant) have no non-test callers, and all are `pub` on a `pub`
type so clippy cannot see them. They were kept because ~15 tests in `tests_dir.rs`
exercise them against each other (append → read, replace_section → read), so deleting
the methods deletes most of that file's coverage. **Settle them deliberately in one
follow-up: either delete methods and tests together, or wire them to a real consumer.**
General rule: when a stage deletes a read-side CLI verb, audit every accessor that verb
was the last caller of — and when a brief justifies keeping a module by naming a
caller, check whether that caller is itself reachable. A wrapper is not a consumer. The
converse also held here: `loom knowledge replace-section` was restored as a live CLI
verb (`cli/types_memory.rs:19`, `cli/dispatch.rs:84-88`, `commands/knowledge/mod.rs:115`),
which revived two of the original six dead methods — `read_target` (`dir.rs:176`, now
called at `commands/knowledge/mod.rs:126`) and `replace_section_target` (`dir.rs:212`,
now called at `commands/knowledge/mod.rs:130`). A dead-accessor list like this one is
only true against one revision; re-check it before trusting it.

**Plan-key normalisation on the writer side.** `delivery::plan_key` resolves both a blank
`plan_id` in `.work/config.toml` and a stage record with no plan to `"default"`;
`MergeLifecycle`'s writer side does not normalise identically. Silent by construction —
see `mistakes/writer-reader-address.md`.

**A permission deny now reaches child processes.** The knowledge tree is denied to the
agent AND to the `loom` binary the doctrine tells agents to use. See Part C of the
pending-knowledge document, and `concerns/sandbox-write-rules-inert.md` for the history.

**`fs/permissions/constants.rs`** still declares `LOOM_PERMISSIONS_WORKTREE` with
`Write(.work/**)` / `Bash(loom *)` rules that read like a blanket grant but have no real
consumers, and `Write(path)` rules are inert anyway. A documented fossil.

## Resolved: `Channel::Source` and the Source-Graph Deletion Gap (2026-08-17, both resolved by 2026-09-10)

Two related concerns, once open, are now closed:

- **`Channel::Source` is now consulted.** It used to be accepted everywhere (`--scope source` parsed,
  advertised in `--help`, threaded into `PackRequest.scope`) but `rank_channels` ranked it over an
  empty slice, so every pack named a scope it never searched. `context/rank_source.rs` now scores
  source-graph nodes for real, fused with the knowledge ranker by `context/fuse.rs`. The historical
  trail of dead shapes left by shipping the store without the consumer — `ItemKind::SourceNode`,
  `ResolvedGraph::node()`, `ContextItem.excerpt`'s unreachable `None` arm — is still catalogued in
  `mistakes/store-without-consumer.md` as a lesson, even though the specific defect it names is fixed.
- **Overlay deletions are now tombstoned.** `GraphStore::resolved` used to compute `overlay ∪ base`
  with no way to express a file a stage deleted, so `loom map --outline <deleted-file>` kept printing
  the stale base outline. `context/graph_store/mod.rs` now carries `FileCoverage::Deleted` entries
  (see `:85`, `:291`, `:341`) that suppress the base outline once a stage removes the file.

## Retrieval Cannot Distinguish 'No Source Graph' From 'Healthy' (2026-09-01)

`degraded_reason` (`context/retrieve/graph.rs:116-124`) returns `None` when
`semantic_revision` is empty — the never-built case — which is the same value it returns for a
healthy graph. In a checkout with no `.work/` (so no context store, so no graph), the Knowledge
Brief therefore prints `Structural: current` with no `DEGRADED` marker while serving
knowledge-only results.

Observed 2026-09-01 in the loom repo itself: the hook was given the query _how does
`ensure_work_symlink` plant the worktree symlink and what calls it_ — written to need the source
lane — and returned five knowledge chunks, 130 omitted, and zero `Channel::Source` items, with a
clean status line. `loom map --outline/--find-all/--impact` all failed with
`.work directory does not exist` at the same time.

This is the shape recorded in [visibility-and-reachability.md](../mistakes/visibility-and-reachability.md):
an `Option` that is `None` for two reasons cannot gate a claim about either.

**Deliberately NOT fixed on the spot.** The doc comment at `graph.rs:96-115` shows the predicate
was tuned carefully: it is a live input to
`commands::hook::reconcile_graph::spawn_if_needed`, which fires a detached full-repository
tree-sitter rebuild on `stale OR degraded`. Widening it to report the never-built case as degraded
would make every prompt in an uninitialised checkout start an unbounded background rebuild,
throttled only by the reconcile debounce lock — the exact failure the current shape exists to
avoid.

**Fix shape if taken up:** carry the never-built case as its OWN field rather than folding it into
`degraded`, so the status line can say it without feeding the rebuild trigger. Resolve the question
at its own source, and default in the fail-safe direction (do not claim currency).

## Stopwording drops the words a natural-language source-graph question is asked in (2026-09-06)

**Observed:** `loom knowledge context --query "how is the source graph refreshed after a commit"` (`--explain`) drops `source`, `graph`, `commit` as corpus-ubiquitous and returns three unrelated low-confidence chunks; `architecture/source-graph.md#lifecycle-who-builds-it-and-when` — the section that answers it — is not in the pack. The same question phrased with a symbol (`reconcile_base`) ranks the right function first. `loom knowledge eval` still passes at precision@5 = 1.00 because its cases are symbol- or path-shaped.

**Why it matters:** the knowledge-first doctrine now tells every session to pull a question instead of reading; a pull that misses on the project's own vocabulary sends the reader back to paging files.

**Where to look:** `context/rank/corpus/stopwords.rs` (the corpus-derived stopword threshold and its rescue floor, described in `architecture/context-retrieval.md#corpus-derived-query-stopwording-with-a-rescue-floor`), and `loom/eval/retrieval-cases.yaml`, which has no natural-language lifecycle case. A first step is adding that case so the gap is measured before the threshold is tuned.
