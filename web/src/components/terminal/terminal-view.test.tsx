import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createStore } from "jotai";
import { Provider, useAtomValue } from "jotai/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";

import fixtureJson from "@/api/fixtures/snapshot.json";
import { snapshotSchema, type Snapshot, type StageSummary } from "@/api/schema";
import { StageModal } from "@/components/stage-modal";
import { TerminalView } from "@/components/terminal/terminal-view";
import type { Emulator, EmulatorFactory, TerminalDeps } from "@/components/terminal/use-terminal";
import { TooltipProvider } from "@/components/ui/tooltip";
import { applySnapshot } from "@/state/apply";
import { selectStage, snapshotAtom } from "@/state/atoms";
const fixture = snapshotSchema.parse(fixtureJson);

class FakeSocket {
  static readonly OPEN = 1;
  static instances: FakeSocket[] = [];
  binaryType: BinaryType = "blob";
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onopen: ((event: Event) => void) | null = null;
  readyState = 0;
  readonly sent: unknown[] = [];
  readonly url: string;
  constructor(url: string) {
    this.url = url;
    FakeSocket.instances.push(this);
  }
  close(): void {
    this.readyState = 3;
  }
  open(): void {
    this.readyState = FakeSocket.OPEN;
    this.onopen?.(new Event("open"));
  }
  send(data: string | ArrayBufferLike | Blob | ArrayBufferView): void {
    this.sent.push(data);
  }
  closeFromServer(code: number, reason = "server reason"): void {
    this.readyState = 3;
    this.onclose?.({ code, reason } as CloseEvent);
  }
}
interface ScheduledTimer {
  callback: () => void;
  cancelled: boolean;
  delay: number;
  id: number;
}
function fakeTimers() {
  const all: ScheduledTimer[] = [];
  let nextId = 1;
  const schedule = (callback: TimerHandler, delay = 0): number => {
    all.push({ callback: callback as () => void, cancelled: false, delay, id: nextId });
    nextId += 1;
    return nextId - 1;
  };
  const cancel = (id: number): void => {
    const timer = all.find((candidate) => candidate.id === id);
    if (timer) timer.cancelled = true;
  };
  return {
    all,
    active: () => all.filter((timer) => !timer.cancelled),
    deps: {
      WebSocket: FakeSocket as unknown as typeof WebSocket,
      location: { protocol: "http:", host: "127.0.0.1:7373" },
      setTimeout: schedule as unknown as typeof setTimeout,
      clearTimeout: cancel as unknown as typeof clearTimeout,
    } satisfies TerminalDeps,
    fireNext: () => {
      const timer = all.find((candidate) => !candidate.cancelled);
      if (!timer) throw new Error("expected a scheduled timer");
      timer.cancelled = true;
      timer.callback();
    },
  };
}
interface EmulatorDouble {
  calls: {
    disposed: number;
    fit: number;
    focused: number;
    opened: HTMLElement[];
    readOnly: boolean[];
    written: Uint8Array[];
  };
  emulator: Emulator;
  emit(data: string): void;
}
function fakeEmulator(): EmulatorDouble {
  let onData: ((data: string) => void) | undefined;
  const calls: EmulatorDouble["calls"] = {
    disposed: 0,
    fit: 0,
    focused: 0,
    opened: [],
    readOnly: [],
    written: [],
  };
  return {
    calls,
    emulator: {
      open(host) {
        calls.opened.push(host);
      },
      write(data) {
        calls.written.push(data);
      },
      onData(handler) {
        onData = handler;
        return () => {
          if (onData === handler) onData = undefined;
        };
      },
      fit() {
        calls.fit += 1;
        return { cols: 120, rows: 36 };
      },
      focus() {
        calls.focused += 1;
      },
      setReadOnly(readOnly) {
        calls.readOnly.push(readOnly);
      },
      dispose() {
        calls.disposed += 1;
      },
    },
    emit(data) {
      onData?.(data);
    },
  };
}
function fakeFactory() {
  const doubles: EmulatorDouble[] = [];
  return {
    doubles,
    factory: async () => {
      const next = fakeEmulator();
      doubles.push(next);
      return next.emulator;
    },
  };
}
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
/// Reads its stage from the snapshot atom, like `StageModal` does, so a test
/// that pushes a later snapshot into the store (eg. `session_alive` flipping
/// back on) actually reaches `TerminalView` instead of the stage it was
/// first rendered with.
function LiveStage({
  id,
  factory,
  deps,
}: {
  id: string;
  factory: EmulatorFactory;
  deps: TerminalDeps;
}) {
  const snapshot = useAtomValue(snapshotAtom);
  const stage = selectStage(snapshot, id);
  if (!stage) throw new Error(`stage ${id} missing from snapshot`);
  return <TerminalView stage={stage} frame="page" factory={factory} deps={deps} />;
}

