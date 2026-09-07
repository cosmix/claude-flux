import { describe, expect, it } from "vitest";

import fixture from "@/api/fixtures/snapshot.json";
import { snapshotSchema, type StageSummary } from "@/api/schema";
import { dependentsOf, layoutStages, lineage, nodeHeight, threadOf } from "@/lib/graph";

const stages = snapshotSchema.parse(fixture).status.stages;
const byId = (id: string): StageSummary => stages.find((stage) => stage.id === id)!;

describe("layoutStages", () => {
  const layout = layoutStages(stages);

  it("places every stage once, dependencies above dependents", () => {
    expect(layout.nodes.map((node) => node.stage.id).sort()).toEqual(
      stages.map((stage) => stage.id).sort(),
    );
    const y = new Map(layout.nodes.map((node) => [node.stage.id, node.y]));
    for (const { source, target } of layout.edges) {
      expect(y.get(source)!).toBeLessThan(y.get(target)!);
    }
  });

  it("draws one thread per dependency present in the snapshot", () => {
    const expected = stages.flatMap((stage) =>
      stage.dependencies.map((dependency) => `${dependency}->${stage.id}`),
    );
    expect(layout.edges.map((edge) => `${edge.source}->${edge.target}`).sort()).toEqual(
      expected.sort(),
    );
  });

  it("skips a dependency that names no stage, and a duplicate id", () => {
    const phantom = { ...byId("design"), id: "ghost", dependencies: ["nowhere", "server"] };
    const twice = layoutStages([...stages, phantom, byId("server")]);
    expect(twice.nodes.filter((node) => node.stage.id === "server").length).toBe(1);
    expect(twice.edges.filter((edge) => edge.target === "ghost")).toEqual([
      expect.objectContaining({ source: "server", target: "ghost" }),
    ]);
  });

  it("lays out rows in dependency order, captioned from the left margin", () => {
    expect(layout.ranks.map((row) => row.rank)).toEqual([0, 1, 2, 3, 4]);
    const rows = layout.ranks.map((row) => row.y);
    expect([...rows].sort((a, b) => a - b)).toEqual(rows);
    expect(layout.left).toBe(Math.min(...layout.nodes.map((node) => node.x)));
    expect(layout.nodes.every((node) => node.height === nodeHeight(node.stage))).toBe(true);
  });

  it("gives an empty plan an empty sheet", () => {
    expect(layoutStages([])).toEqual({ nodes: [], edges: [], ranks: [], left: 0 });
  });
});

describe("nodeHeight", () => {
  it("adds the footer row only when the card has something live to show", () => {
    expect(nodeHeight(byId("client"))).toBeGreaterThan(nodeHeight(byId("design")));
    expect(nodeHeight(byId("design"))).toBe(nodeHeight(byId("knowledge-distill")));
  });

  it("adds a meter line to a card with a context reading", () => {
    expect(nodeHeight(byId("server"))).toBeGreaterThan(nodeHeight(byId("client")));
  });
});

describe("threadOf", () => {
  it("styles the thread by the state of the stage it leaves", () => {
    expect(threadOf(byId("knowledge-bootstrap"))).toEqual({ tone: "completed", style: "solid" });
    expect(threadOf(byId("server"))).toEqual({ tone: "executing", style: "running" });
    expect(threadOf(byId("client"))).toEqual({ tone: "blocked", style: "dashed" });
    expect(threadOf(byId("docs"))).toEqual({ tone: "warning", style: "dashed" });
    expect(threadOf(byId("design"))).toEqual({ tone: "pending", style: "hairline" });
  });
});

describe("lineage", () => {
  it("walks both ways along the thread", () => {
    expect([...lineage(stages, "design")].sort()).toEqual([
      "client",
      "design",
      "integration-verify",
      "knowledge-bootstrap",
      "knowledge-distill",
      "server",
    ]);
    expect([...lineage(stages, "knowledge-bootstrap")].sort()).toEqual(
      stages.map((stage) => stage.id).sort(),
    );
    expect([...lineage(stages, "missing")]).toEqual(["missing"]);
  });
});

describe("dependentsOf", () => {
  it("lists the stages that depend on the given one", () => {
    expect(dependentsOf(stages, "knowledge-bootstrap").map((stage) => stage.id)).toEqual([
      "server",
      "client",
      "docs",
    ]);
    expect(dependentsOf(stages, "knowledge-distill")).toEqual([]);
  });
});
