import { render } from "@testing-library/react";
import { Position } from "@xyflow/react";
import { describe, expect, it } from "vitest";

import fixture from "@/api/fixtures/snapshot.json";
import { snapshotSchema, type StageSummary } from "@/api/schema";
import { buildEdges } from "@/components/graph/stage-graph";
import { ThreadEdge, type ThreadEdgeType } from "@/components/graph/thread-edge";
import { layoutStages, type GraphLayout } from "@/lib/graph";
import { samplePath } from "@/test/edge-geometry";

const stages = snapshotSchema.parse(fixture).status.stages;
const stage = (id: string, dependencies: string[], template = stages[0]!): StageSummary => ({
  ...template,
  id,
  dependencies,
});

function renderThread(layout: GraphLayout, edge: ThreadEdgeType) {
  const source = layout.nodes.find((node) => node.stage.id === edge.source)!;
  const target = layout.nodes.find((node) => node.stage.id === edge.target)!;
  return render(
    <svg>
      <ThreadEdge
        id={edge.id}
        source={edge.source}
        target={edge.target}
        data={edge.data}
        sourceX={source.x + source.width / 2}
        sourceY={source.y + source.height + 5}
        targetX={target.x + target.width / 2}
        targetY={target.y - 5}
        sourcePosition={Position.Bottom}
        targetPosition={Position.Top}
      />
    </svg>,
  );
}

function expectClearThreads(input: StageSummary[]) {
  const layout = layoutStages(input);
  const edges = buildEdges(layout, input, null);
  expect(edges.length).toBeGreaterThan(0);
  for (const edge of edges) {
    const { container, unmount } = renderThread(layout, edge);
    const path = container.querySelector(".thread-base")!.getAttribute("d")!;
    const samples = samplePath(path);
    expect(samples.length).toBeGreaterThan(1);
    const source = layout.nodes.find((node) => node.stage.id === edge.source)!;
    const target = layout.nodes.find((node) => node.stage.id === edge.target)!;
    expect(samples[0]).toEqual({ x: source.x + source.width / 2, y: source.y + source.height + 5 });
    expect(samples.at(-1)).toEqual({ x: target.x + target.width / 2, y: target.y - 5 });
    for (const node of layout.nodes) {
      const hits = samples.filter(
        ({ x, y }) =>
          x > node.x - 2 &&
          x < node.x + node.width + 2 &&
          y > node.y - 2 &&
          y < node.y + node.height + 2,
      );
      expect(hits.length, `${edge.id} crosses ${node.stage.id}`).toBe(0);
    }
    const running = container.querySelector(".thread-run");
    if (edge.data?.thread.style === "running") expect(running?.getAttribute("d")).toBe(path);
    unmount();
  }
}

describe("ThreadEdge routing", () => {
  it("routes a dependency skipping a level around the intermediate card", () => {
    expectClearThreads([stage("a", []), stage("b", ["a"]), stage("c", ["a", "b"])]);
  });

  it("clears multiple skipped levels, branches, and cards of different heights", () => {
    expectClearThreads([
      stage("a", [], stages[1]),
      stage("b", ["a"]),
      stage("c", ["a"], stages[1]),
      stage("d", ["b", "c"]),
      stage("e", ["a", "d"], stages[1]),
      stage("f", ["b", "e"]),
    ]);
  });

  it("clears all cards in the dashboard fixture", () => {
    expectClearThreads(stages);
  });

  it("clears taller siblings when fanning out between adjacent ranks", () => {
    expectClearThreads([
      stage("a", []),
      stage("b", [], stages[1]),
      stage("c", []),
      stage("d", ["a", "b", "c"]),
      stage("e", ["a", "b", "c"], stages[1]),
      stage("f", ["a", "b", "c"]),
    ]);
  });

  it("updates routes when live state changes card heights", () => {
    const dependencies = [[], ["0"], ["0"], ["0", "1", "2"]];
    for (const template of stages) {
      expectClearThreads(dependencies.map((deps, i) => stage(`${i}`, deps, template)));
    }
  });
});
