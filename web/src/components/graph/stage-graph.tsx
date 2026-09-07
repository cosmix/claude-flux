import {
  Background,
  BackgroundVariant,
  Controls,
  Panel,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type EdgeTypes,
  type NodeMouseHandler,
  type NodeTypes,
} from "@xyflow/react";
import { useAtomValue } from "jotai/react";
import { WorkflowIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";

import { EmptyState } from "@/aurora-ui/feedback/EmptyState";

import type { StageStatus, StageSummary } from "@/api/schema";
import { GraphActionsContext, type Emphasis, type Focus } from "@/components/graph/context";
import { RankNode, type RankNodeType } from "@/components/graph/rank-node";
import { StageNode, type StageNodeType } from "@/components/graph/stage-node";
import { StateKey } from "@/components/graph/state-key";
import { ThreadEdge, type ThreadEdgeType } from "@/components/graph/thread-edge";
import { useOpenStage } from "@/components/stage-modal";
import { Kbd } from "@/components/ui/kbd";
import { Skeleton } from "@/components/ui/skeleton";
import { layoutStages, lineage, threadOf, type GraphLayout } from "@/lib/graph";
import { snapshotAtom } from "@/state/atoms";

type CanvasNode = StageNodeType | RankNodeType;

const nodeTypes: NodeTypes = { stage: StageNode, rank: RankNode };
const edgeTypes: EdgeTypes = { thread: ThreadEdge };
const FIT = { padding: 0.12, maxZoom: 1.3 };
const CAPTION_WIDTH = 64;
const CAPTION_GAP = 14;
const CAPTION_HEIGHT = 16;

/// The plan as a sheet: stages laid out left to right by dependency, joined
/// by threads. Click traces a stage's thread, double-click opens it.
export function StageGraph() {
  const snapshot = useAtomValue(snapshotAtom);
  if (snapshot === null) {
    return <Skeleton aria-busy="true" aria-label="loading graph" className="h-full rounded-none" />;
  }
  const stages = snapshot.status.stages;
  if (stages.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <EmptyState
          icon={WorkflowIcon}
          title="No stages in this workspace"
          description="Run a plan with loom run and its stages will be drawn here."
          variant="bare"
          tone="muted"
        />
      </div>
    );
  }
  return (
    <ReactFlowProvider>
      <Canvas stages={stages} />
    </ReactFlowProvider>
  );
}

function Canvas({ stages }: { stages: StageSummary[] }) {
  const open = useOpenStage();
  const { fitView } = useReactFlow();
  const [pinned, setPinned] = useState<Focus>(null);
  const [hovered, setHovered] = useState<Focus>(null);
  const focus = hovered ?? pinned;

  const layout = useMemo(() => layoutStages(stages), [stages]);
  const nodes = useMemo(() => buildNodes(layout, stages, focus), [layout, stages, focus]);
  const edges = useMemo(() => buildEdges(layout, stages, focus), [layout, stages, focus]);
  const actions = useMemo(() => ({ open }), [open]);

  // Refit when stages appear or vanish; the `fitView` prop covers first paint.
  const count = layout.nodes.length;
  const previousCount = useRef(count);
  useEffect(() => {
    if (previousCount.current !== count) {
      previousCount.current = count;
      void fitView({ ...FIT, duration: 320 });
    }
  }, [count, fitView]);

  const onNodeClick: NodeMouseHandler<CanvasNode> = useCallback((_, node) => {
    if (node.type !== "stage") return;
    setPinned((current) =>
      current?.kind === "stage" && current.id === node.id ? null : { kind: "stage", id: node.id },
    );
  }, []);
  const onNodeDoubleClick: NodeMouseHandler<CanvasNode> = useCallback(
    (_, node) => {
      if (node.type === "stage") open(node.id);
    },
    [open],
  );
  const onPaneClick = useCallback(() => setPinned(null), []);
  const onPinStatus = useCallback((status: StageStatus) => {
    setPinned((current) =>
      current?.kind === "status" && current.status === status ? null : { kind: "status", status },
    );
  }, []);
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Enter") {
      const id = (event.target as HTMLElement).closest<HTMLElement>(".react-flow__node-stage")
        ?.dataset.id;
      if (id) {
        event.preventDefault();
        open(id);
      }
    } else if (event.key === "Escape") {
      setPinned(null);
    }
  };

  return (
    <GraphActionsContext.Provider value={actions}>
      <div className="sheet h-full" onKeyDown={onKeyDown}>
        <ReactFlow<CanvasNode, ThreadEdgeType>
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          edgeTypes={edgeTypes}
          fitView
          fitViewOptions={FIT}
          minZoom={0.2}
          maxZoom={1.6}
          nodesDraggable={false}
          nodesConnectable={false}
          edgesFocusable={false}
          selectNodesOnDrag={false}
          zoomOnDoubleClick={false}
          onNodeClick={onNodeClick}
          onNodeDoubleClick={onNodeDoubleClick}
          onPaneClick={onPaneClick}
        >
          <Background
            id="fine"
            variant={BackgroundVariant.Lines}
            gap={24}
            lineWidth={1}
            color="var(--sheet-rule)"
          />
          <Background
            id="coarse"
            variant={BackgroundVariant.Lines}
            gap={120}
            lineWidth={1}
            color="var(--sheet-rule-strong)"
          />
          <Controls showInteractive={false} position="bottom-left" />
          <Panel position="top-left">
            <StateKey stages={stages} pinned={pinned} onHover={setHovered} onPin={onPinStatus} />
          </Panel>
          <Panel position="top-right" className="sheet-hint hidden sm:block">
            click traces a thread · <Kbd>dbl-click</Kbd> opens a stage
          </Panel>
        </ReactFlow>
      </div>
    </GraphActionsContext.Provider>
  );
}

