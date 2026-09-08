import { createContext, useContext } from "react";

import type { StageStatus } from "@/api/schema";

/// What the canvas is emphasising: the thread through one stage (click), or
/// every stage in one state (hovering a chip in the key).
export type Focus = { kind: "stage"; id: string } | { kind: "status"; status: StageStatus } | null;

/// Per-node and per-edge emphasis derived from the focus.
export type Emphasis = "plain" | "traced" | "dim";

export interface GraphActions {
  open: (id: string) => void;
  openTerminal: (id: string) => void;
}

export const GraphActionsContext = createContext<GraphActions>({
  open: () => {},
  openTerminal: () => {},
});

export function useGraphActions(): GraphActions {
  return useContext(GraphActionsContext);
}
