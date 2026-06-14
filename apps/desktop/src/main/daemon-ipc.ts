import { EventEmitter } from "node:events";
import net from "node:net";

const PIPE_PATH = "\\\\.\\pipe\\holy-blocker-daemon";
const BACKOFF_INITIAL_MS = 500;
const BACKOFF_MAX_MS = 30_000;
// Drop the connection if the incomplete-line buffer exceeds this size.
// Protects against a rogue daemon flooding us with a line that never terminates.
const MAX_BUFFER_BYTES = 1024 * 1024; // 1 MiB

export type ConnectionState = "connecting" | "connected" | "disconnected";

export type DaemonMessage =
  | { type: "heartbeat"; at: string }
  | {
      type: "scan_event";
      verdict: "block" | "warn" | "allow";
      score: number;
      windowTitle: string;
      at: string;
    }
  | { type: "status_update"; watchedWindows: number };

type SocketFactory = () => net.Socket;

function isDaemonMessage(value: unknown): value is DaemonMessage {
  if (typeof value !== "object" || value === null) return false;
  const obj = value as Record<string, unknown>;
  switch (obj.type) {
    case "heartbeat":
      return typeof obj.at === "string";
    case "scan_event":
      return (
        (obj.verdict === "block" || obj.verdict === "warn" || obj.verdict === "allow") &&
        typeof obj.score === "number" &&
        typeof obj.windowTitle === "string" &&
        typeof obj.at === "string"
      );
    case "status_update":
      return typeof obj.watchedWindows === "number";
    default:
      return false;
  }
}

export class DaemonIpc extends EventEmitter {
  private socket: net.Socket | null = null;
  private state: ConnectionState = "disconnected";
  private backoffMs = BACKOFF_INITIAL_MS;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private stopped = false;
  private buffer = "";
  private readonly socketFactory: SocketFactory;

  constructor(socketFactory?: SocketFactory) {
    super();
    this.socketFactory = socketFactory ?? (() => net.createConnection(PIPE_PATH));
  }

  start(): void {
    this.stopped = false;
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.socket?.destroy();
    this.socket = null;
    this.setState("disconnected");
  }

  send(msg: object): void {
    if (this.socket && this.state === "connected") {
      this.socket.write(`${JSON.stringify(msg)}\n`);
    }
  }

  getState(): ConnectionState {
    return this.state;
  }

  private connect(): void {
    if (this.stopped) return;
    this.setState("connecting");
    this.buffer = "";

    const socket = this.socketFactory();
    this.socket = socket;
    socket.setEncoding("utf8");

    socket.on("connect", () => {
      this.backoffMs = BACKOFF_INITIAL_MS;
      this.setState("connected");
    });

    socket.on("data", (chunk: string) => {
      this.buffer += chunk;

      if (this.buffer.length > MAX_BUFFER_BYTES) {
        // Line buffer overflow — drop this connection; scheduleReconnect via close event.
        socket.destroy(new Error("DaemonIpc: line buffer overflow, dropping connection"));
        this.buffer = "";
        return;
      }

      const lines = this.buffer.split("\n");
      // Last element is either empty or an incomplete line — keep it in the buffer
      this.buffer = lines.pop() ?? "";
      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) continue;
        try {
          const parsed: unknown = JSON.parse(trimmed);
          if (isDaemonMessage(parsed)) {
            this.emit("message", parsed);
          }
          // Unknown message shape — silently skip; daemon protocol may have added a new type
        } catch {
          // Malformed JSON — skip
        }
      }
    });

    socket.on("close", () => {
      this.socket = null;
      this.setState("disconnected");
      this.scheduleReconnect();
    });

    socket.on("error", () => {
      // "close" fires after "error", reconnect is handled there
    });
  }

  private scheduleReconnect(): void {
    if (this.stopped) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, this.backoffMs);
    this.backoffMs = Math.min(this.backoffMs * 2, BACKOFF_MAX_MS);
  }

  private setState(state: ConnectionState): void {
    if (this.state === state) return;
    this.state = state;
    this.emit("stateChange", state);
  }
}
