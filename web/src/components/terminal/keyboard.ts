import type { Terminal } from "@xterm/xterm";

/** Use readline's word motions across shells with different arrow bindings. */
export function handleTerminalKey(
  event: KeyboardEvent,
  terminal: Pick<Terminal, "options" | "input">,
): boolean {
  if (
    terminal.options.disableStdin ||
    event.type !== "keydown" ||
    event.isComposing ||
    event.shiftKey ||
    event.metaKey ||
    event.getModifierState("AltGraph") ||
    !(event.ctrlKey || event.altKey)
  ) {
    return true;
  }
  const sequence =
    event.key === "ArrowLeft" ? "\x1bb" : event.key === "ArrowRight" ? "\x1bf" : null;
  if (sequence === null) return true;

  event.preventDefault();
  event.stopPropagation();
  terminal.input(sequence, true);
  return false;
}
