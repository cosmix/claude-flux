---
---
# Concurrency And Locking

> Locked-handle writes and read-mutate-save races that lose concurrent updates.

## File Locking: Writing to Locked Handles

**Mistake:** `fs::write()` opens a NEW handle that ignores locks held by other handles.
**Fix:** Write to the locked handle: `file.set_len(0)`, `file.seek(Start(0))`, `file.write_all()`.

## Stage-File Lost Updates: Whole-Stage Save Reverts Concurrent Writers (A-5, 2026-06-09)

**What happened:** `locked_read`/`locked_write` make individual reads/writes atomic, but load → mutate → whole-record save releases the lock between read and write. Three writer classes race on the same stage file — the orchestrator main loop, daemon IPC handlers, and agent-run CLI commands. A writer that loaded a stage minutes earlier can therefore revert status, counters, close reason, session identity, or amended verification policy written in the gap.

**Misleading signal:** patterns.md claimed a "daemon single-writer model" and "no explicit file locking." Both were false — `fs/locking.rs` exists, and the daemon, its dispute thread, and CLI agents all write stage files concurrently. "Each save is locked" hid that locked atomic saves of a STALE whole object still lose updates.

**Why:** per-operation locking serializes the WRITE but not the read→write transaction. Two transactions that both `load(); mutate_field_A_or_B(); save_whole_stage()` interleave as load-A, load-B, save-A, save-B → B's save reverts A's field.

**Prevention — use `verify::transitions::update_stage(id, work_dir, |s| …)` for every existing-stage mutation, never load + mutate + whole-record save.** It re-reads under the stages-directory lock, applies the closure, and writes in one critical section. Mutate only operation-owned fields. Run slow Git, terminal, network, and verification work outside the lock, then apply a short delta. Whole-record persistence is for actual creation only; the orchestrator loop is not exempt because daemon and CLI writers remain concurrent.

**Detection rule:** any `load_stage(); …; save_stage()` pair where the `…` can run while the daemon/dispute-thread/another-CLI is live is a lost-update candidate. Especially when `…` contains a multi-minute step (acceptance, verification, git merge) or increments a counter read from the in-memory stage (`fix_attempts += 1`, `dispute_count += 1`).
