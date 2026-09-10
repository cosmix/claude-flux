# B1 — Snapshot identity: working-tree generation, tombstones, untracked files, committed-content bases, honest counters

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `source-graph-snapshot` · Wave 0 (runs ALONE; B2 and B3 follow).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F2, F3, F5, F14, F18)

- **F2.** `semantic_freshness_against_head` (`loom/src/context/refresh.rs`) compares the stored revision with `git rev-parse HEAD` and nothing else: an edit at unchanged HEAD reports `stale: false` while retrieval returns the old symbols.
- **F3.** `tracked_source_files` (`refresh/source_graph.rs`) drops paths git lists but the tree no longer has; the overlay has no deletion marker; `GraphStore::resolved` (`graph_store/mod.rs`) seeds from the base and only inserts. A deleted file's base entry is resurrected.
- **F5.** Enumeration is `git ls-files -z` only; an untracked new source file is invisible until something adds it to git.
- **F18.** `build_layer` reads and sha256-hashes EVERY enumerated file on every run, even when it reuses every entry; `files_extracted = layer.files.len()` counts reused files as extracted; a `Base` publish reuses only a base for the exact same revision (so a documentation-only commit re-parses every source file); and any dirty tracked file (a regenerated `INDEX.md` included) refuses the base outright.
- **F14.** `refresh` fingerprints the knowledge tree twice on a stale catalog (`evaluate` and `rebuild_and_persist`).

`retrieval-contracts` (already merged) added `GraphLayer::{generation, blob_index}` and `FileCoverage::Deleted` for you. Codex's `loom map` answers from the published base and will not show those edits — read `graph_store/mod.rs` and `source_graph/node.rs` directly.

## Files you own (write)

- `loom/src/context/refresh/source_graph.rs` (split into `source_graph/{enumerate.rs, generation.rs, layer.rs}` submodules as needed — every file under 400 lines)
- `loom/src/context/refresh/semantic.rs`
- `loom/src/context/refresh.rs`
- `loom/src/context/graph_store/mod.rs`
- `loom/src/context/retrieve/graph.rs` and the few lines in `loom/src/context/retrieve.rs` that consume its result
- `loom/src/context/coverage.rs` (only if a `Deleted` entry needs counting — resolved graphs never contain one, so probably not)
- Tests: `loom/src/context/refresh/tests_source_graph.rs`, `refresh/tests_freshness.rs`, `context/graph_store/tests.rs`, `context/tests/retrieve_source.rs`
- COMPILE-FIX ONLY (rename fields/variants so the crate builds; no behavior change — B2 rewires these next): `loom/src/commands/run/checks.rs`, `loom/src/commands/knowledge/sync.rs`, `loom/src/orchestrator/merge_lifecycle.rs`, `loom/src/commands/hook/reconcile_graph.rs`, `loom/src/commands/run/tests.rs`, `loom/src/commands/knowledge/tests_sync.rs`

Read-only: everything else under `commands/**`, `map/**` and `resolve/**` (B3), `context/extract/**`, `context/source_graph/**`, `lexical_index/**`, `git/runner.rs` (`run_git_checked(args, cwd)` is the one git helper you use).

## Contract

### Enumeration (`source_graph/enumerate.rs`)

```rust
pub(crate) struct Enumeration {
    /// Path → blob object id, from `git ls-tree -r -z HEAD` (Base) or `git ls-files -s -z` (Overlay).
    pub known: BTreeMap<String, String>,
    /// Existing untracked files from `git ls-files --others --exclude-standard -z` (Overlay only).
    pub untracked: Vec<String>,
    /// Known paths absent from the working tree (Overlay only).
    pub deleted: Vec<String>,
}
pub(crate) fn enumerate(project_root: &Path, scope: &SourceGraphScope) -> Result<Enumeration>
```

`EXCLUDED_ROOTS` filtering (first path component) applies to every list. `-z` output is NUL-separated; `ls-tree` rows are `<mode> <type> <oid>\t<path>`; `ls-files -s` rows are `<mode> <oid> <stage>\t<path>`.

### Working-tree generation (`source_graph/generation.rs`)

