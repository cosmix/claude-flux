# W4 — component tests (sonnet, `loom-software-engineer`)

You run after W1, W2 and W3 returned and the orchestrator confirmed `bun run typecheck` is
green. Load `Skill(skill="loom-skills", args="loom-react loom-typescript")`. Read the three
components you test before writing a single assertion: `settings-dialog.tsx`,
`settings-lanes.tsx`, `settings-cards.tsx`, and W1's `settings-control.tsx`. The accessible
names, class names and data attributes below come from the W1–W3 briefs; if a component
deviates, test what it DOES and report the deviation.

## Files you own (write)

- `web/src/components/settings-dialog.test.tsx` — rewrite. Keep the harness at the top of today's
  file: `entry()`, `snapshot()`, the fake `ConfigClient` with its recorded writes and scripted
  failures, `renderAt(search)` over `createMemoryRouter(routes)` in a jotai `Provider`, `row()`.
  Extend `snapshot()` so it has BOTH halves of one pair set at different tiers
  (`models.standard_model` project-set, `models.standard_effort` user-set is what today's slice
  already has — keep it) and a second pair (`models.knowledge_model`/`_effort`) fully inherited.
- `web/src/components/settings-cards.test.tsx` — new.

Read-only: everything else under `web/src/components/settings-*`, `web/src/test/setup.ts`
(`matchMedia` stub returns `matches: false`), `web/src/router.tsx`.

## `settings-dialog.test.tsx` — cases

Queries: a control is `getByRole("combobox" | "switch" | "textbox", { name: "<field> at user scope" })`
(or `at project scope`); a clear button is `getByRole("button", { name: "clear <field> at user scope" })`;
a row is `row("<key>")` by the key cell's text as today; the tier in effect is the element with
`[data-effective="true"]` inside a cell (`container.querySelector`), or the sr-only text
`in effect` within it.

1. opens from `?settings=1`, renders the three lane heads (`built-in`, `user`, `project`) with the
   user path `~/.loom/config.toml` and the project path from the snapshot, and one control per
   writable lane for `terminal.backend` (two comboboxes) — no radio group exists any more.
2. closes when the param goes away (today's case, unchanged).
3. a pair renders as ONE row: `row("standard")` contains four comboboxes (model/effort × user/project)
   and the summary `runs sonnet · <effort>` built from the effective values; the model's user
   combobox is dashed-inherited (`data-provenance="inherited"` on its slot) while the project one
   is set (`data-provenance="set"`) and carries `data-effective`.
4. user-only sections draw the pane once: `row("check")`'s row contains the text
   `User-only keys`, and `update.check_interval_hours` has no project control; a section with
   `projectAllowed` (`context`) has a project textbox.
5. writing at the project lane sends `{ scope: "project", name, value }` (change the project
   combobox for `terminal.backend` to `tmux`; assert the recorded write and that the control shows
   `tmux` after the client answers).
6. clearing a project override sends `value: null` at project scope and the control falls back to
   the user value (`context.ceiling_tokens` → `800000`); the footer toast says
   `cleared, falls back to user 800,000`.
7. a 400 keeps the draft, shows the message in `role="alert"` beside the control, marks the cell
   `hazard-error`, and Escape in that field drops the draft without closing the dialog; a second
   Escape closes it (today's two cases merged; keep the assertion that the message id is in the
   control's `aria-describedby`).
8. a switch reverts after a failed write, and the error shows only on the lane it was written at
   (today's case, re-targeted to `check at user scope`).
9. the number field does not write when left unchanged (today's case).
10. filter: typing `sonnet` in `getByRole("searchbox", { name: "filter settings" })` leaves only
    rows whose values or help mention it (compute the expected set from `snapshot()` in the test,
    do not hardcode); typing gibberish shows `nothing matches`; pressing `/` on the dialog focuses
    the filter (`document.activeElement`), and `/` typed INSIDE a combobox does not; Escape with
    text in the filter clears it and keeps the dialog open.
11. no project workspace: with `project.available: false`, the project head shows `not available`,
    the pane title is `No project workspace`, and there is no `at project scope` control anywhere.
12. pending state: with a client whose `write` never resolves, the control is disabled and the
    roundel's status text `saving` is in the document (`getByText("saving")` — it is the
    roundel's live region).

## `settings-cards.test.tsx`

Override `window.matchMedia` in a `beforeEach` to return `matches: true` for
`(max-width: 699px)` (and false otherwise), restore in `afterEach`. Then:

1. opening the dialog renders `settings-cards` and NO `<table>`.
2. a pair card (`standard`) has the three tiers labelled `built-in`, `user`, `project` in that
   order, two controls in the user tier and two in the project tier, the summary line, and
   `data-effective` on the project tier.
3. a user-only section's cards carry the slim `user-only keys, no project tier` row and no
   project control.
4. a write from a card control records the same `{ scope, name, value }` as the table would.

Run ONLY `cd web && bunx vitest run src/components/settings-dialog.test.tsx src/components/settings-cards.test.tsx`
once. Report: the test names, which passed, and for each failure whether it is your test or the
component (quote the component line) — the orchestrator decides who fixes it.
