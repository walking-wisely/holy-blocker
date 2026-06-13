import { ipcMain } from "electron";
import type { DaemonIpc, DaemonMessage } from "./daemon-ipc.js";

export type ConnectionState = "connecting" | "connected" | "disconnected";

export type ScanEvent = {
  verdict: "block" | "warn" | "allow";
  score: number;
  windowTitle: string;
  at: string;
  flaggedAsFalsePositive?: boolean;
};

export type DaemonStatus = {
  state: ConnectionState;
  lastHeartbeatAt: string | null;
  watchedWindows: number;
};

const RING_BUFFER_CAP = 500;
const ringBuffer: ScanEvent[] = [];

let lastHeartbeatAt: string | null = null;
let watchedWindows = 0;

function pushToRingBuffer(event: ScanEvent): void {
  if (ringBuffer.length >= RING_BUFFER_CAP) {
    ringBuffer.shift();
  }
  ringBuffer.push(event);
}

export function registerIpcHandlers(daemonIpc: DaemonIpc): void {
  daemonIpc.on("message", (msg: DaemonMessage) => {
    if (msg.type === "heartbeat") {
      lastHeartbeatAt = msg.at;
    } else if (msg.type === "status_update") {
      watchedWindows = msg.watchedWindows;
    } else if (msg.type === "scan_event") {
      pushToRingBuffer({
        verdict: msg.verdict,
        score: msg.score,
        windowTitle: msg.windowTitle,
        at: msg.at,
      });
    }
  });

  ipcMain.handle(
    "daemon:get-status",
    (): DaemonStatus => ({
      state: daemonIpc.getState(),
      lastHeartbeatAt,
      watchedWindows,
    })
  );

  ipcMain.handle("daemon:get-events", (_event, n: number = 100): ScanEvent[] => {
    const limit = Math.min(n, ringBuffer.length);
    return ringBuffer.slice(-limit);
  });
}
