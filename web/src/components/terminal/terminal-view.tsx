import { cn } from "cn";
import { useAtomValue } from "jotai/react";
import { ExternalLinkIcon, KeyboardIcon } from "lucide-react";
import {
  createContext,
  useContext,
  useEffect,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactElement,
} from "react";
import { Link } from "react-router";

import type { StageSummary } from "@/api/schema";
import { CopyCommand } from "@/components/copy-command";
import { useNow } from "@/components/hooks/use-now";
import { StateLine } from "@/components/stage-heading";
import { toneClass } from "@/components/state-badge";
import { ModeKey } from "@/components/terminal/mode-key";
import {
  useTerminal,
  type EmulatorFactory,
  type EmulatorSize,
  type TerminalDeps,
  type TerminalMode,
  type TerminalPhase,
  type TerminalState,
} from "@/components/terminal/use-terminal";
import { DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { formatElapsed, stateMeta } from "@/lib/format";
import { snapshotAtom } from "@/state/atoms";

export interface TerminalViewProps {
  stage: StageSummary;
  frame: "dialog" | "page";
  factory?: EmulatorFactory;
  deps?: TerminalDeps;
}

/// What the dialog needs to know about the view inside it: the mode decides
/// whether Esc and an outside click may dismiss, the phase is mirrored as a
/// data attribute on the dialog content.
export interface TerminalFrameState {
  mode: TerminalMode;
  phase: TerminalPhase;
}

export const IDLE_FRAME: TerminalFrameState = { mode: "view", phase: "connecting" };

/// The dialog provides a reporter; the view calls it whenever mode or phase
/// change and resets it on unmount. The page frame provides nothing.
export const TerminalFrameContext = createContext<((state: TerminalFrameState) => void) | null>(
  null,
);

/// The full-page route for a stage's terminal.
export function terminalHref(id: string): string {
  return `/terminal/${encodeURIComponent(id)}`;
}

/// Delay for the header's staggered first paint, as the graph's `--i`.
function rise(index: number): CSSProperties {
  return { "--i": index } as CSSProperties;
}

/// The agent behind glass: header, the well xterm mounts into, and the
/// footer. Mode is component state, so a reload always lands in view.
export function TerminalView({ stage, frame, factory, deps }: TerminalViewProps): ReactElement {
  const [mode, setMode] = useState<TerminalMode>("view");
  const { hostRef, state, size, retry, focus } = useTerminal(stage.id, mode, factory, deps);
  const report = useContext(TerminalFrameContext);
  const terminals = useAtomValue(snapshotAtom)?.terminals ?? false;
  const phase = state.phase;

  useEffect(() => {
    report?.({ mode, phase });
  }, [report, mode, phase]);
  useEffect(() => () => report?.(IDLE_FRAME), [report]);
  useEffect(() => {
    if (mode === "control" && phase === "live") focus();
  }, [mode, phase, focus]);

  // Bubble phase, so xterm's own listener on the helper textarea has already
  // run; stopping here keeps Tab and Shift+Tab from the dialog's focus trap.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (mode === "control" && hostRef.current?.contains(event.target as Node)) {
      event.stopPropagation();
    }
  };
  const takeControl = () => {
    if (mode === "view" && phase === "live") setMode("control");
  };
  const switchable = phase === "live" || phase === "connecting";

  return (
    <div
      className={cn("terminal-frame", toneClass(stateMeta(stage.status).tone))}
      data-frame={frame}
      data-mode={mode}
      data-phase={phase}
    >
      <Head
        stage={stage}
        frame={frame}
        mode={mode}
        phase={phase}
        switchable={switchable}
        onMode={setMode}
      />
      <div className="terminal-well-wrap">
        <div
          className="terminal-well"
          data-mode={mode}
          data-phase={phase}
          onClick={takeControl}
          onKeyDown={onKeyDown}
        >
          <div className="terminal-bar" aria-hidden="true" />
          <div ref={hostRef} className="terminal-host" aria-label="Agent terminal" />
          {mode === "view" && phase === "live" && (
            <span className="terminal-takeover-cue" aria-hidden="true">
              <KeyboardIcon />
              take control
            </span>
          )}
          <Notice state={state} alive={stage.session_alive} terminals={terminals} retry={retry} />
        </div>
      </div>
      <Foot stage={stage} frame={frame} mode={mode} phase={phase} size={size} />
    </div>
  );
}

function Head({
  stage,
  frame,
  mode,
  phase,
  switchable,
  onMode,
}: {
  stage: StageSummary;
  frame: TerminalViewProps["frame"];
  mode: TerminalMode;
  phase: TerminalPhase;
  switchable: boolean;
  onMode: (mode: TerminalMode) => void;
}) {
  const base = stage.base_branch === null ? "" : ` · from ${stage.base_branch}`;
  return (
    <header className="terminal-head">
      <div className="flex min-w-0 flex-col gap-1.5">
        <span className="eyebrow rise" style={rise(0)}>
          Terminal
        </span>
        <div className="rise flex flex-wrap items-baseline gap-x-3 gap-y-1" style={rise(1)}>
          <Name stage={stage} frame={frame} />
          <span className="font-mono text-sm text-muted-foreground">{stage.id}</span>
        </div>
        <Line stage={stage} frame={frame} />
        <span className="rise text-sm text-muted-foreground" style={rise(3)}>
          loom/{stage.id} · worktree{base}
        </span>
      </div>
      <div className={cn("terminal-controls rise", frame === "dialog" && "pr-9")} style={rise(2)}>
        <ModeKey mode={mode} phase={phase} switchable={switchable} onMode={onMode} />
      </div>
    </header>
  );
}

