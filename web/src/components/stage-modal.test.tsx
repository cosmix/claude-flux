import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createStore } from "jotai";
import { Provider } from "jotai/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, describe, expect, it } from "vitest";

import fixtureJson from "@/api/fixtures/snapshot.json";
import { snapshotSchema, type Snapshot, type StageSummary } from "@/api/schema";
import { StageModal } from "@/components/stage-modal";
import type { EmulatorFactory } from "@/components/terminal/use-terminal";
import { TooltipProvider } from "@/components/ui/tooltip";
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

function pendingFactory(): EmulatorFactory {
  return () => new Promise(() => {});
}

function renderModal(stage: StageSummary, path: string, terminals = true) {
  const store = createStore();
  applySnapshot(store, snapshotFor(stage, terminals));
  const router = createMemoryRouter(
    [{ path: "/", element: <StageModal terminalFactory={pendingFactory()} /> }],
    { initialEntries: [path] },
  );
  render(
    <Provider store={store}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </Provider>,
  );
  return router;
}

function terminalButton(): HTMLButtonElement {
  const button = screen.getByRole("button", { name: "open terminal" });
  if (!(button instanceof HTMLButtonElement)) throw new Error("terminal button is missing");
  return button;
}

afterEach(cleanup);

describe("stage modal", () => {
  it("renders the terminal face wide for a terminal query and details without one", () => {
    const stage = terminalStage();
    renderModal(stage, `/?stage=${stage.id}&view=terminal`);
    const dialog = screen.getByRole("dialog");

    expect(screen.getByText("Terminal")).toBeTruthy();
    expect(dialog.className).toContain("terminal-dialog");
    expect(dialog.className).toContain("sm:max-w-[min(96vw,1280px)]");
    cleanup();

    renderModal(stage, `/?stage=${stage.id}`);
    expect(screen.getByText(stage.name)).toBeTruthy();
    expect(screen.queryByText("every key reaches the agent")).toBeNull();
  });

  it("opens the terminal with t only when the stage passes the terminal gate", async () => {
    const stage = terminalStage();
    const router = renderModal(stage, `/?stage=${stage.id}`);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "t" });
    await waitFor(() =>
      expect(router.state.location.search).toBe(`?stage=${stage.id}&view=terminal`),
    );
    cleanup();

    const native = terminalStage({ session_backend: "native" });
    const blocked = renderModal(native, `/?stage=${native.id}`);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "t" });

    expect(blocked.state.location.search).toBe(`?stage=${native.id}`);
  });

  it.each([
    [terminalStage({ session_backend: "native" }), true, "--backend tmux"],
    [terminalStage(), false, "loom status --web --terminals"],
  ])("explains why a disabled terminal button cannot open", async (stage, terminals, reason) => {
    renderModal(stage, `/?stage=${stage.id}`, terminals);
    const button = terminalButton();

    expect(button.disabled).toBe(true);
    fireEvent.pointerMove(button.parentElement!);

    expect(await screen.findByText(new RegExp(reason))).toBeTruthy();
  });
});
