# W1 — model, controls, media query (sonnet, `loom-software-engineer`)

You run FIRST and alone: W2 and W3 compile against the exports you write, so every signature
below is a contract. Match it exactly. Load `Skill(skill="loom-skills", args="loom-typescript loom-react")`
before writing anything.

The approved design is `doc/plans/briefs/settings-lanes/settings-lanes/mockup.html`. Read its
`<script>` block: the functions `laneState`-equivalents (`shown`, `own`, `fallback`, `effLane`,
`prov`), `rowText`/`visibleRows` (the filter), and `sections` (the pairing shape) are the
behaviour you are porting to typed code over `ConfigEntry`. The mock's data model is hand-rolled;
the real one is `web/src/api/config.ts` (`ConfigEntry`, `ConfigKind`, `ConfigScope`,
`ConfigSnapshot`). Never change `web/src/api/**`.

## Files you own (write)

- `web/src/components/settings-model.ts` — extend; keep every existing export except `railStops`
  and `RailStop`, which retire with the provenance rail (their only importer was
  `settings-row.tsx`, which W2 deletes).
- `web/src/components/settings-model.test.ts` — new.
- `web/src/components/settings-control.tsx` — rewrite around the contract below.
- `web/src/lib/use-media-query.ts` — new.

Read-only: `web/src/api/config.ts`, `web/src/api/fixtures/config.json` (18 real entries, use it
in tests), `web/src/components/settings-row.tsx` (the logic you are moving out of it before W2
deletes it), `web/src/aurora-ui/feedback/BusyRoundel.tsx`, `web/src/test/setup.ts` (the
`matchMedia` stub your hook must tolerate: `matches: false`, no-op `addEventListener`).

## 1. `settings-model.ts` — the contract

Keep as they are: `USER_CONFIG_PATH`, `SCOPES`, `scopePath`, `sectionOf`, `fieldOf`, `Section`,
`groupBySection`, `Provenance`, `provenanceAt`, `valueAt`, `fallbackFor`, `displayValue`.

Add, exported, with these exact names and shapes:

    export type Lane = "builtin" | "user" | "project";
    export const LANES: readonly { lane: Lane; label: string; path: string | null; blurb: string }[];
    // builtin: label "built-in", path null, blurb "loom's defaults"
    // user:    label "user",     path USER_CONFIG_PATH, blurb "every loom project on this machine"
    // project: label "project",  path ".loom/work/config.toml", blurb "this workspace only"
    // (the dialog substitutes data.project.path for the project row at render time)

    export function effectiveLane(entry: ConfigEntry): Lane;
    // entry.effective.source: "default" -> "builtin", otherwise the same word.

    export type LaneProvenance = Provenance | "readonly";
    export interface LaneState {
      lane: Lane;
      /// what a control in this lane shows: the value the file sets, or the value falling
      /// through from the tier below; null only when provenance is "unavailable".
      value: string | null;
      provenance: LaneProvenance;   // builtin lane is always "readonly"
      effective: boolean;           // effectiveLane(entry) === lane
    }
    export function laneState(entry: ConfigEntry, lane: Lane): LaneState;
    // builtin: value entry.default. user/project: provenanceAt + valueAt from today's file.

    export function laneScope(lane: Lane): ConfigScope | null;   // builtin -> null

    export interface PairRow { kind: "pair"; label: string; caption: string | null; model: ConfigEntry; effort: ConfigEntry }
    export interface SingleRow { kind: "single"; entry: ConfigEntry }
    export type SettingsRow = PairRow | SingleRow;
    export interface SectionRows { section: string; caption: string | null; projectAllowed: boolean; rows: SettingsRow[] }
    export function sectionRows(entries: ConfigEntry[]): SectionRows[];
    export function rowEntries(row: SettingsRow): ConfigEntry[];   // [model, effort] or [entry]
    export function rowLabel(row: SettingsRow): string;            // pair label, or fieldOf(entry.name)

    export function filterSections(sections: SectionRows[], query: string): SectionRows[];

    export function formatValue(kind: ConfigKind, value: string): string;
    // bool -> displayValue ("on"/"off"); u32 -> Number(value).toLocaleString("en-US") when the
    // string is all digits, else the string unchanged; enum -> value.

    export type WriteStatus =
      | { phase: "idle" }
      | { phase: "pending"; value: string | null }
      | { phase: "saved" }
      | { phase: "error"; message: string };
    export function statusKey(scope: ConfigScope, name: string): string;   // `${scope}:${name}`

    export const SECTION_CAPTIONS: Readonly<Record<string, string>>;
    export const ROW_CAPTIONS: Readonly<Record<string, string>>;

