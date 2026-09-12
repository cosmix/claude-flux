import { ArrowRightIcon, CornerDownLeftIcon, KeyboardIcon } from "lucide-react";
import type { FocusEvent, ReactElement } from "react";

import type { TerminalMode, TerminalPhase } from "@/components/terminal/use-terminal";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/// The word the key shows on its left: the mode while live, else the phase.
function stateWord(mode: TerminalMode, phase: TerminalPhase): string {
  if (phase === "live") return mode === "view" ? "viewing" : "controlling";
  return phase;
}

/// One instrument for the header: a switch that reads the connection on its
/// left and names what a press does on its right. When the phase cannot be
/// switched it turns into a plain indicator and drops the verb.
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
  const armed = control && switchable;
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
          <span className="terminal-key-state">
            {armed ? (
              <KeyboardIcon aria-hidden="true" />
            ) : (
              <span className="terminal-dot" aria-hidden="true" />
            )}
            <span key={word} className="terminal-key-word">
              {word}
            </span>
          </span>
          {switchable && (
            <span className="terminal-key-verb">
              {control ? "release" : "take control"}
              {control ? (
                <CornerDownLeftIcon aria-hidden="true" />
              ) : (
                <ArrowRightIcon aria-hidden="true" />
              )}
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
