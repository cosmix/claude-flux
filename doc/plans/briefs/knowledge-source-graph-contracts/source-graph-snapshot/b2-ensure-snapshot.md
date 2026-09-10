# B2 — One snapshot-ensuring decision path for sync, init/run, map, and the hook; standalone `loom map`; identical-index skip; reuse regression suite

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `source-graph-snapshot` · Wave 1 (parallel with B3; after B1).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F14, F18, standalone operation)

Four entry points each decide on their own whether the graph is current, and none can say "reused": `commands/run/checks.rs::publish_source_graph` checks `load_base(HEAD)` only, then re-scans the `_local` overlay in full on a dirty tree; `commands/map.rs::load_graph` reconciles the `_local` overlay unconditionally on every invocation; `commands/knowledge/sync.rs` reaches `semantic::try_reconcile_semantic`, which rebuilds even a current overlay; `commands/hook/reconcile_graph.rs`'s checkout arm goes through the same. `loom map` also refuses to run without an initialised `.loom/work` (`work_dir.load()`), and `fs/knowledge/index.rs::write_index` rewrites `INDEX.md` byte-for-byte identical, which used to dirty the tree.

B1 (merged before you) made a base publishable from committed content, gave every layer a `generation` and `blob_index`, and returns `SourceGraphCounters`. Read B1's report and the files it changed directly — codex's `loom map` will not show them.

## Files you own (write)

- `loom/src/context/refresh/snapshot.rs` — NEW; declare it from `refresh.rs` (`pub mod snapshot;`) and re-export `ensure_snapshot`, `SnapshotPolicy`, `SnapshotAction`, `SnapshotOutcome`
- `loom/src/context/refresh/semantic.rs` — `try_reconcile_semantic` becomes a thin call into `ensure_snapshot(LocalCurrent)`
- `loom/src/context/refresh/tests_snapshot.rs` — NEW
- `loom/src/commands/run/checks.rs`, `loom/src/commands/run/tests.rs`
- `loom/src/commands/init/execute.rs`, `loom/src/commands/run/mod.rs`, `loom/src/commands/run/foreground.rs` — ONLY the `advisory_source_graph_preflight(...)` call lines
- `loom/src/commands/map.rs` — `execute`, `run_views`, `load_graph`, and `MapArgs`
- `loom/src/commands/knowledge/sync.rs`, `tests_sync.rs`
- `loom/src/fs/knowledge/index.rs` (`write_index` only) and `loom/src/fs/knowledge/tests/index.rs` (or wherever `write_index` is tested — grep `write_index` under `fs/knowledge/tests`)
- `loom/src/orchestrator/merge_lifecycle.rs` — `reconcile` logs counters; no policy change

