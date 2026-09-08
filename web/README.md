# loom web dashboard

The browser view behind `loom status --web`: the same ledger `loom status
--live`
draws in a terminal, re-rendered as a page that stays open on a second monitor.
The server in `loom/src/commands/status/web/` serves the built bundle and
streams
snapshots over a WebSocket; this directory is the React client.

## Develop

```bash
loom status --web      # in a loom workspace: starts the server on port 7373
cd web && bun run dev  # Vite dev server; /api and /ws proxy to 127.0.0.1:7373
```

`bun run check` runs the type-check, lint, format check, and tests.

## Build

```bash
cd web && bun run build
```

`dist/` is committed and embedded into the `loom` binary by `loom/build.rs`.
Rebuild and commit `dist/` with every change under `src/`.

## Views

`/` draws the plan as a graph: stages laid out left to right by dependency
(dagre), joined by threads whose colour and dash follow the state of the stage
they leave. Click a stage to trace its thread, double-click (or press Enter, or
use the corner button) to open it in a dialog; `?stage=<id>` on any route opens
the same dialog. `/ledger` is the TUI's table.

The terminal view requires `loom status --web --terminals` and a tmux-backed
session. It supports read-only viewing and interactive control; its `@xterm/*`
emulator chunk is lazy-loaded only when a terminal is opened.

## Styling

The palette is aurora-ui's OKLCH token sheet, vendored under `src/aurora-ui/`
with
the components the page uses (busy roundel, hazard panels, empty state, error
boundary, theme toggle); see `src/aurora-ui/README.md`. Dark mode is the `.dark`
class the theme toggle sets, persisted under `loom:theme`, initialised from the
OS
preference. Loom's own state tones (`--tone-*`) sit on top in `src/index.css`.

The busy roundel is the activity instrument: one in the header turns while any
session is working, and each stage with a live session carries its own in the
ledger's activity column, on its card, and in its dialog.

## Layout

```text
src/main.tsx          mounts the router inside the Jotai store and opens the
socket
src/router.tsx        routes: `/` (graph), `/ledger`, `/stages/:stageId`
(detail)
src/routes/           shell (header, footer, dialogs), overview, ledger, stage,
error
src/components/       header, ledger table, panels, badges, logo, stage dialog
src/components/graph/ the stage graph: canvas, card, thread edge, key
src/lib/graph.ts      dagre layout, thread styling, lineage
src/aurora-ui/        vendored aurora-ui pieces: tokens, theme, roundel, hazard
panels, empty state
src/components/ui/    shadcn primitives
src/state/            store, atoms, snapshot application
src/lib/format.ts     display semantics shared with the TUI
src/api/              wire schema, socket, fixtures
```

Stage states and their glyphs come from `src/api/fixtures/statuses.json`, which
a
Rust test pins against the `StageStatus` enum; the legend and the state badge
read
that table rather than restating it.
