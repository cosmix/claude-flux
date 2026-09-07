import { BaseEdge, type Edge, type EdgeProps } from "@xyflow/react";

import type { Emphasis } from "@/components/graph/context";
import { toneClass } from "@/components/state-badge";
import { threadPath, type Point } from "@/lib/edge-routing";
import type { Thread } from "@/lib/graph";

export interface ThreadEdgeData extends Record<string, unknown> {
  thread: Thread;
  emphasis: Emphasis;
  points: Point[];
}

export type ThreadEdgeType = Edge<ThreadEdgeData, "thread">;

/// A dependency drawn through its layout route, from the dependency's bottom
/// anchor to the dependent's top one. Colour and dash come from the source state;
/// a running thread carries a second, moving dash over the base line.
export function ThreadEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  data,
}: EdgeProps<ThreadEdgeType>) {
  if (!data) return null;
  const path = threadPath(data.points, { x: sourceX, y: sourceY }, { x: targetX, y: targetY });
  const { thread, emphasis } = data;
  return (
    <g className={toneClass(thread.tone)} data-thread={thread.style} data-emphasis={emphasis}>
      <BaseEdge id={id} path={path} className="thread-base" interactionWidth={0} />
      {thread.style === "running" && <path d={path} className="thread-run" />}
    </g>
  );
}
