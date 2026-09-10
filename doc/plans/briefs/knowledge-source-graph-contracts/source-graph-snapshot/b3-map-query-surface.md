# B3 — Bounded impact, direct callers/callees, one footer, JSON views

Plan: `doc/plans/PLAN-knowledge-source-graph-contracts.md` · Stage: `source-graph-snapshot` · Wave 1 (parallel with B2; after B1).
Lane: codex `gpt-5.6-sol`, effort `xhigh`. Paths are repo-root relative; the crate is `loom/`.

## Why (F13, navigation half)

`context/resolve/impact.rs::reverse_adjacency` reverses EVERY resolved edge kind — `Contains` and `Imports` included — so `loom map --impact retrieve_for_stage` returns file-containment nodes and imports alongside real dependents (51 lines, 5.2 KB for one symbol). `map/views/mod.rs::render_impact_for` uses the fixed `IMPACT_MAX_DEPTH = 3` and prints every hit with no cap, no kind filter, no path filter; `MapArgs` has no depth/limit/filter flags. Every view also appends the global coverage footer, so three views print it three times. There is no direct callers/callees view and no machine-readable output.

B1 (merged) removed tombstoned files from `ResolvedGraph`; nothing else in the graph types changed. B2 is editing `commands/map.rs` IN PARALLEL and will call the functions below by these exact signatures — implement them exactly.

## Files you own (write)

- `loom/src/context/resolve/impact.rs`, `loom/src/context/resolve/tests_impact.rs`
- `loom/src/context/resolve/neighbors.rs` — NEW (direct callers/callees); declare it in `resolve/mod.rs` and re-export the two functions + `Neighbor`
- `loom/src/map/views/mod.rs`, `loom/src/map/views/json.rs` — NEW, `loom/src/map/views/tests.rs`, `loom/src/map/mod.rs` (module declaration only)

Read-only: `commands/map.rs` (B2), `context/graph_store/mod.rs` (`ResolvedGraph::{edges, node, files}`), `context/source_graph/{edge,node}.rs`, `context/resolve/mod.rs` beyond the declaration, `context/coverage.rs` (`CoverageReport::of`), `context/untrusted.rs` (`inline_safe`).

## Contract

### `resolve/impact.rs`

```rust
pub struct ImpactOptions {
    pub max_depth: usize,                 // 0 → empty result (existing semantics)
    pub kinds: Vec<SourceEdgeKind>,       // empty → every kind EXCEPT Contains and Imports
    pub limit: usize,                     // 0 → unlimited; else at most this many hits, in the existing sort order
    pub path_prefix: Option<String>,      // keep only hits whose `path` starts with it (project-relative, forward slashes)
    pub min_confidence: f32,              // drop hits whose `min_confidence` is below it
}
impl Default for ImpactOptions { /* depth 3, kinds empty, limit 0, no prefix, 0.0 */ }
pub struct ImpactResult { pub hits: Vec<ImpactHit>, pub suppressed: usize /* hits dropped by limit only */ }
pub fn impact_with(graph: &ResolvedGraph, start_id: &str, options: &ImpactOptions) -> ImpactResult
pub fn impact(graph: &ResolvedGraph, start_id: &str, max_depth: usize) -> Vec<ImpactHit>   // KEEP: now `impact_with` with kinds = ALL kinds (Contains/Imports included) so existing callers and tests keep their meaning
```

Kind and confidence filters apply DURING traversal (an edge that fails the filter is not followed); `path_prefix` and `limit` apply to the result. Sort order unchanged (depth, confidence desc, id).

### `resolve/neighbors.rs`

```rust
pub struct Neighbor { pub id: String, pub kind: SourceNodeKind, pub path: String, pub edge_kind: SourceEdgeKind, pub confidence: f32, pub provenance: EdgeProvenance, pub line_start: Option<usize> }
/// Symbol nodes with a resolved Calls/References/Implements/Extends edge INTO `node_id`.
pub fn direct_callers(graph: &ResolvedGraph, node_id: &str, limit: usize) -> (Vec<Neighbor>, usize)
/// Symbol nodes such an edge OUT OF `node_id` reaches.
pub fn direct_callees(graph: &ResolvedGraph, node_id: &str, limit: usize) -> (Vec<Neighbor>, usize)
```

Unresolved edges and `File` endpoints are excluded; ordered by `(confidence desc, id asc)`; the second tuple element is the count suppressed by `limit` (0 = unlimited).

### `map/views/mod.rs`

