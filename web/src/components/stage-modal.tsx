import { cn } from "cn";
import { useAtomValue } from "jotai/react";
import { ExternalLinkIcon, TerminalIcon } from "lucide-react";
import { useCallback, useState, type KeyboardEvent } from "react";
import { Link, useSearchParams } from "react-router";

import type { StageSummary } from "@/api/schema";
import { HazardHeader } from "@/aurora-ui/feedback/HazardPanel";
import { AttentionBody, attentionDetail, attentionHazard } from "@/components/attention-panel";
import { CopyCommand } from "@/components/copy-command";
import { StateLine, ThreadRows } from "@/components/stage-heading";
import { stageHref } from "@/components/stage-href";
import { StageSectionGrid } from "@/components/stage-sections";
import { toneClass } from "@/components/state-badge";
import { terminalGate } from "@/components/terminal/terminal-glyph";
import {
  IDLE_FRAME,
  TerminalFrameContext,
  TerminalView,
  type TerminalFrameState,
} from "@/components/terminal/terminal-view";
import type { EmulatorFactory, TerminalDeps } from "@/components/terminal/use-terminal";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { stateMeta } from "@/lib/format";
import { attentionAtom, orderedStagesAtom, selectStage, snapshotAtom } from "@/state/atoms";

/// The query parameter naming the stage the modal shows, so an open modal
/// has a URL and the back button closes it.
export const STAGE_PARAM = "stage";
/// Which face of the stage the modal shows; absent means the details.
export const STAGE_VIEW_PARAM = "view";
const TERMINAL_VIEW = "terminal";

export function useOpenStage(): (id: string) => void {
  const [, setParams] = useSearchParams();
  return useCallback(
    (id: string) =>
      setParams((params) => {
        const next = new URLSearchParams(params);
        next.set(STAGE_PARAM, id);
        return next;
      }),
    [setParams],
  );
}

/// Opens the stage's terminal view: `?stage=<id>&view=terminal`. Back returns
/// to the details, back again closes.
export function useOpenTerminal(): (id: string) => void {
  const [, setParams] = useSearchParams();
  return useCallback(
    (id: string) =>
      setParams((params) => {
        const next = new URLSearchParams(params);
        next.set(STAGE_PARAM, id);
        next.set(STAGE_VIEW_PARAM, TERMINAL_VIEW);
        return next;
      }),
    [setParams],
  );
}

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

/// The stage page's content in a dialog, opened from any view with
/// `?stage=<id>`; upstream and downstream chips move the dialog along the
/// thread without closing it. With `&view=terminal` the dialog becomes the
/// window onto the agent: Esc goes back to the details in view mode and to
/// the agent in control mode, where only the close button or the browser's
/// back button leave.
export interface StageModalProps {
  terminalFactory?: EmulatorFactory;
  terminalDeps?: TerminalDeps;
}

export function StageModal({ terminalFactory, terminalDeps }: StageModalProps = {}) {
  const [params, setParams] = useSearchParams();
  const id = params.get(STAGE_PARAM);
  const terminal = params.get(STAGE_VIEW_PARAM) === TERMINAL_VIEW;
  const snapshot = useAtomValue(snapshotAtom);
  const openTerminal = useOpenTerminal();
  const [frame, setFrame] = useState<TerminalFrameState>(IDLE_FRAME);
  const stage = id === null ? undefined : selectStage(snapshot, id);
  const terminalsEnabled = snapshot?.terminals ?? false;
  const gate = stage === undefined ? null : terminalGate(terminalsEnabled, stage);
  const nativeBackend =
    stage !== undefined && terminalsEnabled && stage.session_backend === "native";
  const showTerminal = terminal && stage !== undefined;
  const controlling = showTerminal && frame.mode === "control";

  const close = () =>
    setParams((current) => {
      const next = new URLSearchParams(current);
      next.delete(STAGE_PARAM);
      next.delete(STAGE_VIEW_PARAM);
      return next;
    });
  const toDetails = () =>
    setParams((current) => {
      const next = new URLSearchParams(current);
      next.delete(STAGE_VIEW_PARAM);
      return next;
    });
  const guard = (event: Event) => {
    if (controlling) event.preventDefault();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (showTerminal || event.key !== "t" || event.altKey || event.ctrlKey || event.metaKey) return;
    if (isTyping(event.target) || stage === undefined || gate !== null) return;
    event.preventDefault();
    openTerminal(stage.id);
  };

  return (
    <Dialog
      open={id !== null}
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <DialogContent
        className={cn(
          "stage-modal gap-0 overflow-hidden p-0",
          showTerminal
            ? "terminal-dialog h-[min(86dvh,860px)] grid-rows-[auto_1fr_auto] sm:max-w-[min(96vw,1280px)]"
            : "sm:max-w-3xl",
        )}
        data-mode={showTerminal ? frame.mode : undefined}
        data-phase={showTerminal ? frame.phase : undefined}
        onKeyDown={onKeyDown}
        onEscapeKeyDown={(event) => {
          if (!showTerminal) return;
          event.preventDefault();
          if (frame.mode === "view") toDetails();
        }}
        onInteractOutside={guard}
        onPointerDownOutside={guard}
      >
        {id !== null && stage === undefined && <Missing id={id} />}
        {stage && !terminal && (
          <Body stage={stage} gate={gate} nativeBackend={nativeBackend} onTerminal={openTerminal} />
        )}
        {stage && terminal && (
          <TerminalFrameContext.Provider value={setFrame}>
            <TerminalView
              stage={stage}
              frame="dialog"
              factory={terminalFactory}
              deps={terminalDeps}
            />
          </TerminalFrameContext.Provider>
        )}
      </DialogContent>
    </Dialog>
  );
}

