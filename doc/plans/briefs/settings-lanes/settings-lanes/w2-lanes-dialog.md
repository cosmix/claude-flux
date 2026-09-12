# W2 — the dialog, the lanes table, the stylesheet (sonnet, `loom-software-engineer`)

Load `Skill(skill="frontend-design:frontend-design")` and
`Skill(skill="loom-skills", args="loom-react loom-typescript")` first. The design is settled:
`doc/plans/briefs/settings-lanes/settings-lanes/mockup.html`, the `.lp` block and its CSS. Open
it in a browser AND read it. Match it; the page chrome around it is not part of the design.
Your job is craft in the port, not a new direction.

W1 has already written `settings-model.ts`, `settings-control.tsx` and `use-media-query.ts`;
their exports are quoted in the plan under "Contract from W1". Import them; do not edit them.
W3 writes the phone card list (`settings-cards.tsx`) in parallel with you and imports the same
W1 exports; it also renders inside YOUR dialog, so leave the mount point described in §1.

## Files you own (write)

- `web/src/components/settings-dialog.tsx` — rewrite.
- `web/src/components/settings-lanes.tsx` — new.
- `web/src/components/settings.css` — rewrite.
- `web/src/components/settings-row.tsx` — DELETE (its rail, tags and reset button retire;
  `WriteStatus` now lives in `settings-model.ts`).

Read-only: everything W1 wrote, `web/src/api/config.ts`, `web/src/components/ui/dialog.tsx`,
`web/src/components/ui/kbd.tsx`, `web/src/components/ui/button.tsx`,
`web/src/aurora-ui/feedback/EmptyState.tsx`, `web/src/aurora-ui/feedback/hazard.css`,
`web/src/index.css` (tokens; do not edit), `web/src/routes/shell.tsx:51` (mounts
`<SettingsDialog />`, unchanged) and `web/src/components/header.tsx:22-45` (calls
`useOpenSettings()` with no argument, unchanged).

## 1. `settings-dialog.tsx`

Keep: `SETTINGS_PARAM`, `useOpenSettings`, the `Dialog`/`DialogContent` shell, the `?settings=`
URL contract (presence opens, removal closes, back button closes), the `LoadState` machine, the
`write` function with its CSRF-403 reload, `replaceEntry`. Drop: `scopeFromParam`, `ScopeSwitch`,
`Entries`, the per-scope `setScope` replace-navigation, the "writing to" line. The scope is no
longer state: every write names its lane, so `write(scope: ConfigScope, name, value)` takes
the scope as its first argument and `statuses` stays keyed by `statusKey(scope, name)`.

`useOpenSettings` returns `() => void` and sets the param to `"1"` (the header is its only
caller and passes nothing; grep to confirm before you change the signature).

Shape of the content:

    <DialogContent className={cn("settings-dialog gap-0 overflow-hidden p-0 sm:max-w-[980px]")}
                   onKeyDown={onSlash} onEscapeKeyDown={onEscape}>
      <DialogHeader className="settings-head">
        <div> <DialogTitle>Settings</DialogTitle>
              <DialogDescription>A key takes the rightmost value that is set. Edit a cell to set it in that file; clear it to fall back. Sections with two sub-columns are read as one row per stage or step.</DialogDescription> </div>
        <label className="settings-filter"> <SearchIcon/> <input type="search" data-filter aria-label="filter settings" placeholder="filter keys, values, help" .../> <Kbd>/</Kbd> </label>
      </DialogHeader>
      <div className="settings-scroll"> {notice | (narrow ? <SettingsCards .../> : <SettingsLanes .../>)} </div>
      <footer className="settings-foot"> <span><i className="settings-legend-eff"/>in effect · <Kbd>Esc</Kbd> closes</span> <span className="settings-toast" aria-live="polite">{toast}</span> </footer>
    </DialogContent>

