# Web Dashboard Latent Issues

> Issues reviewed in `loom/src/commands/status/web/` during integration-verify and
> deliberately left unchanged — recorded so a later reader does not mistake them for
> oversights. See [architecture/web-dashboard.md](../architecture/web-dashboard.md) for
> context.

1. **Mutex poisoning cascade.** `loom/src/commands/status/web/broadcast.rs` calls
   `.expect()` on its lock in several places; a panic inside any of those critical sections
   poisons the mutex, after which every connection thread and the producer thread panic on
   the next frame while the accept loop keeps spawning threads that panic immediately.
   Near-unreachable today — those sections only do push/`try_send`/clone.
2. **`GET /ws` without an `Upgrade` header returns the SPA page with 200, not 400** —
   `loom/src/commands/status/web/connection.rs` falls through to `Route::Spa`. Cosmetic.
3. **Inherited partial-frame truncation risk.** The broadcaster's 50ms read timeout over
   `daemon/wire.rs`'s `read_exact`-based framing can truncate a large `StatusData` body
   mid-read; it self-heals via reconnect. The TUI has the identical timeout and the same
   risk, so this is inherited behaviour, not new — a real fix means owning `daemon/wire.rs`,
   out of scope for this plan.
4. **524 kB `index.js` chunk-size warning left in place.** `bun run build` warns that the
   bundle exceeds rollup's 500 kB default. Code-splitting conflicts with the single-bundle
   design `loom/build.rs` embeds via `include_bytes!` (and the acceptance criterion pinning
   `index.js` as one file); raising the warning limit would be suppression. Left for whoever
   owns the bundle's size budget.

## Resolved

`DEFAULT_PORT` is now the production starting point for bare `loom status --web`. Binding
advances from 7373 only when a candidate is already in use; explicit ports remain exact and
explicit `0` delegates selection to the OS.
