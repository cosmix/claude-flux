import { beforeEach, describe, expect, it } from "vitest";

import {
  RETRY_DROPPED_MS,
  RETRY_WAITING_MS,
  TerminalConnection,
  type TerminalDeps,
  type TerminalMode,
  type TerminalState,
} from "@/api/terminal";

class FakeSocket {
  static readonly OPEN = 1;
  static instances: FakeSocket[] = [];

  binaryType: BinaryType = "blob";
  closed = false;
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
    this.closed = true;
    this.readyState = 3;
  }

  open(): void {
    this.readyState = FakeSocket.OPEN;
    this.onopen?.(new Event("open"));
  }

  send(data: unknown): void {
    this.sent.push(data);
  }

  receive(data: ArrayBuffer | string): void {
    this.onmessage?.({ data } as MessageEvent);
  }

  closeFromServer(code: number, reason = "server reason"): void {
    this.readyState = 3;
    this.onclose?.({ code, reason } as CloseEvent);
  }
}

interface ScheduledTimer {
  id: number;
  callback: () => void;
  delay: number;
  cancelled: boolean;
}

let nextTimerId = 1;
let timers: ScheduledTimer[] = [];

function fakeTimeout(handler: TimerHandler, delay = 0): number {
  const timer = { id: nextTimerId++, callback: handler as () => void, delay, cancelled: false };
  timers.push(timer);
  return timer.id;
}

function fakeClearTimeout(id: number): void {
  const timer = timers.find((candidate) => candidate.id === id);
  if (timer) timer.cancelled = true;
}

function fireNextTimer(): void {
  const timer = timers.find((candidate) => !candidate.cancelled);
  if (!timer) throw new Error("expected a scheduled reconnect");
  timer.cancelled = true;
  timer.callback();
}

function dependencies(): TerminalDeps {
  let timestamp = 0;
  return {
    WebSocket: FakeSocket as unknown as typeof WebSocket,
    location: { protocol: "http:", host: "127.0.0.1:7373" },
    setTimeout: fakeTimeout as unknown as typeof setTimeout,
    clearTimeout: fakeClearTimeout as unknown as typeof clearTimeout,
    now: () => ++timestamp,
  };
}

function connect(mode: TerminalMode = "control"): {
  connection: TerminalConnection;
  states: TerminalState[];
  output: Uint8Array[];
} {
  const states: TerminalState[] = [];
  const output: Uint8Array[] = [];
  const connection = new TerminalConnection(
    "stage/a",
    mode,
    (bytes) => output.push(bytes),
    (state) => states.push(state),
    dependencies(),
  );
  return { connection, states, output };
}

beforeEach(() => {
  FakeSocket.instances = [];
  nextTimerId = 1;
  timers = [];
});

