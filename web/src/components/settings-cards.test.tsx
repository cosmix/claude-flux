import { cleanup, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { fakeClient, renderAt } from "@/test/settings-kit";

/// A card is found by the text of its heading — `rowLabel`, same as a lanes
/// row: `"standard"` for the pair, `"check"` for the single key.
function card(label: string): HTMLElement {
  const heading = screen.getByText(label, { selector: ".settings-key-name" });
  const element = heading.closest(".settings-card");
  if (!(element instanceof HTMLElement)) throw new Error(`no card for ${label}`);
  return element;
}

const originalMatchMedia = window.matchMedia;

beforeEach(() => {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches: query === "(max-width: 699px)",
      media: query,
      onchange: null,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      dispatchEvent: () => false,
    }),
  });
});

afterEach(() => {
  cleanup();
  Object.defineProperty(window, "matchMedia", { configurable: true, value: originalMatchMedia });
});

describe("settings cards", () => {
  it("renders cards instead of a table under the narrow breakpoint", async () => {
    renderAt("?settings=1", fakeClient().client);

    await screen.findByRole("switch", { name: "check at user scope" });
    expect(document.querySelector(".settings-cards")).toBeTruthy();
    expect(document.querySelector("table")).toBeNull();
  });

  it("draws a pair card with three ordered tiers, two controls each, and the effective project tier", async () => {
    renderAt("?settings=1", fakeClient().client);
    await screen.findByRole("switch", { name: "check at user scope" });

    const standard = card("standard");
    const tiers = within(standard).getAllByText(/^(built-in|user|project)$/, {
      selector: ".settings-tl",
    });
    expect(tiers.map((tier) => tier.textContent)).toEqual(["built-in", "user", "project"]);

    const userTier = standard.querySelector('.settings-tier[data-lane="user"]');
    if (!userTier) throw new Error("standard card has no user tier");
    expect(within(userTier as HTMLElement).getAllByRole("combobox")).toHaveLength(2);

    const projectTier = standard.querySelector('.settings-tier[data-lane="project"]');
    if (!projectTier) throw new Error("standard card has no project tier");
    expect(within(projectTier as HTMLElement).getAllByRole("combobox")).toHaveLength(2);
    expect(projectTier.getAttribute("data-effective")).toBe("true");

    // The pair's model resolves at project, its effort at user: a tier
    // carries `data-effective` when any of its entries is the one in effect.
    expect(userTier.getAttribute("data-effective")).toBe("true");
    const builtinTier = standard.querySelector('.settings-tier[data-lane="builtin"]');
    if (!builtinTier) throw new Error("standard card has no built-in tier");
    expect(builtinTier.getAttribute("data-effective")).toBeNull();

    expect(standard.querySelector(".settings-res")?.textContent).toBe("runs sonnet · high");
  });

  it("shows the slim user-only row on a single-tier card with no project control", async () => {
    renderAt("?settings=1", fakeClient().client);
    await screen.findByRole("switch", { name: "check at user scope" });

    const check = card("check");
    expect(within(check).getByText("user-only keys, no project tier")).toBeTruthy();
    expect(within(check).queryByRole("switch", { name: /at project scope/ })).toBeNull();
  });

  it("writes from a card control the same way the table would", async () => {
    const { client, writes } = fakeClient();
    renderAt("?settings=1", client);
    const toggle = await screen.findByRole("switch", { name: "check at user scope" });

    fireEvent.click(toggle);

    expect(writes).toEqual([{ scope: "user", name: "update.check", value: "false" }]);
  });
});