/// The ids the focus emphasises, or null when nothing is focused. A pinned
/// stage that has since left the snapshot focuses nothing, and neither does
/// a hovered status chip that has lost its last stage and unmounted under
/// the pointer without firing its mouse-leave.
function tracedIds(stages: readonly StageSummary[], focus: Focus): Set<string> | null {
  if (focus === null) return null;
  if (focus.kind === "status") {
    const traced = stages.filter((stage) => stage.status === focus.status).map((s) => s.id);
    return traced.length === 0 ? null : new Set(traced);
  }
  if (!stages.some((stage) => stage.id === focus.id)) return null;
  return lineage(stages, focus.id);
}

export function buildNodes(
  layout: GraphLayout,
  stages: readonly StageSummary[],
  focus: Focus,
): CanvasNode[] {
  const traced = tracedIds(stages, focus);
  const emphasis = (id: string): Emphasis =>
    traced === null ? "plain" : traced.has(id) ? "traced" : "dim";
  // Reveal order sweeps top to bottom, left to right.
  const sweep = [...layout.nodes].sort((a, b) => a.y - b.y || a.x - b.x);
  const index = new Map(sweep.map((node, position) => [node.stage.id, position]));

  // Every node declares the size it renders at: React Flow drops a node's
  // handle bounds when a node object arrives without `measured`, and it draws
  // no edge without handle bounds on both ends — and every snapshot frame
  // rebuilds these objects.
  const captions: RankNodeType[] = layout.ranks.map(({ rank, y }) => ({
    id: `rank:${rank}`,
    type: "rank",
    position: { x: layout.left - CAPTION_WIDTH - CAPTION_GAP, y: y - CAPTION_HEIGHT / 2 },
    data: { rank },
    width: CAPTION_WIDTH,
    height: CAPTION_HEIGHT,
    measured: { width: CAPTION_WIDTH, height: CAPTION_HEIGHT },
    selectable: false,
    focusable: false,
    draggable: false,
    connectable: false,
    zIndex: -1,
  }));
  const cards: StageNodeType[] = layout.nodes.map(({ stage, x, y, width, height }) => ({
    id: stage.id,
    type: "stage",
    position: { x, y },
    width,
    height,
    measured: { width, height },
    data: { stage, index: index.get(stage.id) ?? 0, emphasis: emphasis(stage.id) },
  }));
  return [...captions, ...cards];
}

export function buildEdges(
  layout: GraphLayout,
  stages: readonly StageSummary[],
  focus: Focus,
): ThreadEdgeType[] {
  const traced = tracedIds(stages, focus);
  const byId = new Map(layout.nodes.map((node) => [node.stage.id, node.stage]));
  // A stage's thread includes only edges with both ends on it; a state focus
  // keeps every thread touching one of its stages.
  const emphasis = (source: string, target: string): Emphasis => {
    if (traced === null) return "plain";
    const on =
      focus?.kind === "stage"
        ? traced.has(source) && traced.has(target)
        : traced.has(source) || traced.has(target);
    return on ? "traced" : "dim";
  };
  return layout.edges.map(({ source, target, points }) => ({
    id: `${source}->${target}`,
    source,
    target,
    type: "thread",
    data: { thread: threadOf(byId.get(source)!), emphasis: emphasis(source, target), points },
  }));
}