`WriteStatus` moves here from `settings-row.tsx` and `statusKey` from `settings-dialog.tsx`,
unchanged.

### Pairing rule (`sectionRows`)

Group entries with `groupBySection` (registry order is preserved). Within a section, a field
name ending in `_model` or `_effort` has a prefix (the part before that suffix). When BOTH
`<prefix>_model` and `<prefix>_effort` exist in the section, they form ONE `PairRow` placed
where the first of the two appears; every other entry is a `SingleRow` in registry order. A
prefix with only one half stays a single row, so a new registry key never breaks the layout.
`projectAllowed` is true when ANY entry in the section has `"project"` in `entry.scopes`.
`caption` is `SECTION_CAPTIONS[section] ?? null`; a pair's `caption` is
`ROW_CAPTIONS[\`${section}.${prefix}\`] ?? null`.

Captions (frontend copy, the registry has none):

    SECTION_CAPTIONS: pressure -> "who runs each step of loom pressure"
                      models   -> "a stage's main agent session, by stage type"
    ROW_CAPTIONS:     pressure.claude  -> "/pressure, Claude"
                      pressure.codex   -> "$pressure, Codex"
                      pressure.address -> "/address reconciliation"

With the 18-entry fixture, `sectionRows` yields five sections: update (2 singles), terminal (1),
context (1), pressure (3 pairs: claude, codex, address), models (4 pairs: standard, knowledge,
knowledge_distill, integration_verify). `projectAllowed` is true only for terminal and context.

### Filter rule (`filterSections`)

`query.trim().toLowerCase()`; empty returns the input array unchanged (same reference). A row
matches when the haystack contains the query as a substring. The haystack for a row is the
section name, section caption, `rowLabel`, the pair caption, and for each entry in `rowEntries`:
`entry.name`, `fieldOf(entry.name)`, `entry.help`, `entry.default`, `entry.user.value`,
`entry.project?.value`, `entry.effective.value`, and `displayValue(kind, ...)` of each of those
values (so "on" finds a true bool). Sections with no matching rows are dropped.

## 2. `settings-control.tsx` — the contract

Keep `ValueControl` and its three kinds (switch, text-as-number with local draft, select with
the unknown-variant option). Behaviour to keep verbatim from today's file: the number field's
draft/commit/Escape semantics and its comment about why it is `type="text"`; the select's
`options` fallback. Change the props to:

    export interface ControlProps {
      id: string;
      kind: ConfigKind;
      value: string;
      pending: boolean;
      invalid: boolean;
      /// accessible name; the same key appears once per lane, so callers pass
      /// `${fieldOf(name)} at ${lane} scope`.
      label: string;
      describedBy?: string;
      onCommit: (value: string) => void;
    }

Class names (W2 writes their CSS; use them exactly): the text field and select carry
`settings-ctl` (plus `settings-ctl-num` on the number field), the select wrapper
`settings-select-wrap` with the chevron `settings-select-chevron` (lucide `ChevronDownIcon`,
as today), the switch `settings-switch`. Every control sets `aria-label={label}`,
`aria-busy` while pending, `aria-invalid` when invalid, `disabled` while pending.

Add two components:

    export interface LaneSlotProps {
      entry: ConfigEntry;
      lane: "user" | "project";
      status: WriteStatus;
      controlId: string;
      onWrite: (value: string | null) => void;   // null clears the key at this lane
    }
    export function LaneSlot(props: LaneSlotProps): ReactElement;

    export function BuiltinValue({ entry }: { entry: ConfigEntry }): ReactElement;