```rust
pub(crate) struct WorkingTree {
    pub head: String,
    /// sha256 hex over `head`, then one line per dirty path: `<xy> <path> <content_hash|deleted>`.
    pub generation: String,
    /// Path → status code from `git status --porcelain=v1 -z --untracked-files=all` (renames yield both paths).
    pub dirty: BTreeMap<String, String>,
}
pub(crate) fn working_tree(project_root: &Path) -> Result<WorkingTree>
pub(crate) fn clean_generation(head: &str) -> String   // the generation of a tree with no dirty paths
```

`content_hash` is `body_hash(bytes)` of the dirty file on disk (only dirty files are read). Paths under `EXCLUDED_ROOTS` are ignored. This is the ONE working-tree probe every caller uses; `dirty_tree_reason` is deleted.

### Layer building (`source_graph/layer.rs`)

- **Base scope** reads the COMMITTED tree: file set = `enumeration.known` from `ls-tree HEAD`; untracked never enters a base. For a path whose status is dirty (modified, deleted, or staged), bytes come from `git show HEAD:<path>`; otherwise from disk (identical bytes by definition). The dirty-tree refusal is GONE: a base for HEAD is always publishable.
- **Overlay scope** reads the WORKING tree: file set = `known` (index) minus `deleted`, plus `untracked`; every path in `deleted` becomes `FileEntry::tombstone()` = `{ content_hash: String::new(), nodes: vec![], edges: vec![], coverage: FileCoverage::Deleted }`.
- **Reuse without reading.** For each path, look up the previous layer (overlay, then base seed) entry AND its `blob_index[path]`. When the current oid (from `known`) equals the recorded oid, the path is not dirty, and `parser_version_matches`, reuse the entry without reading the file. Otherwise read the bytes, hash them, and reuse by hash-equality as today, else extract. Untracked files are always read and hashed (no oid); record them in `blob_index` only when git reports an oid.
- **Base seeding.** `resolve_scope_layers` for `Base { revision }`: `previous` = the `_local` overlay if present, else the newest existing base file (`prune.rs` already lists bases by mtime — reuse its listing helper), else `None`. A documentation-only commit then reuses every source entry by oid.
- **Overlay persistence.** Keep entries that differ from the base, INCLUDING tombstones for paths the base has; drop tombstones for paths the base lacks. Set `layer.generation = working_tree.generation` (overlay) or `String::new()` (base). Fill `blob_index` for every path with a known oid.
- **Resolution.** `GraphStore::resolved`: an overlay entry with `coverage == FileCoverage::Deleted` REMOVES the path from `files` and does not enter `overlaid`.

