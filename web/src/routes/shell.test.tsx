import { cleanup, render, screen } from "@testing-library/react";
import { createStore } from "jotai";
import { Provider } from "jotai/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import fixture from "@/api/fixtures/snapshot.json";
import { snapshotSchema } from "@/api/schema";
import { routes } from "@/router";
import { applySnapshot } from "@/state/apply";
import { connectionAtom, type ConnectionState } from "@/state/atoms";

const snapshot = snapshotSchema.parse(fixture);

// Renders at /ledger rather than the overview: the header/footer chrome under
// test is identical either way, and the ledger body skips React Flow, whose
// inert ResizeObserver in jsdom makes the graph noisy to render around.
function renderShell(connection: ConnectionState, path = "/ledger") {
  const store = createStore();
  applySnapshot(store, snapshot);
  store.set(connectionAtom, connection);
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  render(
    <Provider store={store}>
      <RouterProvider router={router} />
    </Provider>,
  );
  return router;
}

afterEach(cleanup);

describe("shell chrome", () => {
  it("shows connecting while waiting for the first snapshot", () => {
    const store = createStore();
    store.set(connectionAtom, { phase: "connecting", since: Date.now() });
    const router = createMemoryRouter(routes, { initialEntries: ["/ledger"] });
    render(
      <Provider store={store}>
        <RouterProvider router={router} />
      </Provider>,
    );

    expect(screen.getByLabelText("loading ledger")).toBeTruthy();
    expect(screen.getByText("connecting")).toBeTruthy();
    expect(screen.queryByText(/daemon (running|stopped|unknown)/)).toBeNull();
  });

  it("shows the initial connection error reason", () => {
    const store = createStore();
    store.set(connectionAtom, {
      phase: "error",
      since: Date.now(),
      message: "status.stages: required",
    });
    const router = createMemoryRouter(routes, { initialEntries: ["/ledger"] });
    render(
      <Provider store={store}>
        <RouterProvider router={router} />
      </Provider>,
    );

    const line = screen.getByText("connection error").parentElement;
    expect(line?.getAttribute("title")).toBe("status.stages: required");
  });

  it("reports a live feed as running without a redundant connection pill", () => {
    renderShell({ phase: "live", since: Date.now() });

    expect(screen.getByText("daemon running")).toBeTruthy();
    expect(screen.getByText(/tick/)).toBeTruthy();
    expect(screen.queryByText("daemon unknown")).toBeNull();
    expect(screen.queryByRole("button", { name: /connection/ })).toBeNull();
  });

  it("marks a dropped feed unknown instead of repeating the frozen snapshot", () => {
    renderShell({ phase: "offline", since: Date.now() - 135_000 });

    expect(screen.getByText("daemon unknown")).toBeTruthy();
    expect(screen.getByText(/no data for 2m15s/)).toBeTruthy();
    expect(screen.queryByText("daemon running")).toBeNull();
    expect(screen.queryByRole("button", { name: /connection/ })).toBeNull();
  });

  it("treats reconnecting as stale too", () => {
    renderShell({ phase: "reconnecting", since: Date.now() - 5_000 });

    expect(screen.getByText("daemon unknown")).toBeTruthy();
    expect(screen.getByText(/no data for 5s/)).toBeTruthy();
  });

  it("keeps a live feed distinct from a stopped daemon", () => {
    const stopped = snapshotSchema.parse({ ...fixture, daemon: "not-running" });
    const store = createStore();
    applySnapshot(store, stopped);
    store.set(connectionAtom, { phase: "live", since: Date.now() });
    const router = createMemoryRouter(routes, { initialEntries: ["/ledger"] });
    render(
      <Provider store={store}>
        <RouterProvider router={router} />
      </Provider>,
    );

    expect(screen.getByText("daemon stopped")).toBeTruthy();
    expect(screen.queryByText("daemon running")).toBeNull();
  });

  it("labels the footer's clock and what it means, in 24-hour time", () => {
    renderShell({ phase: "live", since: Date.now() });

    const footer = screen.getByRole("contentinfo");
    expect(footer.textContent).toContain("updated ");
    expect(footer.textContent).toContain(" ago");
    expect(footer.textContent).toContain(`via ${snapshot.source}`);
    expect(footer.textContent).toMatch(/\b\d{2}:\d{2}:\d{2}\b/);
    expect(footer.textContent).not.toMatch(/AM|PM/);
  });
});
