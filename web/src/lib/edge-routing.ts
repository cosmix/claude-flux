import type { LaidOutEdge, LaidOutNode } from "@/lib/graph";

export interface Point {
  x: number;
  y: number;
}

interface RowBounds {
  top: number;
  bottom: number;
}

const CLEARANCE = 8;
const centerY = (node: LaidOutNode) => node.y + node.height / 2;

function rowBounds(nodes: LaidOutNode[]): Map<number, RowBounds> {
  const rows = new Map<number, RowBounds>();
  for (const node of nodes) {
    const y = centerY(node);
    const row = rows.get(y);
    rows.set(y, {
      top: Math.min(row?.top ?? Infinity, node.y),
      bottom: Math.max(row?.bottom ?? -Infinity, node.y + node.height),
    });
  }
  return rows;
}

// Dagre reserves a lane beside the cards for each edge crossing a rank.
// Keep that lane for the full height of the row, including taller siblings;
// changing x only in the empty rank gaps keeps curves clear of every card.
export function routeEdges(edges: LaidOutEdge[], nodes: LaidOutNode[]): LaidOutEdge[] {
  const rows = rowBounds(nodes);
  const byId = new Map(nodes.map((node) => [node.stage.id, node]));
  return edges.map((edge) => {
    const source = byId.get(edge.source)!;
    const target = byId.get(edge.target)!;
    const points: Point[] = [
      { x: source.x + source.width / 2, y: rows.get(centerY(source))!.bottom + CLEARANCE },
    ];
    for (const point of edge.points.slice(1, -1)) {
      const row = rows.get(point.y);
      if (row) {
        points.push(
          { x: point.x, y: row.top - CLEARANCE },
          { x: point.x, y: row.bottom + CLEARANCE },
        );
      }
    }
    points.push({ x: target.x + target.width / 2, y: rows.get(centerY(target))!.top - CLEARANCE });
    return { ...edge, points };
  });
}

// Each cubic stays inside its rank gap. A single spline over the entire
// route can cut across the very cards that Dagre's waypoints avoid.
export function threadPath(points: readonly Point[], source: Point, target: Point): string {
  let path = `M ${source.x},${source.y}`;
  let previous = source;
  for (const point of [...points, target]) {
    if (point.x === previous.x) {
      path += ` L ${point.x},${point.y}`;
    } else {
      const middle = (previous.y + point.y) / 2;
      path += ` C ${previous.x},${middle} ${point.x},${middle} ${point.x},${point.y}`;
    }
    previous = point;
  }
  return path;
}