- `narrow = useMediaQuery("(max-width: 699px)")`. `SettingsCards` is W3's component; import it
  from `@/components/settings-cards` with props `{ data, sections, statuses, onWrite }` — the same
  props as `SettingsLanes` (pinned in the plan). It will not exist until W3 returns; write the
  import anyway, the orchestrator compiles after both of you finish.
- Filter: `query` is component state; `sections = useMemo(() => filterSections(sectionRows(data.entries), query))`.
  `onSlash`: when `event.key === "/"` and the target is not an `input`, `select` or `textarea`,
  `preventDefault()` and focus + select the filter input (a ref). Escape inside the filter with
  a non-empty value clears it and `stopPropagation()`s; `onEscape` additionally
  `preventDefault()`s when `event.target` matches `[data-filter]` with a value or `[data-draft]`
  (today's number-draft rule, keep the comment).
- Toast: `"each cell writes on its own"` at rest; while a write is pending
  `writing <name> at <lane>…`; after success `saved` (or `cleared, falls back to <tier> <value>`
  when the value written was null, using `fallbackFor` and `formatValue`); after failure
  `the server rejected the write`. The per-control error text itself is rendered by `LaneSlot`.
- `sections` with zero rows (a filter that matches nothing): `SettingsLanes` renders one row
  spanning the table saying `nothing matches “<query>”`; pass `query` down for that.

## 2. `settings-lanes.tsx`

    export interface SettingsTableProps {
      data: ConfigSnapshot;
      sections: SectionRows[];
      query: string;
      statuses: Record<string, WriteStatus>;
      onWrite: (scope: ConfigScope, name: string, value: string | null) => void;
    }
    export function SettingsLanes(props: SettingsTableProps): ReactElement;

A real `<table className="settings-table">` with `<colgroup>` widths 186 / 104 / 78 / 132 / 98 /
132 / 98 px (from the mock), `table-layout: fixed`. Head row: an empty first `<th>`, then three
`<th scope="colgroup" colSpan={2}>` lane heads from `LANES` — `<b>label</b>` and the path in
mono (project shows `data.project.path`); when `data.project.available` is false the project
head also carries `<span className="settings-tag tone-warning">not available</span>`
(`toneClass("warning")` from `state-badge.tsx`).

Per section, one `<tbody>`:

- Section row `<tr className="settings-sech">`: `<th className="settings-key">` with the
  eyebrow section name (`className="eyebrow"`, the existing utility) and the caption under it;
  then for a section with pairs six `<th>`s labelled model/effort per lane (the project pair of
  heads collapses to one empty `<th colSpan={2}>` when `!projectAllowed`); for a singles-only
  section three empty `<th colSpan={2}>`. The FIRST cell of every lane, in every row, carries
  `settings-ls` (lane start); every user-lane cell carries `settings-lane-user`, every project
  cell `settings-lane-project`, built-in `settings-lane-builtin`.
- Key cell: pair → `<span className="settings-key-name">{label}</span>`, the caption if any, and
  `<span className="settings-res">runs <b>{formatValue(model)}</b> · <b>{formatValue(effort)}</b></span>`
  using `entry.effective.value`; single → name + help. Give the pair label a `title` joining
  both keys' `help` texts.
- Built-in cells: `<BuiltinValue entry=.../>`, two per pair, one `colSpan={2}` per single.
- User cells: `<LaneSlot lane="user" .../>` with `status = statuses[statusKey("user", name)] ?? {phase:"idle"}`
  and `onWrite = (v) => props.onWrite("user", name, v)`; `controlId` from `useId()` + name.
- Project cells: same with `lane="project"` when `projectAllowed && data.project.available`.
  Otherwise the FIRST row of the section gets ONE `<td className="settings-lane-project settings-ls settings-na" colSpan={2} rowSpan={rows.length}>`
  holding the pane (§3) and the other rows emit no project cell.
- Cell error state: when the lane's status is `error`, add `hazard-error` (aurora's stripe class,
  already imported through `index.css`) to that `<td>`.
- Empty result: `<tr><td colSpan={7} className="settings-key settings-empty">nothing matches “{query}”</td></tr>`.

## 3. The user-only pane

