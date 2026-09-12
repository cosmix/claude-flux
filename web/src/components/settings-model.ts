import type { ConfigEntry, ConfigKind, ConfigScope } from "@/api/config";

export const USER_CONFIG_PATH = "~/.loom/config.toml";

/// Key names are dotted `section.field`; entries arrive in registry order
/// and keep it, with sections in order of first appearance.
export function sectionOf(name: string): string {
  const dot = name.indexOf(".");
  return dot === -1 ? name : name.slice(0, dot);
}

export function fieldOf(name: string): string {
  const dot = name.indexOf(".");
  return dot === -1 ? name : name.slice(dot + 1);
}

export interface Section {
  section: string;
  entries: ConfigEntry[];
}

export function groupBySection(entries: ConfigEntry[]): Section[] {
  const groups: Section[] = [];
  for (const entry of entries) {
    const section = sectionOf(entry.name);
    const group = groups.find((candidate) => candidate.section === section);
    if (group) group.entries.push(entry);
    else groups.push({ section, entries: [entry] });
  }
  return groups;
}

/// How a control at `scope` relates to the file it writes: `set` means the
/// file names the value itself; `inherited` means it falls through to the
/// tier below and the control shows that tier's value; `unavailable` means
/// the key cannot be written at this scope at all.
export type Provenance = "set" | "inherited" | "unavailable";

export function provenanceAt(entry: ConfigEntry, scope: ConfigScope): Provenance {
  if (scope === "user") return entry.user.set ? "set" : "inherited";
  if (entry.project === null) return "unavailable";
  return entry.project.set ? "set" : "inherited";
}

/// The value a control at `scope` shows, or null when it has none to show.
export function valueAt(entry: ConfigEntry, scope: ConfigScope): string | null {
  if (scope === "user") return entry.user.value;
  return entry.project?.value ?? null;
}

/// What clearing the key at `scope` falls back to.
export function fallbackFor(
  entry: ConfigEntry,
  scope: ConfigScope,
): { tier: string; value: string } {
  if (scope === "project") return { tier: "user", value: entry.user.value };
  return { tier: "built-in", value: entry.default };
}

/// Values are strings on the wire; booleans read better as words.
export function displayValue(kind: ConfigKind, value: string): string {
  if (kind.type === "bool") return value === "true" ? "on" : "off";
  return value;
}

/// The three files a key can resolve from, left (weakest) to right
/// (strongest): built-in, then user, then project.
export type Lane = "builtin" | "user" | "project";

export const LANES: readonly { lane: Lane; label: string; path: string | null; blurb: string }[] = [
  { lane: "builtin", label: "built-in", path: null, blurb: "loom's defaults" },
  {
    lane: "user",
    label: "user",
    path: USER_CONFIG_PATH,
    blurb: "every loom project on this machine",
  },
  {
    lane: "project",
    label: "project",
    path: ".loom/work/config.toml",
    blurb: "this workspace only",
  },
];

/// The lane whose value loom actually uses.
export function effectiveLane(entry: ConfigEntry): Lane {
  return entry.effective.source === "default" ? "builtin" : entry.effective.source;
}

/// A lane's provenance is a scope's `Provenance`, plus `"readonly"` for the
/// built-in lane, which no control ever writes to.
export type LaneProvenance = Provenance | "readonly";

export interface LaneState {
  lane: Lane;
  /// What a control in this lane shows: the value the file sets, or the
  /// value falling through from the tier below; null only when provenance
  /// is "unavailable".
  value: string | null;
  provenance: LaneProvenance;
  effective: boolean;
}

export function laneState(entry: ConfigEntry, lane: Lane): LaneState {
  if (lane === "builtin") {
    return {
      lane,
      value: entry.default,
      provenance: "readonly",
      effective: effectiveLane(entry) === "builtin",
    };
  }
  return {
    lane,
    value: valueAt(entry, lane),
    provenance: provenanceAt(entry, lane),
    effective: effectiveLane(entry) === lane,
  };
}

export function laneScope(lane: Lane): ConfigScope | null {
  return lane === "builtin" ? null : lane;
}

export interface PairRow {
  kind: "pair";
  label: string;
  caption: string | null;
  model: ConfigEntry;
  effort: ConfigEntry;
}

export interface SingleRow {
  kind: "single";
  entry: ConfigEntry;
}

export type SettingsRow = PairRow | SingleRow;