`LaneSlot` renders:

    <span class="settings-slot" data-lane={lane} data-provenance={p} data-effective={state.effective || undefined}>
      <ValueControl ... />                              // shown value, see below
      {pending && <BusyRoundel busy size={12} busyLabel="saving" idleLabel="" />}
      {p === "set" && !pending && (
        <button type="button" class="settings-clear" aria-label={`clear ${field} at ${lane} scope`}
                title={`falls back to ${fallback.tier} ${formatValue(kind, fallback.value)}`}
                onClick={() => onWrite(null)}>✕</button>
      )}
      {state.effective && <span class="sr-only">in effect</span>}
    </span>
    {status.phase === "error" && <span class="hazard-text settings-error" role="alert" id={`${controlId}-error`}>{status.message}</span>}

where `state = laneState(entry, lane)`, `p = status.phase === "error" ? "error" : status.phase === "pending" ? "pending" : state.provenance`,
and the shown value is today's `SettingRow` rule moved here: while pending, `status.value ?? fallbackFor(entry, lane).value`;
otherwise `state.value`. `describedBy` is the error id when there is an error. `BusyRoundel` is
`@/aurora-ui/feedback/BusyRoundel` (see its props in the file; `busy`, `size`, `busyLabel`,
`idleLabel`). When `state.provenance === "unavailable"` render `<span class="settings-na">user only</span>`
instead of a control (this is the per-row fallback for a mixed section; W2 draws the
whole-section pane itself).

The `data-effective` attribute plus the sr-only text is how tests and CSS find the tier in
effect; do not use `aria-current` on a form control.

`BuiltinValue` renders `<span class="settings-slot" data-lane="builtin" data-provenance="readonly" data-effective=...><span class="settings-ctl settings-ctl-readonly">{formatValue(kind, entry.default)}</span>{effective && <span class="sr-only">in effect</span>}</span>`.

## 3. `use-media-query.ts`

    export function useMediaQuery(query: string): boolean

`useSyncExternalStore` over `window.matchMedia(query)`: subscribe with
`addEventListener("change", cb)` and return the remover; `getSnapshot` returns `.matches`;
`getServerSnapshot` returns false. Guard `typeof window === "undefined"` → false. The jsdom stub
in `web/src/test/setup.ts` returns `matches: false` and no-op listeners, so the desktop table is
the default under tests; a test that wants the phone layout overrides `window.matchMedia` itself.

## 4. Tests — `settings-model.test.ts`

Vitest, no DOM. Load the fixture with `import fixture from "@/api/fixtures/config.json"` and
parse it through `configResponseSchema` from `@/api/config` (the existing `schema.test.ts`-style
precedent: grep `fixtures/` under `web/src/api` for the import form). Cases:

- `sectionRows(fixture.entries)` gives the five sections in order with the row shapes listed in
  the pairing rule; pressure rows are pairs labelled `claude`, `codex`, `address` with the three
  captions; `projectAllowed` true only for terminal and context.
- an entry set with only `foo_model` in a made-up section stays a `SingleRow`.
- `laneState` for `context.ceiling_tokens` in the fixture (project sets 900000 or whatever the
  fixture says — read it): builtin readonly/not effective, user inherited, project set and
  effective; `effectiveLane` agrees.
- `filterSections` with `"sonnet"` keeps exactly the rows whose values or help mention sonnet
  (measure against the fixture in the test, don't hardcode a count you did not compute);
  `""` returns the same array reference; a nonsense query returns `[]`; matching is
  case-insensitive; `"on"` matches `update.check` through `displayValue`.
- `formatValue` groups `"800000"` to `"800,000"`, leaves `"abc"` alone, maps bool to on/off.

Run ONLY `cd web && bunx vitest run src/components/settings-model.test.ts` once. Report files
changed, the exact exported signatures, and anything you deviated from.
