import { describe, expect, it } from "vitest";

import { terminalTheme } from "./emulator";

describe("terminal emulator", () => {
  it("provides the dark-well palette and dashboard tone mapping", () => {
    expect(terminalTheme()).toEqual({
      background: "#101216",
      foreground: "#d8dce3",
      cursor: "#8fb4f0",
      selectionBackground: "#8fb4f04d",
      black: "#101216",
      red: "#e07a6f",
      green: "#7fc79a",
      yellow: "#d9b96a",
      blue: "#8fb4f0",
      magenta: "#c59bd9",
      cyan: "#7fc3cf",
      white: "#c8cbd3",
      brightBlack: "#89919e",
      brightRed: "#ed978e",
      brightGreen: "#9bd8ae",
      brightYellow: "#e6cc88",
      brightBlue: "#acc8f3",
      brightMagenta: "#d8b6e5",
      brightCyan: "#9bd6df",
      brightWhite: "#eef0f4",
    });
  });

  it("returns a fresh theme object", () => {
    expect(terminalTheme()).not.toBe(terminalTheme());
  });
});