Read-only: everything B1 owns except `semantic.rs`; `map/views/**` and `resolve/**` (B3 is editing them IN PARALLEL — you call B3's functions by the exact signatures below and never edit those files); `commands/hook/**`.

## Contract

### `refresh/snapshot.rs`

```rust
pub enum SnapshotPolicy {
    /// Base for HEAD present, plus the `_local` overlay current when the tree is dirty.
    LocalCurrent,
    /// The named stage overlay current (a stage worktree; no base publish).
    StageOverlay { plan: String, stage: String },
    /// Base for HEAD present; the working tree is not consulted beyond HEAD.
    BaseOnly,
}
pub enum SnapshotAction { Reused, Updated, Rebuilt, Unavailable }
pub struct SnapshotOutcome {
    pub action: SnapshotAction,
    pub reason: String,            // one sentence, e.g. "base for abc12345 present; tree clean"
    pub revision: String,          // HEAD
    pub generation: String,        // working-tree generation observed
    pub overlay: Option<(String, String)>,
    pub counters: SourceGraphCounters,   // zero on Reused
    pub elapsed: std::time::Duration,
}
pub fn ensure_snapshot(store: &ContextStore, graph_store: &GraphStore, project_root: &Path, policy: SnapshotPolicy) -> Result<SnapshotOutcome>
```

Decision:

1. `working_tree(project_root)` (B1's probe: one `git status`, hashing dirty files only).
2. `BaseOnly`: base for HEAD exists → `Reused`; else publish (B1's `reconcile_source_graph(Base)`) → `Rebuilt` when no previous layer seeded reuse, `Updated` otherwise (`counters.files_reused > 0`).
3. `LocalCurrent`: ensure the base as in 2; then if `generation != clean_generation(head)`: load the `_local` overlay — `generation` equal → nothing; else reconcile the overlay. `Reused` only when nothing was written.
4. `StageOverlay`: overlay exists with equal `generation` → `Reused`; else reconcile it.
5. Any git failure (not a repository, no HEAD) → `Unavailable` with the reason; never an `Err` for that case.

`SnapshotOutcome` renders through one helper `pub fn describe(&self) -> String`: `source graph: reused base abc12345 (tree clean)` / `source graph: updated local overlay _local/map-loom (3 parsed, 1571 reused, 2 deleted; 0.41s)` / `source graph: rebuilt base abc12345 (1575 parsed; 12.3s)` / `source graph: unavailable (<reason>)`. Counters and elapsed appear on every non-`Reused` line.

### Callers

- `checks.rs`: `advisory_source_graph_preflight(repo_root: &Path, work_dir: &WorkDir)` (the `allow_overlay_fallback` parameter is deleted — a base is always publishable now). It calls `ensure_snapshot(BaseOnly)` and prints `describe()` on `Updated`/`Rebuilt`/`Unavailable`; silent on `Reused`. Update the three call sites. Keep the `preflight_is_called_before_the_plan_rename_in_both_run_paths` test working (the ordering still matters for the plan-file rename? No — a dirty tree no longer refuses; keep the test anyway, and update its doc comment to say why the ordering is now merely tidy).
- `map.rs`: replace `work_dir.load()?` with `WorkDir::new(".")?` plus `store.ensure()` (overlay directories are created lazily by `write_layer`), so `loom map` works in a checkout that never ran `loom init`. `load_graph` calls `ensure_snapshot(LocalCurrent)` and prints `describe()` to stderr only when the action is not `Reused`. Then `graph_store.resolved(...)` and `resolve_graph` as today. The coverage/resolution footer is printed ONCE per invocation after all views, through B3's `render_footer` (below).
- `MapArgs` gains: `--callers <SYMBOL>`, `--callees <SYMBOL>`, `--depth <N>` (default 3), `--kinds <LIST>` (comma-separated edge kinds; default all), `--limit <N>` (default 50 for impact/callers/callees; 0 = unlimited), `--path <PREFIX>` (only hits under this project-relative prefix), `--min-confidence <F>` (default 0.0), `--json`. Extend the "needs a view flag" check to the two new views.
- `sync.rs`: prints `describe()` from the `SemanticOutcome` (B1 put counters there); JSON payload gains `action`, `parsed`, `reused`, `deleted`, `elapsed_ms`; delete the `LocalOverlay` refusal line.
- `index.rs::write_index`: read the existing file; when its bytes equal the generated content, return `Ok(())` without locking or writing (test: mtime unchanged).
- `merge_lifecycle.rs::reconcile`: keep calling `reconcile_source_graph` for stage overlays (the daemon owns those), log the counters.

### B3's functions you call (EXACT signatures — B3 implements them in parallel; do not edit `map/views/**`)

```rust
// loom/src/map/views/mod.rs
pub struct ImpactArgs { pub depth: usize, pub kinds: Vec<SourceEdgeKind>, pub limit: usize, pub path_prefix: Option<String>, pub min_confidence: f32 }
pub fn render_outline(graph: &ResolvedGraph, project_root: &Path, arg: &str) -> String            // unchanged
pub fn render_find_all(graph: &ResolvedGraph, symbol: &str) -> String                               // unchanged
pub fn render_impact(graph: &ResolvedGraph, project_root: &Path, arg: &str, stats: &ResolutionStats, args: &ImpactArgs) -> String
pub fn render_callers(graph: &ResolvedGraph, project_root: &Path, arg: &str, limit: usize) -> String
pub fn render_callees(graph: &ResolvedGraph, project_root: &Path, arg: &str, limit: usize) -> String
pub fn render_footer(graph: &ResolvedGraph, stats: &ResolutionStats) -> String
// loom/src/map/views/json.rs
pub fn outline_json(graph: &ResolvedGraph, project_root: &Path, arg: &str) -> serde_json::Value
pub fn find_all_json(graph: &ResolvedGraph, symbol: &str) -> serde_json::Value
pub fn impact_json(graph: &ResolvedGraph, project_root: &Path, arg: &str, stats: &ResolutionStats, args: &ImpactArgs) -> serde_json::Value
pub fn callers_json(graph: &ResolvedGraph, project_root: &Path, arg: &str, limit: usize) -> serde_json::Value
pub fn callees_json(graph: &ResolvedGraph, project_root: &Path, arg: &str, limit: usize) -> serde_json::Value
pub fn footer_json(graph: &ResolvedGraph, stats: &ResolutionStats) -> serde_json::Value
```

`--json` prints ONE object: `{"views": {"outline": ..., "find_all": ..., "impact": ..., "callers": ..., "callees": ...}, "coverage": <footer_json>}` with only the requested views present. Parse `--kinds` with `SourceEdgeKind::as_str()` names; an unknown kind is a clap error naming the valid set.

## Steps

1. `snapshot.rs` + `tests_snapshot.rs` (temp repo through the `init_repo`/`git_ok`/`stores` fixtures of `refresh/tests_source_graph.rs` — import them or copy the three helpers; do not edit that file).
2. `semantic.rs`: body swap to `ensure_snapshot(LocalCurrent)`; map the outcome to `SemanticOutcome`.
3. `checks.rs` and the three call sites; `run/tests.rs` regression cases (reuse `init_preflight_repo` + `run_git`):
   - `sync_then_init_on_an_unchanged_clean_tree_reuses_without_reading` (`counters.files_hashed == 0`, action `Reused`)
   - `sync_then_init_on_an_unchanged_dirty_tree_reuses_the_current_overlay`
   - `a_documentation_only_commit_publishes_a_new_base_reusing_every_source_entry` (`files_parsed == 0`)
   - `a_source_commit_parses_only_the_changed_file`
   - `explicit_cleanup_keeps_the_base_and_reuses_it` (call `cleanup_work_directory`, then preflight → `Reused`)
   - `an_extractor_change_at_the_same_head_reparses` (simulate by rewriting the stored base's `parser_version` on every node)
   - `a_dirty_checkout_yields_a_base_with_the_committed_symbol_and_a_local_overlay_with_the_edit`
4. `map.rs`: standalone execution, `ensure_snapshot`, the new flags, single footer, `--json`. Add `tests_map.rs` next to `map.rs` (declare with `#[path]` the way sibling command modules do) covering flag parsing (`--kinds` validation) and the "no view flag" error — the graph itself is covered by B3.
5. `sync.rs` + `tests_sync.rs`; `index.rs` + its test `write_index_skips_an_identical_file`.

## Done means

- `cargo build --manifest-path loom/Cargo.toml` warning-free; `cargo test --lib commands::run::`, `commands::knowledge::tests_sync`, `commands::map`, `context::refresh::` green.
- `loom/target/debug/loom map --help` lists `--callers`, `--callees`, `--depth`, `--kinds`, `--limit`, `--path`, `--min-confidence`, `--json`.
- In a fresh temp checkout with no `.loom/work`, `loom/target/debug/loom map --outline <file>` answers (the orchestrator runs this smoke; write it as `tests_snapshot::map_answers_in_a_checkout_that_never_ran_init` if you can drive `commands::map::execute` from a test — it changes cwd, so mark it `#[serial]`).

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib commands::run::tests` — skip if unsure.

## Constraints and traps

- `loom map` output on `Reused` prints NOTHING extra; the whole point is that a one-symbol query costs a `git status`, not a scan.
- `ensure_snapshot` is the only place that decides between reuse and rebuild. Do not leave a second decision in `checks.rs` or `map.rs`.
- The stage overlay path stays with `merge_lifecycle`/`reconcile_graph` (daemon-owned) — `StageOverlay` exists for them and for the hook's stage arm, which `delivery-and-catalog` wires later; do not edit `commands/hook/**`.
- Keep `checks.rs` under 400 lines (it is ~394 today): move `announce_source_graph_build`/`report_source_graph_build_finished` into `run/checks/graph_messages.rs` if you need room.
- B3's signatures above are a contract; if one cannot be honored, report it instead of editing `map/views/**`.
- No git commands of your own.

## Report back

Files changed and created; the final `SnapshotOutcome` definition; the exact `describe()` strings; any B3 signature you could not call as written; anything unresolved.
