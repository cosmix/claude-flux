---
---
# Writer/Reader Address

> A layer written under a key its reader ignores looks identical to no-op.

## A Fallback That Writes Under a Key No Reader Consults

**What happened:** the working-tree source-graph overlay is addressed by a
`(plan, stage)` pair, and the two sides derived that pair differently. The producer
keyed the stage component on `project_root.file_name()`
(`context/local_overlay.rs:20`), while `WorkDir::project_root()` returns `.` at the
repo root and an ABSOLUTE path from any subdirectory — so ONE tree had TWO
addresses, `_local/map` and `_local/map-<dirname>`. Proven in a fresh clone with no
base layer: `loom knowledge sync` at the repo root wrote `_local/map`, then
`cd loom && loom knowledge context --scope source --json` returned ZERO source-node
items, because it read `_local/map-<dirname>`. It also built two ~16 MB overlays for
one tree.

**Why it is invisible:** `GraphStore::resolved` returns the base layer with NO error
when the overlay key is absent or `None` (`graph_store/mod.rs:251-271`), so a
mismatch degrades to "the last merged revision" instead of failing. The write
succeeded, the read succeeded, the command exited 0, every gate stayed green. A
fallback that WRITES a derived layer under a key no reader consults is
indistinguishable, from outside, from doing nothing at all — and it spends disk and
wall-clock producing the thing nobody reads.

**Prevention:**

1. When a layer is addressed by a composite key, that key belongs to exactly ONE
   shared definition. Here it is `context/local_overlay.rs` — `LOCAL_PLAN_KEY`,
   `local_overlay_stage_name`, `local_overlay_key`, `OverlayScope::resolve`. Two
   derivations that "should agree" is the same bug in a different costume; the
   earlier instance in this codebase is `delivery::plan_key`, where a hand-rolled
   second derivation read an empty directory rather than a missing record (see
   *Delivery Records* in [Context Retrieval](../architecture/context-retrieval.md)).
2. An address derived from a path must be derived from the CANONICAL path.
   `file_name()` on a caller-supplied spelling is an identity bug, not a naming
   choice — and a comment asserting "every caller spells it the same way" is the
   smell, never the proof.
3. **The test that matters is the round trip:** write through the producer's
   address, read through the CONSUMER's, assert the same bytes come back. A test
   that asserts the producer wrote something, or that the reader reads back what the
   test itself planted, proves nothing about the two agreeing.

**Fix:** canonicalize `project_root` before taking `file_name()`, keeping the
un-canonicalized path as the fallback for a path that does not exist yet.

## The Same Shape One Layer Up: The Stage Overlay Key

`orchestrator/signals/retrieval.rs` builds a stage brief's overlay scope with
`stage_overlay_scope(stage)`, whose plan component MUST equal
`delivery::plan_key(stage)` — the address `MergeLifecycle::reconcile_overlay` writes
to. The agreement is spelled out inline at the call site rather than merely
imported, precisely because a mismatch is silent by construction: the brief degrades
to the last merged revision and nothing anywhere reports it.

`OverlayScope::Local` is the WRONG default here and that is worth knowing before you
"simplify" it: the daemon runs from the main repo, so `Local` would resolve against
the main checkout's directory name rather than the stage's worktree.

Residue, not yet fixed: a blank `plan_id` in `.work/config.toml` and a stage record
carrying no plan both resolve to `"default"` through `plan_key`, but the writer side
does not normalize the same way.

## Why This Class Keeps Recurring Here

Silent degradation is the house style in `context/`, on purpose: `load_resolved_graph`
returns `None` on a missing base, a missing overlay or any IO error rather than
failing retrieval. That is right — a cold cache must not break a spawn. The cost is
that EVERY addressing bug in this subsystem presents as "slightly thinner output",
never as an error. In a subsystem that degrades silently by design, the round-trip
test is not a nicety; it is the only detector you have.