function renderView(stage: StageSummary, factory: EmulatorFactory, deps: TerminalDeps) {
  const store = createStore();
  applySnapshot(store, snapshotFor(stage));
  return {
    store,
    ...render(
      <Provider store={store}>
        <TooltipProvider>
          <LiveStage id={stage.id} factory={factory} deps={deps} />
        </TooltipProvider>
      </Provider>,
    ),
  };
}
function renderModal(
  stage: StageSummary,
  factory: EmulatorFactory,
  deps: TerminalDeps,
  path: string,
) {
  const store = createStore();
  applySnapshot(store, snapshotFor(stage));
  const router = createMemoryRouter(
    [{ path: "/", element: <StageModal terminalFactory={factory} terminalDeps={deps} /> }],
    { initialEntries: [path] },
  );
  render(
    <Provider store={store}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </Provider>,
  );
  return { router, store };
}
function well(): HTMLElement {
  const element = screen.getByLabelText("Agent terminal").closest(".terminal-well");
  if (!(element instanceof HTMLElement)) throw new Error("terminal well is missing");
  return element;
}
async function waitForMount(factory: ReturnType<typeof fakeFactory>): Promise<EmulatorDouble> {
  await waitFor(() => expect(factory.doubles).toHaveLength(1));
  return factory.doubles[0];
}
async function mounted(stage = terminalStage()) {
  const timers = fakeTimers();
  const factory = fakeFactory();
  const rendered = renderView(stage, factory.factory, timers.deps);
  return { ...rendered, emulator: await waitForMount(factory), factory, timers };
}

async function live(stage = terminalStage()) {
  const terminal = await mounted(stage);
  act(() => FakeSocket.instances[0].open());
  return terminal;
}

afterEach(() => {
  cleanup();
  FakeSocket.instances = [];
  vi.unstubAllGlobals();
});

