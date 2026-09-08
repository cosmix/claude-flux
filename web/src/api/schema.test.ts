import { describe, expect, it } from "vitest";

import fixtureJson from "@/api/fixtures/snapshot.json";
import { snapshotSchema, stageStatusSchema } from "@/api/schema";

describe("snapshot schema", () => {
  it("the fixture parses and terminals is false", () => {
    const snapshot = snapshotSchema.parse(fixtureJson);

    expect(snapshot.status.stages).toHaveLength(7);
    expect(stageStatusSchema.options).toHaveLength(13);
    expect(snapshot.terminals).toBe(false);
  });

  it("a frame without terminals fails to parse", () => {
    const withoutTerminals = structuredClone(fixtureJson) as Record<string, unknown>;
    delete withoutTerminals.terminals;

    expect(snapshotSchema.safeParse(withoutTerminals).success).toBe(false);
  });

  it("rejects an unknown stage status", () => {
    const invalid = structuredClone(fixtureJson) as {
      status: { stages: Array<{ status: string }> };
    };
    invalid.status.stages[0].status = "bogus";

    expect(snapshotSchema.safeParse(invalid).success).toBe(false);
  });

  it("parses a stage's cleanup_warning when present, and still parses when it's absent", () => {
    // The fixture's stages never carry the key (Rust's skip_serializing_if
    // omits it when None), so this is the only place the present case is
    // exercised for a *stage* (WebAttention entries pin the null case).
    const withWarning = structuredClone(fixtureJson) as {
      status: { stages: Array<Record<string, unknown>> };
    };
    withWarning.status.stages[0].cleanup_warning = "leftover";
    const parsed = snapshotSchema.parse(withWarning);
    expect(parsed.status.stages[0].cleanup_warning).toBe("leftover");

    const withoutKey = structuredClone(fixtureJson) as {
      status: { stages: Array<Record<string, unknown>> };
    };
    for (const stage of withoutKey.status.stages) {
      delete stage.cleanup_warning;
    }
    expect(snapshotSchema.safeParse(withoutKey).success).toBe(true);
  });

  it("accepts an omitted healthy-case notice", () => {
    expect(snapshotSchema.parse(fixtureJson).notice).toBeUndefined();
  });

  it("parses the quota block", () => {
    const quota = snapshotSchema.parse(fixtureJson).status.quota;

    expect(quota.claude?.windows.map((window) => window.kind)).toEqual(["five-hour", "seven-day"]);
    expect(quota.codex?.windows).toHaveLength(1);
    expect(quota.codex?.plan).toBe("pro");
  });

  it("rejects a status without quota", () => {
    const withoutQuota = structuredClone(fixtureJson) as {
      status: Record<string, unknown>;
    };
    delete withoutQuota.status.quota;

    expect(snapshotSchema.safeParse(withoutQuota).success).toBe(false);
  });

  it("accepts a provider with no data", () => {
    const withoutCodex = structuredClone(fixtureJson) as {
      status: { quota: { codex: unknown } };
    };
    withoutCodex.status.quota.codex = null;

    expect(snapshotSchema.safeParse(withoutCodex).success).toBe(true);
  });
});
