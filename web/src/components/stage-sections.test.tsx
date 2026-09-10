import { describe, expect, it } from "vitest";

import fixtureJson from "@/api/fixtures/snapshot.json";
import { snapshotSchema, type StageSummary } from "@/api/schema";
import { stageSections } from "@/components/stage-sections";

const fixture = snapshotSchema.parse(fixtureJson);

function fixtureStage(overrides: Partial<StageSummary> = {}): StageSummary {
  const stage = fixture.status.stages.find((candidate) => candidate.id === "server");
  if (!stage) throw new Error("fixture server stage is missing");
  return { ...stage, ...overrides };
}

function sessionLabels(stage: StageSummary): string[] {
  const session = stageSections(stage, null).find((section) => section.title === "session");
  return session?.rows.map((entry) => entry.label) ?? [];
}

describe("stage session details", () => {
  it("omits the generated tool activity that repeats last tool", () => {
    const stage = fixtureStage({ last_tool: "Bash", last_activity: "Tool executed: Bash" });

    expect(sessionLabels(stage)).toContain("last tool");
    expect(sessionLabels(stage)).not.toContain("last activity");
  });

  it("keeps a non-tool heartbeat activity", () => {
    const stage = fixtureStage({ last_tool: null, last_activity: "Session started" });

    expect(sessionLabels(stage)).toContain("last activity");
  });
});
