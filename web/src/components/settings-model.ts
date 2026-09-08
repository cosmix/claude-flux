import type { ConfigEntry, ConfigKind, ConfigScope } from "@/api/config";

export const USER_CONFIG_PATH = "~/.loom/config.toml";

/// The two files a key can be written to, in layering order: the project
/// file overrides the user file, which overrides loom's built-in.
export const SCOPES: { scope: ConfigScope; path: string; blurb: string }[] = [
  { scope: "user", path: USER_CONFIG_PATH, blurb: "every loom project on this machine" },
  { scope: "project", path: ".loom/work/config.toml", blurb: "this workspace only" },
];

export function scopePath(scope: ConfigScope, projectPath: string): string {
  return scope === "project" ? projectPath : USER_CONFIG_PATH;
}

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

/// One stop on the resolution rail: built-in, then user, then project.
export interface RailStop {
  tier: "built-in" | "user" | "project";
  /// null when the tier has nothing to report: an unset user/project tier,
  /// or a project tier the key cannot use.
  value: string | null;
  set: boolean;
  effective: boolean;
  applicable: boolean;
}

export function railStops(entry: ConfigEntry): RailStop[] {
  const source = entry.effective.source;
  const projectAllowed = entry.scopes.includes("project");
  return [
    {
      tier: "built-in",
      value: entry.default,
      set: true,
      effective: source === "default",
      applicable: true,
    },
    {
      tier: "user",
      value: entry.user.set ? entry.user.value : null,
      set: entry.user.set,
      effective: source === "user",
      applicable: true,
    },
    {
      tier: "project",
      value: entry.project?.set ? entry.project.value : null,
      set: entry.project?.set ?? false,
      effective: source === "project",
      applicable: projectAllowed && entry.project !== null,
    },
  ];
}

/// Values are strings on the wire; booleans read better as words.
export function displayValue(kind: ConfigKind, value: string): string {
  if (kind.type === "bool") return value === "true" ? "on" : "off";
  return value;
}
