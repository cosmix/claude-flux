import { cleanup, render, screen } from "@testing-library/react";
import { createStore } from "jotai";
import { Provider } from "jotai/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import fixtureJson from "@/api/fixtures/snapshot.json";
import { snapshotSchema, type Snapshot, type StageSummary } from "@/api/schema";
import { TerminalPage } from "@/routes/terminal";
import type { EmulatorFactory } from "@/components/terminal/use-terminal";
import { TooltipProvider } from "@/components/ui/tooltip";
import { applySnapshot } from "@/state/apply";

const fixture = snapshotSchema.parse(fixtureJson);

function terminalStage(): StageSummary {
  const source = fixture.status.stages.find((stage) => stage.id === "client");
  if (!source) throw new Error("fixture stage client is missing");
  return { ...source, session_alive: true, session_backend: "tmux" };
}

function snapshotFor(stage: StageSummary): Snapshot {
  return {
    ...structuredClone(fixture),
    terminals: true,
    status: {
      ...fixture.status,
      stages: [stage, ...fixture.status.stages.filter((candidate) => candidate.id !== "client")],
    },
  };
}

function pendingFactory(): EmulatorFactory {
  return () => new Promise(() => {});
}

function renderPage(path: string, stage = terminalStage()) {
  const store = createStore();
  applySnapshot(store, snapshotFor(stage));
  const router = createMemoryRouter(
    [{ path: "/terminal/:stageId", element: <TerminalPage factory={pendingFactory()} /> }],
    { initialEntries: [path] },
  );
  render(
    <Provider store={store}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </Provider>,
  );
}

afterEach(cleanup);

describe("terminal route", () => {
  it("renders a known terminal in the page frame", () => {
    const stage = terminalStage();
    renderPage(`/terminal/${stage.id}`, stage);

    expect(screen.getByRole("heading", { name: stage.name })).toBeTruthy();
    expect(document.querySelector(".terminal-frame")?.getAttribute("data-frame")).toBe("page");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("renders No such stage for an unknown terminal id", () => {
    renderPage("/terminal/nope");

    expect(screen.getByText("No such stage")).toBeTruthy();
  });
});
