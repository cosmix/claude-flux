import dagre from "@dagrejs/dagre";

import type { StageStatus, StageSummary } from "@/api/schema";
import { routeEdges, type Point } from "@/lib/edge-routing";
import {
  activityText,
  contextUsage,
  mergeText,
  stateMeta,
  timeText,
  type Tone,
} from "@/lib/format";

/// Card geometry shared by the layout and the node component so the two
/// never disagree: dagre lays out the size the card will actually render at.
export const NODE_WIDTH = 248;
const NODE_BASE_HEIGHT = 92;
const NODE_FOOTER_HEIGHT = 26;
const NODE_METER_HEIGHT = 18;
const RANK_GAP = 64;
const NODE_GAP = 24;
const MARGIN = 24;

/// Whether the card shows its activity/context/time/merge row.
export function hasFooter(stage: StageSummary): boolean {
  return (
    activityText(stage) !== null ||
    contextUsage(stage) !== null ||
    (timeText(stage) ?? "") !== "" ||
    mergeText(stage) !== null
  );
}

/// The footer gets a second line for the context meter when there is one.
export function nodeHeight(stage: StageSummary): number {
  return (
    NODE_BASE_HEIGHT +
    (hasFooter(stage) ? NODE_FOOTER_HEIGHT : 0) +
    (contextUsage(stage) !== null ? NODE_METER_HEIGHT : 0)
  );
}

/// How a dependency edge is drawn, decided by the state of the stage it
/// leaves: a finished stage has woven its thread (solid), a running one is
/// still drawing it (running dash), a failed one left it torn (dashed), and
/// a stage that has not started yet has only a hairline to its dependents.
export type ThreadStyle = "solid" | "running" | "dashed" | "hairline";

export interface Thread {
  tone: Tone;
  style: ThreadStyle;
}

const THREAD_STYLE: Record<StageStatus, ThreadStyle> = {
  "waiting-for-deps": "hairline",
  queued: "hairline",
  executing: "running",
  "waiting-for-input": "running",
  "needs-handoff": "running",
  completed: "solid",
  skipped: "dashed",
  blocked: "dashed",
  "completed-with-failures": "dashed",
  "merge-conflict": "dashed",
  "merge-blocked": "dashed",
  "needs-human-review": "dashed",
  "needs-adjudication": "dashed",
};

export function threadOf(source: StageSummary): Thread {
  return { tone: stateMeta(source.status).tone, style: THREAD_STYLE[source.status] };
}

export interface LaidOutNode {
  stage: StageSummary;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface LaidOutEdge {
  source: string;
  target: string;
  points: Point[];
}

export interface RankRow {
  rank: number;
  /// Vertical centre of the row's cards.
  y: number;
}

export interface GraphLayout {
  nodes: LaidOutNode[];
  edges: LaidOutEdge[];
  /// One entry per distinct row, top to bottom.
  ranks: RankRow[];
  /// Left edge of the leftmost card, for placing the row captions.
  left: number;
}

/// Deduplicate by id, keeping the first, as the ledger does.
function uniqueStages(stages: readonly StageSummary[]): StageSummary[] {
  const seen = new Set<string>();
  const unique: StageSummary[] = [];
  for (const stage of stages) {
    if (!seen.has(stage.id)) {
      seen.add(stage.id);
      unique.push(stage);
    }
  }
  return unique;
}

/// Top-to-bottom layered layout: dependencies above, dependents below, so
/// the threads hang like a warp. Dependencies naming a stage absent from the
/// snapshot are skipped rather than drawn to a phantom.
export function layoutStages(stages: readonly StageSummary[]): GraphLayout {
  const unique = uniqueStages(stages);
  const ids = new Set(unique.map((stage) => stage.id));
  const graph = new dagre.graphlib.Graph();
  graph.setGraph({
    rankdir: "TB",
    nodesep: NODE_GAP,
    ranksep: RANK_GAP,
    marginx: MARGIN,
    marginy: MARGIN,
  });
  graph.setDefaultEdgeLabel(() => ({}));

  for (const stage of unique) {
    graph.setNode(stage.id, { width: NODE_WIDTH, height: nodeHeight(stage) });
  }
  for (const stage of unique) {
    for (const dependency of stage.dependencies) {
      if (dependency !== stage.id && ids.has(dependency)) {
        graph.setEdge(dependency, stage.id);
      }
    }
  }
  dagre.layout(graph);

  const nodes: LaidOutNode[] = unique.map((stage) => {
    const { x, y, width, height } = graph.node(stage.id);
    return { stage, x: x - width / 2, y: y - height / 2, width, height };
  });
  const edges = routeEdges(
    graph.edges().map(({ v, w }) => ({ source: v, target: w, points: graph.edge(v, w).points })),
    nodes,
  );
  const rows = [...new Set(nodes.map((node) => node.y + node.height / 2))].sort((a, b) => a - b);
  const ranks = rows.map((y, rank) => ({ rank, y }));
  const left = nodes.length === 0 ? 0 : Math.min(...nodes.map((node) => node.x));
  return { nodes, edges, ranks, left };
}

/// Every stage on the same thread as `id`: its transitive dependencies,
/// its transitive dependents, and itself.
export function lineage(stages: readonly StageSummary[], id: string): Set<string> {
  const unique = uniqueStages(stages);
  const upstream = new Map<string, string[]>();
  const downstream = new Map<string, string[]>();
  for (const stage of unique) {
    upstream.set(stage.id, stage.dependencies);
    for (const dependency of stage.dependencies) {
      downstream.set(dependency, [...(downstream.get(dependency) ?? []), stage.id]);
    }
  }
  const walk = (start: string, next: Map<string, string[]>, into: Set<string>) => {
    const pending = [start];
    while (pending.length > 0) {
      const current = pending.pop()!;
      for (const neighbour of next.get(current) ?? []) {
        if (!into.has(neighbour)) {
          into.add(neighbour);
          pending.push(neighbour);
        }
      }
    }
  };
  const thread = new Set<string>([id]);
  walk(id, upstream, thread);
  walk(id, downstream, thread);
  return thread;
}

/// Stages that list `id` as a dependency.
export function dependentsOf(stages: readonly StageSummary[], id: string): StageSummary[] {
  return uniqueStages(stages).filter((stage) => stage.dependencies.includes(id));
}