### Counters and outcome

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceGraphCounters {
    pub files_enumerated: usize, pub files_hashed: usize, pub files_parsed: usize,
    pub files_reused: usize, pub files_deleted: usize, pub files_untracked: usize,
    pub bytes_serialized: u64,
    pub enumerate_ms: u64, pub hash_ms: u64, pub parse_ms: u64, pub persist_ms: u64,
}
pub struct SourceGraphOutcome { pub nodes: usize, pub edges: usize, pub freshness: Freshness, pub counters: SourceGraphCounters }
```

`files_extracted` is removed (it lied). `SemanticOutcome` carries `counters` the same way; `bytes_serialized` is the JSON length actually written (0 when the write was skipped as unchanged).

### Semantic policy (`semantic.rs`)

`try_reconcile_semantic`: compute `working_tree`; ensure a base exists for `head` (publish from committed content if missing); when `generation != clean_generation(head)`, also reconcile the `_local` overlay. `SemanticLayer` becomes `Base { revision }` | `BaseAndLocalOverlay { revision, plan, stage }` | `Skipped { reason }` (the `LocalOverlay { refusal }` variant is deleted — nothing is refused any more). B2 will later route this through `ensure_snapshot`; keep the function shape so that is a body swap.

### Freshness (F2) — `refresh/graph.rs` and `refresh.rs`

- `refresh.rs::evaluate` keeps the HEAD comparison but computes fingerprints ONCE: introduce a private `Evaluated { state, fingerprints }` returned by an inner function; `refresh` hands the fingerprints to `rebuild_and_persist` instead of re-walking. Public `evaluate` keeps its signature.
- `retrieve/graph.rs::load_resolved_graph` returns a `GraphLoad { graph: Option<ResolvedGraph>, degraded: Option<String>, working_tree_stale: Option<String> }`. `working_tree_stale` is `Some(detail)` when the overlay the query resolves to exists and its `generation != working_tree.generation`, or when no overlay exists and the tree is dirty (`generation != clean_generation(head)`). `retrieve_for_stage` copies it onto `pack.semantic_freshness` (`stale = true`, `detail`). Retrieval still never writes a layer.

## Steps

1. `enumerate.rs`, `generation.rs`: implement and unit-test against a temp repo (`init_repo` / `git_ok` fixtures in `tests_source_graph.rs` are reusable; keep the `GIT_CONFIG_*` neutralisation).
2. `layer.rs`: rewrite `build_layer` per the contract with counters and phase timings; tombstones; committed reads. Move `unreadable_entry`, `parser_version_matches` with it.
3. `source_graph.rs`: `reconcile_source_graph` orchestrates enumerate → build → persist → freshness; delete `dirty_tree_reason`; `degraded_outcome` keeps zero counters.
4. `graph_store/mod.rs`: tombstone-aware `resolved`; `persist` unchanged otherwise; `FileEntry::tombstone()`.
5. `semantic.rs`: the new policy and enum; `refresh.rs`: single fingerprint pass.
6. `retrieve/graph.rs` + `retrieve.rs`: `GraphLoad` and the stale override.
7. Tests (names are what the plan's acceptance filters on):
   - `tests_source_graph.rs`: `an_edit_at_unchanged_head_changes_the_generation_and_marks_the_overlay_stale`; `a_deleted_tracked_file_is_tombstoned_and_absent_from_the_resolved_graph`; `a_renamed_file_appears_under_its_new_path_only`; `an_untracked_source_file_is_indexed_in_the_overlay_and_absent_from_the_base`; `a_base_publishes_from_committed_content_on_a_dirty_tree` (dirty `src.rs`: the base holds the committed symbol, the local overlay the edited one — the plan's "Working-tree snapshot" and "Repeated maintenance" rows); `a_documentation_only_commit_reuses_every_source_entry_without_reading_it` (`files_parsed == 0`, `files_hashed == 0`, `files_reused == N`); `a_source_commit_parses_exactly_the_changed_file`; `an_extractor_version_change_reparses_everything_at_the_same_head`; `counters_distinguish_parsed_from_reused`; `an_unchanged_overlay_rerun_writes_no_bytes`.
   - `graph_store/tests.rs`: `a_tombstone_in_the_overlay_hides_the_base_entry`; `a_tombstone_for_a_path_the_base_lacks_is_dropped_at_persist`.
   - `tests_freshness.rs`: `refresh_fingerprints_the_tree_once_per_call`.
   - `tests/retrieve_source.rs`: `retrieval_reports_the_working_tree_stale_when_the_overlay_generation_moved`.
   - Update every test that asserted the old refusal (`base_scope_refuses_to_publish_when_the_tracked_tree_is_dirty`, `test_dirty_tree_falls_back_to_local_overlay`, `test_dirty_tree_overlay_is_readable_through_local_scope`) to the new policy rather than deleting coverage.

## Done means

`cargo build --manifest-path loom/Cargo.toml` warning-free and `cargo test --lib context::` green. `rg -n "files_extracted|dirty_tree_reason|LocalOverlay \{" loom/src` returns nothing. The compile-fix files listed above keep their current behavior (the preflight still prints its two message shapes; `sync` prints the new layer variant; `merge_lifecycle` logs the counters) — B2 replaces their logic in the next wave.

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib context::refresh` — skip if unsure.

## Constraints and traps

- `context` may import `crate::git::runner::run_git_checked` and nothing else from `git`; no upward imports into `commands`/`orchestrator`.
- Renames in `git status -z` occupy TWO NUL fields for `R`/`C` codes; parse them or the path list shifts by one.
- `body_hash` is the one content identity for the lexical index key; never put a git oid into `content_hash`.
- The lexical index key hashes the RESOLVED graph, which now excludes tombstoned paths — verify `lexical_index::cache::source_layer_key` needs no change (it should not).
- Never publish untracked or uncommitted bytes into a base. The base for HEAD must equal what a clean checkout of HEAD would produce; `a_base_publishes_from_committed_content_on_a_dirty_tree` is the contract.
- No git commands of your own (the ones your CODE runs are the feature); do not touch `commands/**`.

## Report back

Files changed and created; the final `SourceGraphOutcome` / `SemanticOutcome` / `SemanticLayer` definitions verbatim; the exact edits made to the compile-fix files (B2 reads your report before touching them); anything unresolved.
