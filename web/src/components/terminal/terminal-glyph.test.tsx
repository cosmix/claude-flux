import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { createStore } from "jotai";
import { Provider } from "jotai/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import fixtureJson from "@/api/fixtures/snapshot.json";
import { snapshotSchema, type Snapshot, type StageSummary } from "@/api/schema";
import { LedgerRow } from "@/components/ledger-row";
import { useOpenTerminal } from "@/components/stage-modal";
import { TerminalGlyph, terminalGate } from "@/components/terminal/terminal-glyph";
import { TooltipProvider } from "@/components/ui/tooltip";
import { routes } from "@/router";
import { applySnapshot } from "@/state/apply";

const fixture = snapshotSchema.parse(fixtureJson);

function terminalStage(overrides: Partial<StageSummary> = {}): StageSummary {
  const source = fixture.status.stages.find((stage) => stage.id === "client");
  if (!source) throw new Error("fixture stage client is missing");
  return { ...source, session_alive: true, session_backend: "tmux", ...overrides };
}

function snapshotFor(stage: StageSummary, terminals = true): Snapshot {
  return {
    ...structuredClone(fixture),
    terminals,
    status: {
      ...fixture.status,
      stages: [stage, ...fixture.status.stages.filter((candidate) => candidate.id !== "client")],
    },
  };
}

function renderGraph(stage: StageSummary) {
  const store = createStore();
  applySnapshot(store, snapshotFor(stage));
  const router = createMemoryRouter(routes, { initialEntries: ["/"] });
  return render(
    <Provider store={store}>
      <RouterProvider router={router} />
    </Provider>,
  );
}

function renderLedger(stage: StageSummary) {
  const store = createStore();
  applySnapshot(store, snapshotFor(stage));
  const router = createMemoryRouter(
    [
      {
        path: "/",
        element: (
          <TooltipProvider>
            <table>
              <tbody>
                <LedgerRow stage={stage} level={0} />
              </tbody>
            </table>
          </TooltipProvider>
        ),
      },
    ],
    { initialEntries: ["/"] },
  );
  render(
    <Provider store={store}>
      <RouterProvider router={router} />
    </Provider>,
  );
  return router;
}

function GlyphEntry({ stage }: { stage: StageSummary }) {
  const open = useOpenTerminal();
  return <TerminalGlyph stage={stage} onOpen={open} />;
}

function renderGlyphEntry(stage: StageSummary) {
  const store = createStore();
  applySnapshot(store, snapshotFor(stage));
  const router = createMemoryRouter([{ path: "/", element: <GlyphEntry stage={stage} /> }], {
    initialEntries: ["/"],
  });
  render(
    <Provider store={store}>
      <RouterProvider router={router} />
    </Provider>,
  );
  return router;
}

afterEach(cleanup);

describe("terminal glyph", () => {
  it("renders on both graph cards and ledger rows when the tmux session is live", () => {
    const stage = terminalStage();
    const graph = renderGraph(stage);
    expect(screen.getByRole("button", { name: `open terminal for ${stage.name}` })).toBeTruthy();
    graph.unmount();

    renderLedger(stage);
    expect(screen.getByRole("button", { name: `open terminal for ${stage.name}` })).toBeTruthy();
  });

  it.each([
    [false, {}, "terminals are disabled"],
    [true, { session_backend: "native" as const }, "the backend is native"],
    [true, { session_alive: false }, "the session is not alive"],
  ])("does not render when %s", (terminals, overrides, _description) => {
    const stage = terminalStage(overrides);
    const store = createStore();
    applySnapshot(store, snapshotFor(stage, terminals));
    render(
      <Provider store={store}>
        <TerminalGlyph stage={stage} onOpen={() => {}} />
      </Provider>,
    );

    expect(screen.queryByRole("button", { name: `open terminal for ${stage.name}` })).toBeNull();
  });

  it("opens the terminal search parameters when clicked", () => {
    const stage = terminalStage();
    const router = renderGlyphEntry(stage);

    fireEvent.click(screen.getByRole("button", { name: `open terminal for ${stage.name}` }));

    expect(router.state.location.search).toBe(`?stage=${stage.id}&view=terminal`);
  });

  it("stops a ledger glyph click before the row navigates", () => {
    const stage = terminalStage();
    const router = renderLedger(stage);

    fireEvent.click(screen.getByRole("button", { name: `open terminal for ${stage.name}` }));

    expect(router.state.location.pathname).toBe("/");
    expect(router.state.location.search).toBe(`?stage=${stage.id}&view=terminal`);
  });

  it("names the native-backend requirement in the gate reason", () => {
    expect(terminalGate(true, terminalStage({ session_backend: "native" }))).toContain(
      "--backend tmux",
    );
  });
});
