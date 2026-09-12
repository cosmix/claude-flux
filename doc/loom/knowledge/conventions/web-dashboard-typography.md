# Web Dashboard Typography

> Dashboard chrome type conventions and CSS gotchas for settings/graph views

## Dashboard chrome uses the body face, never all-caps monospace (2026-09-12)

The web dashboard's controls, cues and status words are set in the body face (Inter) at 12-13px, weight 500, sentence case. Monospace is reserved for values that are literally code or identifiers: stage ids, branch names, screen sizes, and the terminal well itself. Uppercase with wide tracking is reserved for the three existing label utilities (`eyebrow`, `.stage-tag`, `.rank-caption`); do not coin new all-caps monospaced chips for buttons or indicators.

Set on 2026-09-12 when the terminal view's mode key, the "take control" cue over the well, the ended/dropped stamp and the `>_` card glyph were reset from 10-11px uppercase mono to this convention (`web/src/components/terminal/terminal-controls.css`, `terminal.css`). The card glyph sits last in the stage card header so it lands in the card's top-right corner, after the hover-only open button (`web/src/components/graph/stage-node.tsx`).

Rebuilding the bundle: `web/dist` is embedded by `loom/build.rs`, so a frontend change ships only once `cd web && bun run build` has run and the rebuilt `web/dist` is committed as its own `chore(web): rebuild the dashboard bundle ...` commit.

## A Read-Only Slot's Text Needs an Explicit Line-Height, Not `inline-flex`, to Centre (2026-09-12)

`.settings-ctl-readonly` is a `<span>` blockified as a flex item of `.settings-slot`
(28px tall); with `line-height: normal` its text hugs the top instead of centring.
Fix with an explicit `line-height: 26px` (the 28px border-box height minus two 1px
borders) on the span itself — not `inline-flex`, which centres the text but disables
`text-overflow: ellipsis` on the phone-card layout. `overflow: hidden` +
`text-overflow: ellipsis` is scoped to `.settings-tc .settings-ctl` (the phone-card
selector) rather than the shared `.settings-ctl` rule, so the table layout's cells
never truncate.
