import { ArrowLeftIcon, ArrowRightIcon, KeyboardIcon } from "lucide-react";
import type { FocusEvent, ReactElement, ReactNode } from "react";

import type { TerminalMode, TerminalPhase } from "@/components/terminal/use-terminal";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/// The word the key shows on its left: the mode while live, else the phase.
function stateWord(mode: TerminalMode, phase: TerminalPhase): string {
  if (phase === "live") return mode === "view" ? "viewing" : "controlling";
  return phase;
}

/// One of a cell's two stacked labels; the inactive one stays in the box
/// (so the width holds) but is faded out and hidden from assistive tech.
function Label({
  active,
  kind,
  children,
}: {
  active: boolean;
  kind: "verb" | "state";
  children: ReactNode;
}): ReactElement {
  return (
    <span
      className="terminal-key-label"
      data-kind={kind}
      data-active={active}
      aria-hidden={!active}
    >
      {children}
    </span>
  );
}

/// One instrument for the header: a two-cell sliding toggle of fixed width.
/// The knob starts under the left cell, naming the take-control action, and
/// slides to the right cell when control is taken, where it turns quiet
/// since that cell now only lets go. When the phase cannot be switched it
/// turns into a plain indicator and drops both cells.
export function ModeKey({
  mode,
  phase,
  switchable,
  onMode,
}: {
  mode: TerminalMode;
  phase: TerminalPhase;
  switchable: boolean;
  onMode: (mode: TerminalMode) => void;
}): ReactElement {
  const control = mode === "control";
  const word = stateWord(mode, phase);
  // The key is the dialog's first tabbable, so the dialog's auto-focus lands
  // here on open. A tooltip that opened on focus would then sit above the
  // dialog as the topmost dismissable layer and swallow the first Esc; the
  // tooltip is a hover affordance and the switch is already named for
  // assistive tech, so focus never opens it (Radix skips a prevented event).
  const onFocus = (event: FocusEvent<HTMLButtonElement>) => event.preventDefault();
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          className="terminal-key"
          role="switch"
          aria-checked={control}
          aria-label="Take control"
          disabled={!switchable}
          data-mode={mode}
          data-phase={phase}
          onClick={() => onMode(control ? "view" : "control")}
          onFocus={onFocus}
        >
          {switchable ? (
            <>
              <span className="terminal-key-knob" aria-hidden="true" />
              <span className="terminal-key-cell">
                <Label active={!control} kind="verb">
                  take control
                  <ArrowRightIcon aria-hidden="true" />
                </Label>
                <Label active={control} kind="state">
                  <KeyboardIcon aria-hidden="true" />
                  {control ? word : "controlling"}
                </Label>
              </span>
              <span className="terminal-key-cell">
                <Label active={!control} kind="state">
                  <span className="terminal-dot" aria-hidden="true" />
                  {control ? "viewing" : word}
                </Label>
                <Label active={control} kind="verb">
                  <ArrowLeftIcon aria-hidden="true" />
                  release
                </Label>
              </span>
            </>
          ) : (
            <span className="terminal-key-state">
              <span className="terminal-dot" aria-hidden="true" />
              <span key={word} className="terminal-key-word">
                {word}
              </span>
            </span>
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent>
        {control
          ? "Back to viewing — keys stop reaching the agent"
          : "Keystrokes reach a running autonomous agent"}
      </TooltipContent>
    </Tooltip>
  );
}