describe("terminal view", () => {
  it("opens in view mode and says viewing once the connection opens", async () => {
    const { emulator } = await live();
    expect(emulator.calls.readOnly).toEqual([true]);
    expect(await screen.findByText("viewing")).toBeTruthy();
  });

  it("returns to details when Esc is pressed in view mode", async () => {
    const timers = fakeTimers();
    const factory = fakeFactory();
    const stage = terminalStage();
    const { router } = renderModal(
      stage,
      factory.factory,
      timers.deps,
      `/?stage=${stage.id}&view=terminal`,
    );

    await waitForMount(factory);
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });

    await waitFor(() => expect(router.state.location.search).toBe(`?stage=${stage.id}`));
  });

  it("takes control by remounting the emulator in control mode", async () => {
    const timers = fakeTimers();
    const factory = fakeFactory();
    renderView(terminalStage(), factory.factory, timers.deps);

    await waitForMount(factory);
    fireEvent.click(screen.getByRole("radio", { name: "Take control" }));
    await waitFor(() => expect(factory.doubles).toHaveLength(2));
    act(() => FakeSocket.instances[1].open());

    expect(factory.doubles[1].calls.readOnly).toEqual([false]);
    expect(factory.doubles[1].calls.focused).toBe(1);
    expect(well().dataset.mode).toBe("control");
  });

  it("keeps the terminal open when Esc is pressed in control mode", async () => {
    const timers = fakeTimers();
    const factory = fakeFactory();
    const stage = terminalStage();
    const { router } = renderModal(
      stage,
      factory.factory,
      timers.deps,
      `/?stage=${stage.id}&view=terminal`,
    );

    await waitForMount(factory);
    fireEvent.click(screen.getByRole("radio", { name: "Take control" }));
    await waitFor(() => expect(well().dataset.mode).toBe("control"));
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });

    expect(router.state.location.search).toBe(`?stage=${stage.id}&view=terminal`);
  });

  it("allows an outside pointer-down to close view mode but not control mode", async () => {
    const stage = terminalStage();
    const view = fakeFactory();
    const viewRouter = renderModal(
      stage,
      view.factory,
      fakeTimers().deps,
      `/?stage=${stage.id}&view=terminal`,
    ).router;
    await waitForMount(view);
    await new Promise((resolve) => window.setTimeout(resolve, 0));
    // Radix defers an outside pointer-down to the trailing click (so a drag
    // selection that ends inside the dialog does not dismiss it), so a real
    // outside dismissal needs both events, exactly like a browser click.
    fireEvent.pointerDown(document.body);
    fireEvent.click(document.body);
    await waitFor(() => expect(viewRouter.state.location.search).toBe(""));
    cleanup();

    const control = fakeFactory();
    const controlRouter = renderModal(
      stage,
      control.factory,
      fakeTimers().deps,
      `/?stage=${stage.id}&view=terminal`,
    ).router;
    await waitForMount(control);
    fireEvent.click(screen.getByRole("radio", { name: "Take control" }));
    await waitFor(() => expect(screen.getByRole("dialog").dataset.mode).toBe("control"));
    await new Promise((resolve) => window.setTimeout(resolve, 0));
    fireEvent.pointerDown(document.body);
    fireEvent.click(document.body);

    expect(controlRouter.state.location.search).toBe(`?stage=${stage.id}&view=terminal`);
  });

  it("tab in control mode stays in the well and reaches the binary transport", async () => {
    const timers = fakeTimers();
    const factory = fakeFactory();
    const stage = terminalStage();
    renderModal(stage, factory.factory, timers.deps, `/?stage=${stage.id}&view=terminal`);

    await waitForMount(factory);
    fireEvent.click(screen.getByRole("radio", { name: "Take control" }));
    await waitFor(() => expect(factory.doubles).toHaveLength(2));
    const emulator = factory.doubles[1];
    await waitFor(() => expect(emulator.calls.opened).toHaveLength(1));
    const socket = FakeSocket.instances[1];
    act(() => socket.open());
    const input = document.createElement("textarea");
    input.addEventListener("keydown", (event) => {
      if (event.key === "Tab") emulator.emit("\t");
    });
    emulator.calls.opened[0].append(input);
    input.focus();
    fireEvent.keyDown(input, { key: "Tab" });

    expect(well().contains(document.activeElement)).toBe(true);
    // jsdom's TextEncoder builds its Uint8Array from a realm distinct from the
    // test's global Uint8Array, so `instanceof` never matches; ArrayBuffer.isView
    // is the realm-safe way to confirm this is a binary frame, not a JSON one.
    expect(socket.sent.some((frame) => ArrayBuffer.isView(frame))).toBe(true);
    expect(Array.from(socket.sent.at(-1) as Uint8Array)).toEqual([9]);
  });

  it("unmount before the emulator resolves disposes the late emulator without opening a socket", async () => {
    const timers = fakeTimers();
    const late = fakeEmulator();
    let resolve: ((value: Emulator) => void) | undefined;
    const factory: EmulatorFactory = () =>
      new Promise((complete) => {
        resolve = complete;
      });
    const view = renderView(terminalStage(), factory, timers.deps);

    await waitFor(() => expect(resolve).toBeDefined());
    view.unmount();
    resolve?.(late.emulator);
    await waitFor(() => expect(late.calls.disposed).toBe(1));

    expect(late.calls.opened).toEqual([]);
    expect(FakeSocket.instances).toEqual([]);
  });

  it("shows a refused renderer failure without scheduling a retry", async () => {
    const timers = fakeTimers();
    const factory: EmulatorFactory = async () => Promise.reject(new Error("unavailable"));
    renderView(terminalStage(), factory, timers.deps);

    expect((await screen.findByRole("alert")).textContent).toContain(
      "terminal renderer failed to load",
    );
    expect(timers.active()).toEqual([]);
    expect(FakeSocket.instances).toEqual([]);
  });

  it("cancels a pending fit debounce when the view unmounts", async () => {
    class TestResizeObserver {
      static instances: TestResizeObserver[] = [];
      readonly callback: ResizeObserverCallback;

      constructor(callback: ResizeObserverCallback) {
        this.callback = callback;
        TestResizeObserver.instances.push(this);
      }

      observe(): void {}
      disconnect(): void {}
      fire(): void {
        this.callback([], this as unknown as ResizeObserver);
      }
    }

    // `src/test/setup.ts` installs its own ResizeObserver stub via
    // `Object.defineProperty(window, "ResizeObserver", { value: ... })` with no
    // `configurable: true`, unlike the `matchMedia`/`getBBox` stubs beside it in
    // that same file. A non-configurable property can never be redefined to a
    // different value by any means (`vi.stubGlobal`, `Object.defineProperty`,
    // or `delete`), so this stub cannot be swapped for one that fires from
    // this file alone; see the accompanying report for the one-line fix.
    vi.stubGlobal("ResizeObserver", TestResizeObserver);
    const timers = fakeTimers();
    const factory = fakeFactory();
    const view = renderView(terminalStage(), factory.factory, timers.deps);
    const emulator = await waitForMount(factory);
    await waitFor(() => expect(emulator.calls.opened).toHaveLength(1));
    const fitsBefore = emulator.calls.fit;
    TestResizeObserver.instances[0].fire();
    const pending = timers.active()[0];
    view.unmount();
    pending.callback();

    expect(emulator.calls.fit).toBe(fitsBefore);
  });

  it.each([
    [4009, "waiting", "waiting for the session's tmux server", true],
    [1000, "ended", "ended", false],
    [4004, "refused", "server reason", false],
    [4008, "refused", "server reason", false],
  ])("renders close code %i as %s", async (code, phase, text, retries) => {
    const { timers } = await live();
    act(() => FakeSocket.instances[0].closeFromServer(code));

    expect(await screen.findByText(text)).toBeTruthy();
    expect(well().dataset.phase).toBe(phase);
    expect(timers.active().length > 0).toBe(retries);
  });

  it("offers Retry after a second dropped connection and opens a new socket", async () => {
    const { timers } = await live();
    act(() => FakeSocket.instances[0].closeFromServer(1006, "first drop"));
    timers.fireNext();
    act(() => FakeSocket.instances[1].closeFromServer(1006, "second drop"));

    fireEvent.click(await screen.findByRole("button", { name: "Retry" }));
    expect(FakeSocket.instances).toHaveLength(3);
  });

  it.each([
    [false, "Start the dashboard with loom status --web --terminals"],
    [true, "Dashboard cookie missing — open the tokenized URL printed when the dashboard started"],
  ])("explains a pre-open refusal when terminals are %s", async (terminals, message) => {
    const timers = fakeTimers();
    const factory = fakeFactory();
    const store = createStore();
    applySnapshot(store, snapshotFor(terminalStage(), terminals));
    render(
      <Provider store={store}>
        <TooltipProvider>
          <TerminalView
            stage={terminalStage()}
            frame="page"
            factory={factory.factory}
            deps={timers.deps}
          />
        </TooltipProvider>
      </Provider>,
    );
    await waitForMount(factory);
    act(() => FakeSocket.instances[0].closeFromServer(1006));

    expect((await screen.findByRole("alert")).textContent).toContain(message);
  });

  it("offers Reconnect when an ended stage becomes alive again", async () => {
    const stage = terminalStage({ session_alive: false });
    const { store } = await live(stage);
    act(() => FakeSocket.instances[0].closeFromServer(1000));
    act(() => store.set(snapshotAtom, snapshotFor({ ...stage, session_alive: true })));

    fireEvent.click(await screen.findByRole("button", { name: "Reconnect" }));
    expect(FakeSocket.instances).toHaveLength(2);
  });

  it("shows the attach command and encoded page link in a dialog footer", async () => {
    const timers = fakeTimers();
    const factory = fakeFactory();
    const stage = terminalStage({ id: "stage/a" });
    renderModal(stage, factory.factory, timers.deps, "/?stage=stage%2Fa&view=terminal");
    await waitForMount(factory);

    expect(screen.getByText("loom attach stage/a")).toBeTruthy();
    expect(screen.getByRole("link", { name: /open in a tab/ }).getAttribute("href")).toBe(
      "/terminal/stage%2Fa",
    );
  });
});