describe("terminal connection", () => {
  it("live on open", () => {
    const { connection, states } = connect();
    const socket = FakeSocket.instances[0];

    expect(socket.url).toBe("ws://127.0.0.1:7373/ws/terminal/stage%2Fa/control");
    expect(socket.binaryType).toBe("arraybuffer");
    socket.open();

    expect(states.at(-1)).toMatchObject({ phase: "live", mode: "control" });
    connection.close();
  });

  it("encodes keystrokes as binary", () => {
    const { connection } = connect();
    const socket = FakeSocket.instances[0];
    socket.open();

    connection.send("A");
    connection.send("é");

    expect(socket.sent).toHaveLength(2);
    expect(Object.prototype.toString.call(socket.sent[0])).toBe("[object Uint8Array]");
    expect(Array.from(socket.sent[0] as Uint8Array)).toEqual([0x41]);
    expect(Array.from(socket.sent[1] as Uint8Array)).toEqual([0xc3, 0xa9]);
    connection.close();
  });

  it("close code table", () => {
    const cases = [
      { code: 1000, phase: "ended", reason: "server reason", retry: false },
      { code: 1001, phase: "dropped", reason: "server stopping", retry: false },
      { code: 1009, phase: "refused", reason: "input too large", retry: false },
      { code: 4004, phase: "refused", reason: "server reason", retry: false },
      { code: 4008, phase: "refused", reason: "server reason", retry: false },
      { code: 4009, phase: "waiting", reason: "server reason", retry: true },
      { code: 1006, phase: "connecting", reason: "server reason", retry: true },
    ] as const;

    for (const row of cases) {
      FakeSocket.instances = [];
      timers = [];
      const { connection, states } = connect();
      const socket = FakeSocket.instances[0];
      socket.open();
      socket.closeFromServer(row.code);

      expect(states.at(-1)?.phase, `close code ${row.code}`).toBe(row.phase);
      expect(states.at(-1)?.reason, `reason for ${row.code}`).toBe(row.reason);
      expect(
        timers.some((timer) => !timer.cancelled),
        `retry for ${row.code}`,
      ).toBe(row.retry);
      connection.close();
    }
  });

  it("waiting retries after 2000 ms via the injected timers", () => {
    const { connection, states } = connect();
    FakeSocket.instances[0].closeFromServer(4009, "not ready");

    expect(states.at(-1)).toMatchObject({ phase: "waiting", reason: "not ready" });
    expect(timers.at(-1)?.delay).toBe(RETRY_WAITING_MS);
    fireNextTimer();

    expect(FakeSocket.instances).toHaveLength(2);
    expect(states.at(-1)?.phase).toBe("connecting");
    connection.close();
  });

  it("the second 1006 lands in dropped", () => {
    const { connection, states } = connect();
    FakeSocket.instances[0].open();
    FakeSocket.instances[0].closeFromServer(1006, "first drop");
    expect(timers.at(-1)?.delay).toBe(RETRY_DROPPED_MS);

    fireNextTimer();
    FakeSocket.instances[1].closeFromServer(1006, "second drop");

    expect(states.at(-1)).toMatchObject({ phase: "dropped", reason: "second drop" });
    expect(timers.some((timer) => !timer.cancelled)).toBe(false);
    connection.close();
  });

  it("a 1006 before any open lands in refused with no retry scheduled", () => {
    const { connection, states } = connect();
    FakeSocket.instances[0].closeFromServer(1006);

    expect(states.at(-1)).toMatchObject({ phase: "refused", reason: "handshake refused" });
    expect(timers).toHaveLength(0);
    connection.close();
  });

  it("send in view mode sends nothing", () => {
    const { connection } = connect("view");
    const socket = FakeSocket.instances[0];
    socket.open();

    connection.send("ignored");

    expect(socket.sent).toHaveLength(0);
    connection.close();
  });

  it("chunks a paste over 32 KiB into ordered frames that reassemble exactly", () => {
    const { connection } = connect();
    const socket = FakeSocket.instances[0];
    socket.open();

    // 32767 ASCII bytes, then a surrogate-pair emoji (2 UTF-16 code units, 4
    // UTF-8 bytes), so a naive implementation slicing the *string* at a
    // 32768-code-unit boundary would cut the emoji's surrogate pair in half
    // and corrupt it. Byte-slicing the encoded payload instead never does
    // that: the PTY reassembles a byte stream regardless of where a
    // multi-byte character's bytes fall relative to a frame boundary.
    const payload = "a".repeat(32 * 1024 - 1) + "\u{1F4A9}" + "b".repeat(40 * 1024);
    const expected = new TextEncoder().encode(payload);

    connection.send(payload);

    const frames = socket.sent as Uint8Array[];
    expect(frames.length).toBeGreaterThan(1);
    for (const frame of frames) {
      expect(frame.byteLength).toBeLessThanOrEqual(32 * 1024);
    }
    const reassembled = new Uint8Array(expected.length);
    let offset = 0;
    for (const frame of frames) {
      reassembled.set(frame, offset);
      offset += frame.byteLength;
    }
    expect(offset).toBe(expected.length);
    expect(Array.from(reassembled)).toEqual(Array.from(expected));
    connection.close();
  });

  it("resize sends the exact JSON", () => {
    const { connection } = connect();
    const socket = FakeSocket.instances[0];
    socket.open();

    connection.resize({ cols: 132, rows: 41 });

    expect(socket.sent).toEqual(['{"resize":{"cols":132,"rows":41}}']);
    connection.close();
  });

  it("close cancels a pending retry", () => {
    const { connection } = connect();
    const socket = FakeSocket.instances[0];
    socket.open();
    socket.closeFromServer(1006);
    const pending = timers[0];

    connection.close();
    pending.callback();

    expect(pending.cancelled).toBe(true);
    expect(FakeSocket.instances).toHaveLength(1);
  });

  it("a superseded socket's late close is ignored", () => {
    const { connection, states } = connect();
    const first = FakeSocket.instances[0];
    connection.retry();
    const stateCount = states.length;

    first.closeFromServer(4009);

    expect(states).toHaveLength(stateCount);
    expect(timers).toHaveLength(0);
    expect(FakeSocket.instances).toHaveLength(2);
    connection.close();
  });

  it("writes binary server frames and ignores text frames", () => {
    const { connection, output } = connect();
    const socket = FakeSocket.instances[0];
    socket.open();

    socket.receive(new Uint8Array([0x1b, 0x5b, 0x41]).buffer);
    socket.receive("reserved");

    expect(output.map((bytes) => Array.from(bytes))).toEqual([[0x1b, 0x5b, 0x41]]);
    connection.close();
  });
});