export interface SectionRows {
  section: string;
  caption: string | null;
  projectAllowed: boolean;
  rows: SettingsRow[];
}

export const SECTION_CAPTIONS: Readonly<Record<string, string>> = {
  pressure: "who runs each step of loom pressure",
  models: "a stage's main agent session, by stage type",
};

export const ROW_CAPTIONS: Readonly<Record<string, string>> = {
  "pressure.claude": "/pressure, Claude",
  "pressure.codex": "$pressure, Codex",
  "pressure.address": "/address reconciliation",
};

/// A `_model`/`_effort` suffix's prefix, or null when the field doesn't end
/// in either.
function pairPrefix(field: string): string | null {
  if (field.endsWith("_model")) return field.slice(0, -"_model".length);
  if (field.endsWith("_effort")) return field.slice(0, -"_effort".length);
  return null;
}

export function sectionRows(entries: ConfigEntry[]): SectionRows[] {
  return groupBySection(entries).map(({ section, entries: sectionEntries }) => {
    const byField = new Map(sectionEntries.map((entry) => [fieldOf(entry.name), entry]));
    const rows: SettingsRow[] = [];
    const paired = new Set<string>();
    for (const entry of sectionEntries) {
      const field = fieldOf(entry.name);
      if (paired.has(field)) continue;
      const prefix = pairPrefix(field);
      const model = prefix !== null ? byField.get(`${prefix}_model`) : undefined;
      const effort = prefix !== null ? byField.get(`${prefix}_effort`) : undefined;
      if (prefix !== null && model && effort) {
        paired.add(`${prefix}_model`);
        paired.add(`${prefix}_effort`);
        rows.push({
          kind: "pair",
          label: prefix,
          caption: ROW_CAPTIONS[`${section}.${prefix}`] ?? null,
          model,
          effort,
        });
      } else {
        rows.push({ kind: "single", entry });
      }
    }
    return {
      section,
      caption: SECTION_CAPTIONS[section] ?? null,
      projectAllowed: sectionEntries.some((entry) => entry.scopes.includes("project")),
      rows,
    };
  });
}

export function rowEntries(row: SettingsRow): ConfigEntry[] {
  return row.kind === "pair" ? [row.model, row.effort] : [row.entry];
}

export function rowLabel(row: SettingsRow): string {
  return row.kind === "pair" ? row.label : fieldOf(row.entry.name);
}

function rowHaystack(section: SectionRows, row: SettingsRow): string {
  const parts = [section.section, section.caption ?? "", rowLabel(row)];
  if (row.kind === "pair") parts.push(row.caption ?? "");
  for (const entry of rowEntries(row)) {
    parts.push(
      entry.name,
      fieldOf(entry.name),
      entry.help,
      entry.default,
      displayValue(entry.kind, entry.default),
      entry.user.value,
      displayValue(entry.kind, entry.user.value),
      entry.effective.value,
      displayValue(entry.kind, entry.effective.value),
    );
    if (entry.project !== null) {
      parts.push(entry.project.value, displayValue(entry.kind, entry.project.value));
    }
  }
  return parts.join(" ").toLowerCase();
}

export function filterSections(sections: SectionRows[], query: string): SectionRows[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return sections;
  return sections
    .map((section) => ({
      ...section,
      rows: section.rows.filter((row) => rowHaystack(section, row).includes(needle)),
    }))
    .filter((section) => section.rows.length > 0);
}

export function formatValue(kind: ConfigKind, value: string): string {
  if (kind.type === "bool") return displayValue(kind, value);
  if (kind.type === "u32" && /^\d+$/.test(value)) return Number(value).toLocaleString("en-US");
  return value;
}

/// Writes one key's value at one scope, or clears it (`null`) to fall back
/// to the tier below.
export type OnWrite = (scope: ConfigScope, name: string, value: string | null) => void;

/// One key's write in flight or just settled; the control shows the pending
/// value until the server answers, then the entry it returned.
export type WriteStatus =
  | { phase: "idle" }
  | { phase: "pending"; value: string | null }
  | { phase: "saved" }
  | { phase: "error"; message: string };

export function statusKey(scope: ConfigScope, name: string): string {
  return `${scope}:${name}`;
}

/// The status of one key's write at one scope, or idle when none is in flight.
export function statusFor(
  statuses: Record<string, WriteStatus>,
  scope: ConfigScope,
  name: string,
): WriteStatus {
  return statuses[statusKey(scope, name)] ?? { phase: "idle" };
}