aurora-ui's `EmptyState` (`@/aurora-ui/feedback/EmptyState`, read its props): `variant="bare"`,
`size="sm"`, `tone="muted"`, `icon={UserRoundIcon}` from lucide, title `User-only keys`,
description `The project file has no tier for this section.` When `data.project.available` is
false use title `No project workspace` and description
`` `${data.project.path}` cannot be written, so these keys keep their user values. `` (path in
`<code>`). The pane is wrapped in `<div className="settings-pane">` which the CSS draws as the
dashed inset panel from the mock (absolute inset inside the relative `<td>`), row-form when
`rows.length < 3` (`settings-pane-short`), stacked otherwise. EmptyState's own padding is
larger than the cell; override with `className` on it (`py-0 px-0 gap-2`) rather than editing
the kit.

## 4. `settings.css`

Rewrite from the mock's `.lp`/`.lpt`/`.slot`/`.ctl` rules, renamed to the `settings-` classes
named here and in W1's brief, inside the existing `@layer components` block. Rules that matter:

- Lane tones on the dialog root: `--lane-user: var(--tone-executing)`,
  `--lane-project: var(--tone-completed)`, `--lane-builtin: var(--tone-dimmed)`.
- Alternating lane ground: `.settings-lane-user { background: color-mix(in oklch, var(--muted) 55%, var(--card)) }`;
  `.settings-ls { border-left: 1px solid var(--hairline) }`; sticky thead cells with a 3px lane
  tone bar (`::before`) and `background: var(--card)` (user head keeps the muted mix).
- Section rows have `border-top: 1px solid var(--hairline)` except the first `tbody`'s.
- Controls: `.settings-ctl` (28px tall, mono 12.5px, tabular nums, 7px radius),
  `[data-provenance="set"]` solid border in `--ctl-tone`, `[data-provenance="inherited"]`
  dashed + muted + transparent, `[data-provenance="pending"]` 0.55 opacity + progress cursor,
  `[data-provenance="error"]` blocked-tone border + ring, `.settings-ctl-readonly` inert muted
  fill and no border; `.settings-slot[data-lane="project"]` sets `--ctl-tone: var(--lane-project)`,
  user lane the user tone. Number field width `11ch`, right-aligned; select `width: 100%` with the
  chevron absolutely placed. The switch keeps today's pill rules with the hatched inherited fill.
- Effective marker: `.settings-slot[data-effective="true"]::after` 2px `var(--fg)` underline at
  `bottom: -5px`, `left/right: 2px`.
- `.hazard-text` already exists in aurora's `hazard.css`; add `.settings-error { display: block; margin-top: 5px; font-size: 11px; }`.
- Filter: `.settings-filter` 30px, 240px wide, `var(--bg)` fill, ring on `:focus-within`, the
  search input unstyled inside, webkit cancel button hidden.
- Footer legend `.settings-legend-eff` (14×2px bar), `.settings-toast` right-aligned muted.
- `.settings-scroll { max-height: min(660px, 72dvh); overflow: auto; }`.
- Dark mode: this app toggles `.dark` on `<html>` (not the media query) — any dark-only rule
  uses `.dark .settings-…`. Reduced motion: keep today's block and add nothing that animates
  besides the kit's roundel.
- Below 700px the table is never rendered (the dialog swaps to W3's cards), so no responsive
  rules for the table; keep `sm:max-w-[980px]` from being wider than the viewport by leaving
  Radix's `max-w-[calc(100%-2rem)]` in place (it is on `DialogContent` already).

Sizes: no file over 400 lines, no function over 50. If `settings-lanes.tsx` grows past that,
split the pane and the key cell into `settings-lanes-cells.tsx` (you own it then; say so).

Run ONLY `cd web && bunx vitest run src/components/settings-model.test.ts` once if you want a
sanity check (it exercises nothing you wrote; typecheck, lint, format and the dialog tests are
the orchestrator's and W4's). Report: files changed, class names you added beyond this brief,
anything you could not match from the mock.
