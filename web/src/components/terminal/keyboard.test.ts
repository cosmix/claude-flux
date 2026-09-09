import { describe, expect, it, vi } from "vitest";

import { handleTerminalKey } from "./keyboard";

describe("terminal word navigation", () => {
  it.each([
    ["ArrowLeft", { ctrlKey: true }, "\x1bb"],
    ["ArrowRight", { ctrlKey: true }, "\x1bf"],
    ["ArrowLeft", { altKey: true }, "\x1bb"],
    ["ArrowRight", { altKey: true }, "\x1bf"],
    ["ArrowLeft", { ctrlKey: true, altKey: true }, "\x1bb"],
  ])("sends %s with %o as a word motion", (key, modifiers, sequence) => {
    const terminal = { options: { disableStdin: false }, input: vi.fn() };
    const event = new KeyboardEvent("keydown", { key, ...modifiers, cancelable: true });

    expect(handleTerminalKey(event, terminal)).toBe(false);
    expect(terminal.input).toHaveBeenCalledExactlyOnceWith(sequence, true);
    expect(event.defaultPrevented).toBe(true);
  });

  it.each([
    { key: "ArrowLeft" },
    { key: "ArrowUp", altKey: true },
    { key: "ArrowDown", ctrlKey: true },
    { key: "ArrowLeft", ctrlKey: true, shiftKey: true },
    { key: "ArrowLeft", altKey: true, metaKey: true },
    { key: "ArrowLeft", altKey: true, isComposing: true },
    { key: "ArrowLeft", ctrlKey: true, modifierAltGraph: true },
    { key: "b", altKey: true },
    { key: "c", ctrlKey: true },
  ])("leaves other key combinations to xterm: %o", (init) => {
    const terminal = { options: {}, input: vi.fn() };
    const event = new KeyboardEvent("keydown", { ...init, cancelable: true });

    expect(handleTerminalKey(event, terminal)).toBe(true);
    expect(terminal.input).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("does not send a second motion on keyup", () => {
    const terminal = { options: {}, input: vi.fn() };
    const event = new KeyboardEvent("keyup", { key: "ArrowLeft", altKey: true });

    expect(handleTerminalKey(event, terminal)).toBe(true);
    expect(terminal.input).not.toHaveBeenCalled();
  });

  it("preserves read-only mode", () => {
    const terminal = { options: { disableStdin: true }, input: vi.fn() };
    const event = new KeyboardEvent("keydown", { key: "ArrowRight", ctrlKey: true });

    expect(handleTerminalKey(event, terminal)).toBe(true);
    expect(terminal.input).not.toHaveBeenCalled();
  });
});
