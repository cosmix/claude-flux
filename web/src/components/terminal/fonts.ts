import "./fonts.css";

export const TERMINAL_FONT_FAMILY = '"Commit Mono Cosmix", monospace';

/** Resolve every ANSI face before xterm measures cells or builds its glyph atlas. */
export async function loadTerminalFonts(): Promise<void> {
  await Promise.all(
    ["400", "italic 400", "700", "italic 700"].map((face) =>
      document.fonts.load(`${face} 13px "Commit Mono Cosmix"`),
    ),
  );
}
