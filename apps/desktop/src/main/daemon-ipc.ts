import { EventEmitter } from "node:events";
import net from "node:net";

const PIPE_PATH = "\\\\.\\pipe\\holy-blocker-daemon";
const BACKOFF_INITIAL_MS = 500;
const BACKOFF_MAX_MS = 30_000;

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
      const lines = this.buffer.split("\n");
      // Last element is either empty or an incomplete line — keep it in the buffer
      this.buffer = lines.pop() ?? "";
      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) continue;
        try {
          const msg = JSON.parse(trimmed) as DaemonMessage;
          this.emit("message", msg);
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