/// The dialog's accessible title, or a plain heading on the page.
function Name({ stage, frame }: { stage: StageSummary; frame: TerminalViewProps["frame"] }) {
  const className = "text-xl font-semibold tracking-tight text-foreground";
  if (frame === "dialog") return <DialogTitle className={className}>{stage.name}</DialogTitle>;
  return <h1 className={className}>{stage.name}</h1>;
}

function Line({ stage, frame }: { stage: StageSummary; frame: TerminalViewProps["frame"] }) {
  const line = <StateLine stage={stage} className="text-foreground" />;
  return (
    <div className="rise" style={rise(2)}>
      {frame === "dialog" ? <DialogDescription asChild>{line}</DialogDescription> : line}
    </div>
  );
}

/// What the well says when there is no screen to show, or a stamp over the
/// last one: waiting, refused, ended, dropped.
function Notice({
  state,
  alive,
  terminals,
  retry,
}: {
  state: TerminalState;
  alive: boolean;
  terminals: boolean;
  retry: () => void;
}) {
  switch (state.phase) {
    case "waiting":
      return (
        <div className="terminal-notice" role="status">
          <span className="line">
            <span className="terminal-dot dot-live" aria-hidden="true" />
            waiting for the session's tmux server
          </span>
          <span>{[state.reason, "retrying in 2s"].filter(Boolean).join(" · ")}</span>
        </div>
      );
    case "refused":
      return (
        <div className="terminal-notice" role="alert">
          <span className="line">{refusedText(state.reason, terminals)}</span>
        </div>
      );
    case "ended":
      return (
        <Stamp since={state.since} word="ended" action={alive ? "Reconnect" : null} retry={retry} />
      );
    case "dropped":
      return <Stamp since={null} word="connection dropped" action="Retry" retry={retry} />;
    default:
      return null;
  }
}

/// The server's refusals that reach the browser as a handshake failure carry
/// no status; the two causes are told apart by whether terminals are on.
function refusedText(reason: string | undefined, terminals: boolean): string {
  if (reason === "handshake refused") {
    return terminals
      ? "Dashboard cookie missing — open the tokenized URL printed when the dashboard started"
      : "Start the dashboard with loom status --web --terminals";
  }
  return reason ?? "refused";
}

function Stamp({
  since,
  word,
  action,
  retry,
}: {
  since: number | null;
  word: string;
  action: string | null;
  retry: () => void;
}) {
  const now = useNow();
  const ago =
    since === null ? "" : ` · ${formatElapsed(Math.max(0, Math.floor((now - since) / 1000)))} ago`;
  return (
    <div className="terminal-stamp" role="status">
      <span className="terminal-stamp-word">
        <span className="terminal-dot" aria-hidden="true" />
        {word}
        {ago}
      </span>
      {action && (
        <button type="button" onClick={retry}>
          {action}
        </button>
      )}
    </div>
  );
}

function Foot({
  stage,
  frame,
  mode,
  phase,
  size,
}: {
  stage: StageSummary;
  frame: TerminalViewProps["frame"];
  mode: TerminalMode;
  phase: TerminalPhase;
  size: EmulatorSize | null;
}) {
  return (
    <footer className="terminal-foot">
      <Hint frame={frame} mode={mode} phase={phase} size={size} />
      <div className="flex items-center gap-4">
        <CopyCommand command={`loom attach ${stage.id}`} />
        {frame === "dialog" && (
          <Link
            to={terminalHref(stage.id)}
            className="inline-flex items-center gap-1.5 text-foreground hover:underline"
          >
            open in a tab
            <ExternalLinkIcon className="size-3.5" />
          </Link>
        )}
      </div>
    </footer>
  );
}

function Hint({
  frame,
  mode,
  phase,
  size,
}: {
  frame: TerminalViewProps["frame"];
  mode: TerminalMode;
  phase: TerminalPhase;
  size: EmulatorSize | null;
}) {
  if (phase === "ended") {
    return <span className="terminal-hint">the session ended · this is its last screen</span>;
  }
  if (mode === "control") {
    return (
      <span className="terminal-hint" data-control="">
        every key reaches the agent ·{" "}
        {frame === "dialog" ? "close with ✕ or the back button" : "release stops sending"}
      </span>
    );
  }
  const screen = size && (
    <span className="font-mono">
      screen {size.cols} × {size.rows}
    </span>
  );
  return (
    <span className="terminal-hint">
      {frame === "dialog" && (
        <>
          <Kbd>Esc</Kbd> back to details{screen && " · "}
        </>
      )}
      {screen}
    </span>
  );
}
