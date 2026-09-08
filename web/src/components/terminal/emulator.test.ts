import { describe, expect, it } from "vitest";

import { terminalTheme } from "./emulator";

describe("terminal emulator", () => {
  it("provides the dark-well palette and dashboard tone mapping", () => {
    expect(terminalTheme()).toEqual({
      background: "#1c1f27",
      foreground: "#e3e5ea",
      cursor: "#8fb4f0",
      selectionBackground: "#8fb4f04d",
      black: "#1c1f27",
      red: "#e07a6f",
      green: "#7fc79a",
      yellow: "#d9b96a",
      blue: "#8fb4f0",
      magenta: "#c59bd9",
      cyan: "#7fc3cf",
      white: "#c8cbd3",
      brightBlack: "#5f6470",
      brightRed: "#ed978e",
      brightGreen: "#9bd8ae",
      brightYellow: "#e6cc88",
      brightBlue: "#acc8f3",
      brightMagenta: "#d8b6e5",
      brightCyan: "#9bd6df",
      brightWhite: "#f2f3f5",
    });
  });

  it("returns a fresh theme object", () => {
    expect(terminalTheme()).not.toBe(terminalTheme());
  });
});
