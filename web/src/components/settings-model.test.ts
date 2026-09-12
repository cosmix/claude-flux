import { describe, expect, it } from "vitest";

import { configResponseSchema, type ConfigEntry } from "@/api/config";
import fixtureJson from "@/api/fixtures/config.json";
import {
  effectiveLane,
  fieldOf,
  filterSections,
  formatValue,
  laneState,
  rowEntries,
  rowLabel,
  sectionRows,
  statusFor,
  type PairRow,
  type WriteStatus,
} from "@/components/settings-model";

const fixture = configResponseSchema.parse(fixtureJson);

describe("sectionRows", () => {
  const sections = sectionRows(fixture.entries);

  it("groups the fixture into five sections in registry order", () => {
    expect(sections.map((s) => s.section)).toEqual([
      "update",
      "terminal",
      "context",
      "pressure",
      "models",
    ]);
  });

  it("keeps update and terminal/context as single rows, sized as expected", () => {
    expect(sections[0]?.rows).toHaveLength(2);
    expect(sections[0]?.rows.every((row) => row.kind === "single")).toBe(true);
    expect(sections[1]?.rows).toHaveLength(1);
    expect(sections[2]?.rows).toHaveLength(1);
  });

  it("pairs pressure into three rows labelled claude, codex, address with their captions", () => {
    const pressure = sections[3];
    expect(pressure?.rows).toHaveLength(3);
    const pairs = pressure?.rows as PairRow[];
    expect(pairs.map((row) => row.label)).toEqual(["claude", "codex", "address"]);
    expect(pairs.map((row) => row.caption)).toEqual([
      "/pressure, Claude",
      "$pressure, Codex",
      "/address reconciliation",
    ]);
    expect(pressure?.caption).toBe("who runs each step of loom pressure");
  });

  it("pairs models into four rows", () => {
    const models = sections[4];
    expect(models?.rows).toHaveLength(4);
    expect(models?.rows.map((row) => rowLabel(row))).toEqual([
      "standard",
      "knowledge",
      "knowledge_distill",
      "integration_verify",
    ]);
    expect(models?.caption).toBe("a stage's main agent session, by stage type");
  });

  it("projectAllowed matches whether any entry in the section scopes to project", () => {
    // Computed from the fixture's own `scopes` field, not hardcoded: every
    // section but `update` carries at least one project-scoped entry here.
    const want = sections.map((section) => ({
      section: section.section,
      projectAllowed: fixture.entries.some(
        (entry) => entry.name.startsWith(`${section.section}.`) && entry.scopes.includes("project"),
      ),
    }));
    expect(sections.map((s) => ({ section: s.section, projectAllowed: s.projectAllowed }))).toEqual(
      want,
    );
    expect(sections.find((s) => s.section === "update")?.projectAllowed).toBe(false);
  });

  it("a lone half of a pair stays a single row", () => {
    const lonely: ConfigEntry = {
      name: "made_up.foo_model",
      help: "",
      kind: { type: "enum", variants: ["a"] },
      scopes: ["user"],
      default: "a",
      user: { value: "a", set: false },
      project: null,
      effective: { value: "a", source: "default" },
    };
    const rows = sectionRows([lonely]);
    expect(rows).toHaveLength(1);
    expect(rows[0]?.rows).toEqual([{ kind: "single", entry: lonely }]);
  });
});

describe("laneState / effectiveLane", () => {
  const entry = fixture.entries.find((e) => e.name === "context.ceiling_tokens");
  if (!entry) throw new Error("fixture missing context.ceiling_tokens");

  it("builtin is readonly and not effective", () => {
    const state = laneState(entry, "builtin");
    expect(state).toEqual({
      lane: "builtin",
      value: entry.default,
      provenance: "readonly",
      effective: false,
    });
  });

  it("user is inherited (not set) and not effective", () => {
    const state = laneState(entry, "user");
    expect(state.provenance).toBe("inherited");
    expect(state.effective).toBe(false);
    expect(state.value).toBe(entry.user.value);
  });

  it("project is set and effective, matching the fixture's project value", () => {
    const state = laneState(entry, "project");
    expect(state.provenance).toBe("set");
    expect(state.effective).toBe(true);
    expect(state.value).toBe(entry.project?.value ?? null);
    expect(entry.project?.value).toBe("900000");
  });

  it("effectiveLane agrees with the project lane's effective flag", () => {
    expect(effectiveLane(entry)).toBe("project");
  });
});

describe("filterSections", () => {
  const sections = sectionRows(fixture.entries);

  it("empty query returns the same array reference", () => {
    expect(filterSections(sections, "")).toBe(sections);
  });

  it("a nonsense query returns no sections", () => {
    expect(filterSections(sections, "zzzznomatch")).toEqual([]);
  });

  it("'sonnet' keeps exactly the rows whose entries mention sonnet", () => {
    // Independently derived from the fixture, over the haystack fields the
    // spec names (name/help/default/user/project/effective values), not by
    // re-deriving `rowHaystack` itself.
    const wantLabels = sections
      .flatMap((section) => section.rows)
      .filter((row) =>
        rowEntries(row).some(
          (e) =>
            e.name.includes("sonnet") ||
            e.help.toLowerCase().includes("sonnet") ||
            e.default === "sonnet" ||
            e.user.value === "sonnet" ||
            e.project?.value === "sonnet" ||
            e.effective.value === "sonnet",
        ),
      )
      .map((row) => rowLabel(row));

    const filtered = filterSections(sections, "sonnet");
    const gotLabels = filtered.flatMap((section) => section.rows.map((row) => rowLabel(row)));
    expect(gotLabels).toEqual(wantLabels);
    expect(gotLabels.length).toBeGreaterThan(0);
  });

  it("matching is case-insensitive", () => {
    expect(filterSections(sections, "SONNET")).toEqual(filterSections(sections, "sonnet"));
  });

  it("'on' matches update.check through displayValue ('on'/'off')", () => {
    const filtered = filterSections(sections, "on");
    const labels = filtered.flatMap((section) => section.rows.map((row) => rowLabel(row)));
    expect(labels).toContain(fieldOf("update.check"));
  });
});

describe("statusFor", () => {
  const statuses: Record<string, WriteStatus> = {
    "user:models.standard_effort": { phase: "error", message: "bad value" },
  };

  it("returns the stored status for a matching scope and name", () => {
    expect(statusFor(statuses, "user", "models.standard_effort")).toEqual({
      phase: "error",
      message: "bad value",
    });
  });

  it("returns idle for a name with no recorded status", () => {
    expect(statusFor(statuses, "user", "models.standard_model")).toEqual({ phase: "idle" });
  });

  it("returns idle for the same name at the other scope", () => {
    expect(statusFor(statuses, "project", "models.standard_effort")).toEqual({ phase: "idle" });
  });
});

describe("formatValue", () => {
  it("groups an all-digit u32 string with commas", () => {
    expect(formatValue({ type: "u32" }, "800000")).toBe("800,000");
  });

  it("leaves a non-numeric u32 string unchanged", () => {
    expect(formatValue({ type: "u32" }, "abc")).toBe("abc");
  });

  it("maps bool to on/off", () => {
    expect(formatValue({ type: "bool" }, "true")).toBe("on");
    expect(formatValue({ type: "bool" }, "false")).toBe("off");
  });
});
