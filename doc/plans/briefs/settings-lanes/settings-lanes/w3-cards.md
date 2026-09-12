# W3 — the phone card list (sonnet, `loom-software-engineer`)

Load `Skill(skill="frontend-design:frontend-design")` and
`Skill(skill="loom-skills", args="loom-react loom-typescript")` first. The design is the
`.phone` block of `doc/plans/briefs/settings-lanes/settings-lanes/mockup.html` and its
`renderPhone`/`tier` functions in the `<script>`: one card per row, the three tiers stacked
top to bottom, model and effort side by side inside a paired card. Match it.

W1's exports are the contract (quoted in the plan under "Contract from W1"); import, never edit.
W2 writes the dialog and the desktop table in parallel with you and will render your component
below 700px with exactly these props — do not change them.

## Files you own (write)

- `web/src/components/settings-cards.tsx` — new.
- `web/src/components/settings-cards.css` — new.
- `web/src/index.css` — ONE line: `@import "./components/settings-cards.css";` directly after
  the existing `@import "./components/settings.css";` line (index.css:18). Nothing else.

Read-only: everything W1 wrote, `web/src/api/config.ts`, `web/src/components/settings-control.tsx`
(W1's `LaneSlot`/`BuiltinValue` — you compose them, you do not restyle them),
`web/src/components/state-badge.tsx` (`toneClass`).

## Component

    export interface SettingsTableProps {   // identical to W2's; re-declare it here, do not import from W2
      data: ConfigSnapshot;
      sections: SectionRows[];
      query: string;
      statuses: Record<string, WriteStatus>;
      onWrite: (scope: ConfigScope, name: string, value: string | null) => void;
    }
    export function SettingsCards(props: SettingsTableProps): ReactElement;

Render `<div className="settings-cards">`:

- A legend line first: `<div className="settings-cards-key">` with three items from `LANES`
  (`built-in`, `user · ~/.loom`, `project · .loom/work`), each with a 7px tone square before it
  (CSS `::before`, `data-lane` on the item).
- Per section: `<div className="settings-cards-sec">` — the section name as `eyebrow` and, at
  the right, the section caption if any.
- Per row: `<article className="settings-card" aria-labelledby={headingId}>`:
  - header `settings-card-h`: the row label (`settings-key-name`, id = headingId) + the pair
    caption or single help in `settings-help`; at the right for a pair the resolved summary
    `runs <b>model</b> · <b>effort</b>` (`settings-res`, `formatValue` of `effective.value`).
  - for a pair, a sub-header row `settings-card-subh` with cells `tier` (visually hidden),
    `model`, `effort`.
  - three tiers, each `<div className="settings-tier" data-lane=...>` with a label
    (`settings-tl`: `built-in` / `user` / `project`) and `settings-tc` (`settings-tc-pair` for a
    pair) holding `BuiltinValue` or `LaneSlot` per entry. Add `data-effective="true"` on the tier
    whose entries include the lane in effect (any of them), and `hazard-error` on a tier whose
    lane status is `error`.
  - the project tier: when `sectionRows` says `projectAllowed && data.project.available`, a real
    tier; otherwise a slim `settings-tier settings-tier-na` row reading `user-only keys, no project tier`
    (or `no project workspace to write to` when `!data.project.available`).
- Empty result: `<p className="settings-help settings-empty">nothing matches “{query}”</p>`.

`controlId` for each `LaneSlot`: `useId()` + lane + name. Status lookup:
`statuses[statusKey(scope, entry.name)] ?? { phase: "idle" }`.

## Stylesheet `settings-cards.css`

Inside `@layer components`, from the mock's `.phone .ph-*` and `.card` rules renamed:

- `.settings-cards { display: grid; grid-auto-rows: max-content; gap: 10px; padding: 4px 12px 12px; }`
  (the `grid-auto-rows` line is what keeps cards from collapsing inside a fixed-height scroll
  container — it was a real bug in the mock, keep it).
- `.settings-card` bordered 12px radius; header with hairline below; `.settings-card-subh`
  grid `64px 1fr 1fr`, 10px uppercase tracking; `.settings-tier` grid `64px 1fr`, min-height
  42px, `box-shadow: inset 3px 0 0 var(--lane-builtin)` with the user/project tones by
  `data-lane`; user tier ground `color-mix(in oklch, var(--muted) 55%, var(--card))`;
  `.settings-tier-na` 30px tall, muted 11px, a 35%-alpha project bar.
- `.settings-tc` grid, `.settings-tc-pair` two equal columns, `.settings-slot` and its select
  stretch to the column (`width: 100%; min-width: 0`), the number field keeps `11ch`.
- `[data-effective="true"] .settings-tl { color: var(--fg) }`.
- Dark-only rules use `.dark .settings-…`. No `@media` breakpoint here: the dialog decides which
  layout renders.

No file over 400 lines. Do not run the suite; if you want a sanity check, run ONLY
`cd web && bun run typecheck` once — it may fail on W2's unfinished files, which is expected and
not yours to fix. Report: files changed, class names added, anything from the mock you could
not match.