function Body({
  stage,
  gate,
  nativeBackend,
  onTerminal,
}: {
  stage: StageSummary;
  gate: string | null;
  nativeBackend: boolean;
  onTerminal: (id: string) => void;
}) {
  const open = useOpenStage();
  const ordered = useAtomValue(orderedStagesAtom);
  const attention = useAtomValue(attentionAtom).find((entry) => entry.id === stage.id);
  const level = ordered.find((entry) => entry.stage.id === stage.id)?.level ?? null;
  const tone = stateMeta(stage.status).tone;

  const detail = attention && attentionDetail(attention);
  return (
    <>
      {attention && (
        <HazardHeader
          tone={attentionHazard(attention)}
          title={attention.label}
          className="pr-12 sm:pr-12"
          action={
            detail && (
              <span className="max-w-xs truncate text-xs text-muted-foreground" title={detail}>
                {detail}
              </span>
            )
          }
        />
      )}
      <DialogHeader className={cn("stage-modal-head gap-2 p-5 pr-12", toneClass(tone))}>
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <DialogTitle className="text-xl font-semibold tracking-tight text-foreground">
            {stage.name}
          </DialogTitle>
          <span className="font-mono text-sm text-muted-foreground">{stage.id}</span>
          <TerminalButton
            reason={gate}
            nativeBackend={nativeBackend}
            onOpen={() => onTerminal(stage.id)}
          />
        </div>
        <DialogDescription asChild>
          <StateLine stage={stage} className="text-foreground" />
        </DialogDescription>
        <ThreadRows stage={stage} pick={open} className="mt-1" />
      </DialogHeader>
      <div className="max-h-[62dvh] overflow-y-auto p-5">
        <div className="flex flex-col gap-4">
          {attention && (
            <div className="flex flex-col gap-2.5 rounded-lg border border-hairline bg-card p-4 text-sm">
              <h2 className="eyebrow">what to do</h2>
              <AttentionBody entry={attention} />
            </div>
          )}
          <StageSectionGrid stage={stage} level={level} />
        </div>
      </div>
      <footer className="flex items-center justify-between gap-3 border-t border-hairline bg-muted/40 px-5 py-2.5 text-xs text-muted-foreground">
        <span>
          <Kbd>Esc</Kbd> closes
        </span>
        <Link
          to={stageHref(stage.id)}
          className="inline-flex items-center gap-1.5 text-foreground hover:underline"
        >
          open as a page
          <ExternalLinkIcon className="size-3.5" />
        </Link>
      </footer>
    </>
  );
}

/// The primary way into the terminal. Disabled with the reason on hover when
/// the stage cannot be attached to; the span carries the tooltip because a
/// disabled button emits no pointer events.
function TerminalButton({
  reason,
  nativeBackend,
  onOpen,
}: {
  reason: string | null;
  nativeBackend: boolean;
  onOpen: () => void;
}) {
  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="ml-auto inline-flex self-center">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={reason !== null}
              onClick={onOpen}
              aria-label="open terminal"
            >
              <TerminalIcon />
              Terminal
              <Kbd>t</Kbd>
            </Button>
          </span>
        </TooltipTrigger>
        <TooltipContent className="max-w-xs text-left">
          {reason ?? "Watch the session, or take control of it"}
        </TooltipContent>
      </Tooltip>
      {nativeBackend && <CopyCommand command="loom run --backend tmux" className="self-center" />}
    </>
  );
}

function Missing({ id }: { id: string }) {
  return (
    <DialogHeader className="p-5">
      <DialogTitle>No such stage</DialogTitle>
      <DialogDescription>
        No stage named <code className="rounded bg-muted px-1 py-0.5 text-xs">{id}</code> in this
        workspace.
      </DialogDescription>
    </DialogHeader>
  );
}
