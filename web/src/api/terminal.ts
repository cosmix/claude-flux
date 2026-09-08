import type { EmulatorSize } from "@/components/terminal/emulator";

export type TerminalMode = "view" | "control";
export type TerminalPhase = "connecting" | "waiting" | "live" | "ended" | "dropped" | "refused";

export interface TerminalState {
  phase: TerminalPhase;
  mode: TerminalMode;
  reason?: string;
  /** Time when the current phase began, not when the latest frame arrived. */
  since: number;
}

export interface TerminalDeps {
  WebSocket?: typeof WebSocket;
  location?: { protocol: string; host: string };
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
  now?: () => number;
}

export const RETRY_WAITING_MS = 2000;
export const RETRY_DROPPED_MS = 1000;

export class TerminalConnection {
  private attempts = 0;
  private openedOnce = false;
  private retryTimer: ReturnType<typeof setTimeout> | undefined;
  private socket: WebSocket | undefined;
  private state: TerminalState | undefined;
  private stopped = false;
  private readonly Socket: typeof WebSocket;
  private readonly cancel: typeof clearTimeout;
  private readonly mode: TerminalMode;
  private readonly now: () => number;
  private readonly onState: (state: TerminalState) => void;
  private readonly schedule: typeof setTimeout;
  private readonly sink: (bytes: Uint8Array) => void;
  private readonly url: string;

  constructor(
    stageId: string,
    mode: TerminalMode,
    sink: (bytes: Uint8Array) => void,
    onState: (state: TerminalState) => void,
    deps: TerminalDeps = {},
  ) {
    this.mode = mode;
    this.sink = sink;
    this.onState = onState;
    this.Socket = deps.WebSocket ?? globalThis.WebSocket;
    this.schedule = deps.setTimeout ?? globalThis.setTimeout.bind(globalThis);
    this.cancel = deps.clearTimeout ?? globalThis.clearTimeout.bind(globalThis);
    this.now = deps.now ?? Date.now;
    const location = deps.location ?? globalThis.location;
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    this.url = `${scheme}://${location.host}/ws/terminal/${encodeURIComponent(stageId)}/${mode}`;
    this.openSocket();
  }

  send(data: string): void {
    if (this.mode !== "control" || !this.socketIsOpen()) return;
    this.socket?.send(new TextEncoder().encode(data));
  }

  resize(size: EmulatorSize): void {
    if (!this.socketIsOpen()) return;
    this.socket?.send(JSON.stringify({ resize: { cols: size.cols, rows: size.rows } }));
  }

  retry(): void {
    if (this.stopped) return;
    this.attempts = 0;
    this.cancelRetry();
    const previous = this.socket;
    this.socket = undefined;
    previous?.close();
    this.openSocket();
  }

  close(): void {
    if (this.stopped) return;
    this.stopped = true;
    this.cancelRetry();
    const socket = this.socket;
    this.socket = undefined;
    socket?.close();
  }

  private openSocket(): void {
    if (this.stopped) return;
    this.emit("connecting");
    const socket = new this.Socket(this.url);
    this.socket = socket;
    socket.binaryType = "arraybuffer";
    socket.onopen = () => this.handleOpen(socket);
    socket.onmessage = (event) => this.handleMessage(socket, event.data);
    socket.onclose = (event) => this.handleClose(socket, event);
    socket.onerror = () => {
      if (this.socket !== socket) return;
    };
  }

  private handleOpen(socket: WebSocket): void {
    if (this.socket !== socket) return;
    this.attempts = 0;
    this.openedOnce = true;
    this.emit("live");
  }

  private handleMessage(socket: WebSocket, data: unknown): void {
    if (this.socket !== socket || !(data instanceof ArrayBuffer)) return;
    this.sink(new Uint8Array(data));
  }

  private handleClose(socket: WebSocket, event: CloseEvent): void {
    if (this.socket !== socket || this.stopped) return;
    if (this.handleKnownClose(event)) return;
    const reason = event.reason || "connection lost";
    if (!this.openedOnce) {
      this.emit("refused", "handshake refused");
    } else if (this.attempts === 0) {
      this.attempts = 1;
      this.emit("connecting", reason);
      this.scheduleRetry(RETRY_DROPPED_MS);
    } else {
      this.emit("dropped", reason);
    }
  }

  private handleKnownClose(event: CloseEvent): boolean {
    if (event.code === 1000) this.emit("ended", event.reason);
    else if (event.code === 1001) this.emit("dropped", "server stopping");
    else if (event.code === 4004 || event.code === 4008) this.emit("refused", event.reason);
    else if (event.code === 4009) {
      this.emit("waiting", event.reason);
      this.scheduleRetry(RETRY_WAITING_MS);
    } else return false;
    return true;
  }

  private scheduleRetry(delay: number): void {
    this.cancelRetry();
    this.retryTimer = this.schedule(() => {
      this.retryTimer = undefined;
      this.openSocket();
    }, delay);
  }

  private cancelRetry(): void {
    if (this.retryTimer === undefined) return;
    this.cancel(this.retryTimer);
    this.retryTimer = undefined;
  }

  private socketIsOpen(): boolean {
    return this.socket?.readyState === this.Socket.OPEN;
  }

  private emit(phase: TerminalPhase, reason?: string): void {
    const since = this.state?.phase === phase ? this.state.since : this.now();
    this.state = { phase, mode: this.mode, since, ...(reason ? { reason } : {}) };
    this.onState(this.state);
  }
}
