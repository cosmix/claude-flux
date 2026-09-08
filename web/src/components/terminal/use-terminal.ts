import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

import {
  TerminalConnection,
  type TerminalDeps,
  type TerminalMode,
  type TerminalState,
} from "@/api/terminal";
import {
  createXtermEmulator,
  type Emulator,
  type EmulatorFactory,
  type EmulatorSize,
} from "./emulator";

export type { TerminalMode, TerminalPhase, TerminalState, TerminalDeps } from "@/api/terminal";
export type { Emulator, EmulatorFactory, EmulatorSize } from "./emulator";

function sizesMatch(left: EmulatorSize | null, right: EmulatorSize): boolean {
  return left?.cols === right.cols && left.rows === right.rows;
}

class TerminalSession {
  private cancelled = false;
  private connection: TerminalConnection | undefined;
  private disposeInput: (() => void) | undefined;
  private emulator: Emulator | undefined;
  private fitTimer: ReturnType<typeof setTimeout> | undefined;
  private lastState: TerminalState | undefined;
  private measured: EmulatorSize | null = null;
  private observer: ResizeObserver | undefined;
  private readonly cancelTimer: typeof clearTimeout;
  private readonly deps: TerminalDeps;
  private readonly factory: EmulatorFactory;
  private readonly getHost: () => HTMLDivElement | null;
  private readonly mode: TerminalMode;
  private readonly now: () => number;
  private readonly onSize: (size: EmulatorSize | null) => void;
  private readonly onState: (state: TerminalState) => void;
  private readonly schedule: typeof setTimeout;
  private readonly stageId: string;

  constructor(
    stageId: string,
    mode: TerminalMode,
    factory: EmulatorFactory,
    deps: TerminalDeps,
    getHost: () => HTMLDivElement | null,
    onState: (state: TerminalState) => void,
    onSize: (size: EmulatorSize | null) => void,
  ) {
    this.stageId = stageId;
    this.mode = mode;
    this.factory = factory;
    this.deps = deps;
    this.getHost = getHost;
    this.onState = onState;
    this.onSize = onSize;
    this.now = deps.now ?? Date.now;
    this.schedule = deps.setTimeout ?? globalThis.setTimeout.bind(globalThis);
    this.cancelTimer = deps.clearTimeout ?? globalThis.clearTimeout.bind(globalThis);
  }

  start(): void {
    this.publishState({ phase: "connecting", mode: this.mode, since: this.now() });
    this.onSize(null);
    void this.factory().then(
      (created) => this.mount(created),
      () => this.refuse(),
    );
  }

  retry(): void {
    this.connection?.retry();
  }

  focus(): void {
    this.emulator?.focus();
  }

  dispose(): void {
    this.cancelled = true;
    if (this.fitTimer !== undefined) this.cancelTimer(this.fitTimer);
    this.observer?.disconnect();
    this.disposeInput?.();
    this.connection?.close();
    this.emulator?.dispose();
  }

  private mount(created: Emulator): void {
    const host = this.getHost();
    if (this.cancelled || !host) {
      created.dispose();
      return;
    }
    this.emulator = created;
    created.open(host);
    created.setReadOnly(this.mode === "view");
    this.connection = new TerminalConnection(
      this.stageId,
      this.mode,
      (bytes) => created.write(bytes),
      (state) => this.receiveState(state),
      this.deps,
    );
    this.disposeInput = created.onData((data) => this.connection?.send(data));
    this.observer = new ResizeObserver(() => this.scheduleFit());
    this.observer.observe(host);
    this.fitAndResize(false);
  }

  private refuse(): void {
    if (this.cancelled) return;
    this.publishState({
      phase: "refused",
      mode: this.mode,
      reason: "terminal renderer failed to load",
      since: this.now(),
    });
  }

  private receiveState(state: TerminalState): void {
    if (this.cancelled) return;
    this.publishState(state);
    if (state.phase === "live") this.fitAndResize(true);
  }

  private publishState(state: TerminalState): void {
    const since = this.lastState?.phase === state.phase ? this.lastState.since : state.since;
    this.lastState = { ...state, since };
    this.onState(this.lastState);
  }

  private scheduleFit(): void {
    if (this.cancelled) return;
    if (this.fitTimer !== undefined) this.cancelTimer(this.fitTimer);
    this.fitTimer = this.schedule(() => {
      this.fitTimer = undefined;
      this.fitAndResize(false);
    }, 100);
  }

  private fitAndResize(force: boolean): void {
    if (this.cancelled || !this.emulator) return;
    const next = this.emulator.fit();
    if (!next) return;
    const changed = !sizesMatch(this.measured, next);
    this.measured = next;
    if (changed) this.onSize(next);
    if (force || changed) this.connection?.resize(next);
  }
}

export function useTerminal(
  stageId: string,
  mode: TerminalMode,
  factory: EmulatorFactory = createXtermEmulator,
  deps: TerminalDeps = {},
): {
  hostRef: RefObject<HTMLDivElement | null>;
  state: TerminalState;
  size: EmulatorSize | null;
  retry: () => void;
  focus: () => void;
} {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const sessionRef = useRef<TerminalSession | null>(null);
  const now = deps.now ?? Date.now;
  const [state, setState] = useState<TerminalState>(() => ({
    phase: "connecting",
    mode,
    since: now(),
  }));
  const [size, setSize] = useState<EmulatorSize | null>(null);
  const retry = useCallback(() => sessionRef.current?.retry(), []);
  const focus = useCallback(() => sessionRef.current?.focus(), []);

  useEffect(() => {
    const session = new TerminalSession(
      stageId,
      mode,
      factory,
      deps,
      () => hostRef.current,
      setState,
      setSize,
    );
    sessionRef.current = session;
    session.start();
    return () => {
      session.dispose();
      if (sessionRef.current === session) sessionRef.current = null;
    };
  }, [stageId, mode]);

  return { hostRef, state, size, retry, focus };
}
