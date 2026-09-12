import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { createStore } from "jotai";
import { Provider } from "jotai/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { ConfigClient } from "@/api/config";
import { routes } from "@/router";
import { renderAt, snapshot } from "@/test/settings-kit";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("header entry point", () => {
  it("opens the dialog from the header and fetches /api/config", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      if (String(input) === "/api/config") {
        return new Response(JSON.stringify(snapshot()), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      return new Response("{}", { status: 404 });
    });
    vi.stubGlobal("fetch", fetchMock);
    const router = createMemoryRouter(routes, { initialEntries: ["/ledger"] });
    render(
      <Provider store={createStore()}>
        <RouterProvider router={router} />
      </Provider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "open settings" }));

    expect(router.state.location.search).toBe("?settings=1");
    expect(screen.getByRole("dialog", { name: "Settings" })).toBeTruthy();
    await screen.findByRole("switch", { name: "check at user scope" });
    expect(screen.getByText("backend", { selector: ".settings-key-name" })).toBeTruthy();
    expect(fetchMock).toHaveBeenCalledWith("/api/config", { cache: "no-store" });
  });
});

/// Lives here rather than settings-dialog.test.tsx to stay under that file's
/// line ceiling; the scenario is otherwise unrelated to the header.
describe("settings dialog load failure", () => {
  it("offers a retry when the config fetch fails", async () => {
    let calls = 0;
    const client: ConfigClient = {
      load: async () => {
        calls += 1;
        if (calls === 1) throw new Error("config fetch failed: HTTP 500");
        return snapshot();
      },
      write: async () => ({ ok: false, status: null, message: "unused" }),
    };
    renderAt("?settings=1", client);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("HTTP 500");
    fireEvent.click(screen.getByRole("button", { name: "retry" }));
    expect(await screen.findByRole("switch", { name: "check at user scope" })).toBeTruthy();
  });
});
