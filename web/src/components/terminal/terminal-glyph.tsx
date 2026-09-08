import { cn } from "cn";
import { useAtomValue } from "jotai/react";

import type { StageSummary } from "@/api/schema";
import { toneClass } from "@/components/state-badge";
import { stateMeta } from "@/lib/format";
import { snapshotAtom } from "@/state/atoms";

/// Why a stage cannot open a terminal, or `null` when it can: the dashboard
/// was started without `--terminals`, the session runs in a native terminal
/// window, or there is no live session to attach to.
export function terminalGate(terminals: boolean, stage: StageSummary): string | null {
  if (!terminals) return "Start the dashboard with loom status --web --terminals";
  if (stage.session_backend === "native") {
    return "This stage's session runs in a native terminal window. Terminals need loom run --backend tmux.";
  }
  if (stage.session_backend !== "tmux" || !stage.session_alive) return "No live session";
  return null;
}

/// The reason from `terminalGate` for the current snapshot.
export function useTerminalGate(stage: StageSummary): string | null {
  const snapshot = useAtomValue(snapshotAtom);
  return terminalGate(snapshot?.terminals ?? false, stage);
}

/// `>_` in the stage's tone: the one mark on a card or row that says the
/// agent itself can be seen. Rendered only while a terminal can be opened.
export function TerminalGlyph({
  stage,
  onOpen,
  className,
}: {
  stage: StageSummary;
  onOpen: (id: string) => void;
  className?: string;
}) {
  const reason = useTerminalGate(stage);
  if (reason !== null) return null;
  return (
    <button
      type="button"
      className={cn("terminal-glyph", toneClass(stateMeta(stage.status).tone), className)}
      aria-label={`open terminal for ${stage.name}`}
      title="Open terminal"
      onClick={(event) => {
        event.stopPropagation();
        onOpen(stage.id);
      }}
    >
      {">_"}
    </button>
  );
}