```rust
pub struct ImpactArgs { pub depth: usize, pub kinds: Vec<SourceEdgeKind>, pub limit: usize, pub path_prefix: Option<String>, pub min_confidence: f32 }
pub fn render_outline(graph: &ResolvedGraph, project_root: &Path, arg: &str) -> String            // unchanged behavior, MINUS the footer
pub fn render_find_all(graph: &ResolvedGraph, symbol: &str) -> String                               // unchanged, MINUS the footer
pub fn render_impact(graph: &ResolvedGraph, project_root: &Path, arg: &str, stats: &ResolutionStats, args: &ImpactArgs) -> String
pub fn render_callers(graph: &ResolvedGraph, project_root: &Path, arg: &str, limit: usize) -> String
pub fn render_callees(graph: &ResolvedGraph, project_root: &Path, arg: &str, limit: usize) -> String
pub fn render_footer(graph: &ResolvedGraph, stats: &ResolutionStats) -> String   // the coverage line + the resolution line, once
```

- Delete the printing wrappers `outline`, `find_all`, `impact` (B2 prints). Keep `render_outline`/`render_find_all` output byte-identical except the removed footer line(s) — `views/tests.rs` pins the rest.
- `render_impact` heading becomes `Impact of <id> (depth <= N, kinds: calls,references,... | all, reverse edges)`; rows unchanged; after the rows print `... K more suppressed (raise --limit)` when `suppressed > 0`; the ambiguous-start cap `IMPACT_MAX_STARTS` stays.
- `render_callers`/`render_callees`: heading `Callers of <id>` / `Callees of <id>`, one row per neighbor: `{:.2}  {provenance}  {edge_kind}  {id}  ({path}:{line_start})`, then the suppression line when applicable; start resolution reuses `find_symbol_matches` + `IMPACT_MAX_STARTS` exactly as `render_impact` does.
- Every untrusted value (ids, paths, details) still goes through `inline_safe`.

### `map/views/json.rs`

One function per view returning `serde_json::Value`, with these shapes:

- `outline_json` → `{"path", "coverage": {"status", "detail"?}, "symbols": [{"id","kind","scope","line_start","line_end","signature"}]}` (or `{"path","error":"no indexed file"}`).
- `find_all_json` → `{"symbol","exact": bool, "matches": [{"id","kind","path","line_start","coverage_status"}], "suppressed": N}`.
- `impact_json` → `{"start_ids": [...], "depth", "kinds": [...], "hits": [{"id","kind","path","depth","min_confidence","weakest_provenance","weakest_kind"}], "suppressed": N}`.
- `callers_json` / `callees_json` → `{"start_ids": [...], "neighbors": [Neighbor fields], "suppressed": N}`.
- `footer_json` → the `CoverageReport` fields plus `{"resolution": {"retargeted","ambiguous","unresolved"}}`.

Signatures exactly as listed in B2's brief (copied here): `outline_json(graph, project_root, arg)`, `find_all_json(graph, symbol)`, `impact_json(graph, project_root, arg, stats, args)`, `callers_json(graph, project_root, arg, limit)`, `callees_json(graph, project_root, arg, limit)`, `footer_json(graph, stats)`.

## Steps

1. `impact.rs`: `ImpactOptions`, `impact_with`, keep `impact` as the all-kinds wrapper. Tests in `tests_impact.rs`: `default_options_skip_contains_and_imports_edges`; `a_kind_filter_is_applied_while_traversing_not_after`; `a_limit_reports_how_many_hits_it_suppressed`; `a_path_prefix_keeps_only_matching_hits`; `a_min_confidence_stops_traversal_through_weak_edges`; existing tests unchanged.
2. `neighbors.rs` + tests (`tests_neighbors.rs` under `resolve/`): callers vs callees direction; unresolved and File excluded; ordering; limit.
3. `views/mod.rs`: the renderers; move the footer out of every view into `render_footer`; `views/tests.rs`: update the three tests that asserted a footer per view, add `impact_heading_names_the_kind_filter`, `callers_view_lists_direct_callers_with_provenance`, `render_footer_prints_coverage_once`.
4. `views/json.rs` + tests: every shape above round-trips through `serde_json` and contains the documented keys (assert key sets, not full text).

## Done means

`cargo test --manifest-path loom/Cargo.toml --lib map::` and `--lib context::resolve` green; `map/views/mod.rs` under 400 lines (move callers/callees rendering to `views/neighbors.rs` if needed — declare it in `views/mod.rs` and keep the public functions where the contract says).

## Proof (ONE narrow check, run once)

`cargo test --manifest-path loom/Cargo.toml --lib map::views` — skip if unsure.

## Constraints and traps

- `commands/map.rs` will not compile against your changes until B2 lands its half; that is expected — build and test your modules with `--lib map::views` and `--lib context::resolve` filters, and report any signature you had to deviate from (the orchestrator reconciles).
- `impact()`'s old semantics (all kinds) must be preserved for `tests_impact.rs`'s existing cases and any other caller (`rg -n "impact\(" loom/src` — check `rank_source` and hook code before changing it).
- `views/tests.rs` uses `#[serial]` for the three `project_relative` tests (they touch cwd); keep that.
- No git commands.

## Report back

Files changed and created; any deviation from the signatures above; the final heading and row formats; anything unresolved.
