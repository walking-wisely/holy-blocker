import net from "node:net";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { type ConnectionState, DaemonIpc, type DaemonMessage } from "../daemon-ipc.js";

function startServer(): Promise<{ server: net.Server; port: number }> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address() as net.AddressInfo;
      resolve({ server, port: addr.port });
    });
    server.on("error", reject);
  });
}

function makeTcpIpc(port: number): DaemonIpc {
  return new DaemonIpc(() => net.createConnection({ port, host: "127.0.0.1" }));
}

/** Resolves when the next `stateChange` event fires with the given state. */
function waitForState(ipc: DaemonIpc, target: ConnectionState): Promise<void> {
  if (ipc.getState() === target) return Promise.resolve();
  return new Promise((resolve) => {
    const handler = (s: ConnectionState) => {
      if (s === target) {
        ipc.off("stateChange", handler);
        resolve();
      }
    };
    ipc.on("stateChange", handler);
  });
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

describe("DaemonIpc", () => {
  let server: net.Server;
  let port: number;
  let ipc: DaemonIpc;

  beforeEach(async () => {
    ({ server, port } = await startServer());
  });

  afterEach(async () => {
    ipc?.stop();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  it("transitions to connected when the server is available", async () => {
    ipc = makeTcpIpc(port);
    const states: ConnectionState[] = [];
    ipc.on("stateChange", (s: ConnectionState) => states.push(s));

    ipc.start();
    await waitForState(ipc, "connected");

    expect(states).toContain("connecting");
    expect(states).toContain("connected");
    expect(ipc.getState()).toBe("connected");
  });

  it("parses newline-delimited JSON messages", async () => {
    const messages: DaemonMessage[] = [];

    server.on("connection", (conn) => {
      conn.write(`${JSON.stringify({ type: "heartbeat", at: "2024-01-01T00:00:00Z" })}\n`);
      conn.write(
        `${JSON.stringify({
          type: "scan_event",
          verdict: "block",
          score: 90,
          windowTitle: "bad site",
          at: "2024-01-01T00:00:01Z",
        })}\n`
      );
    });

    ipc = makeTcpIpc(port);
    ipc.on("message", (msg: DaemonMessage) => messages.push(msg));
    ipc.start();

    await waitForState(ipc, "connected");
    await new Promise<void>((resolve) => {
      const check = () => (messages.length >= 2 ? resolve() : setTimeout(check, 10));
      check();
    });

    expect(messages[0]).toEqual({ type: "heartbeat", at: "2024-01-01T00:00:00Z" });
    expect(messages[1]).toEqual({
      type: "scan_event",
      verdict: "block",
      score: 90,
      windowTitle: "bad site",
      at: "2024-01-01T00:00:01Z",
    });
  });

  it("handles messages split across multiple data chunks", async () => {
    const messages: DaemonMessage[] = [];

    server.on("connection", (conn) => {
      const full = `${JSON.stringify({ type: "heartbeat", at: "2024-01-01T00:00:00Z" })}\n`;
      conn.write(full.slice(0, 10));
      setTimeout(() => conn.write(full.slice(10)), 20);
    });

    ipc = makeTcpIpc(port);
    ipc.on("message", (msg: DaemonMessage) => messages.push(msg));
    ipc.start();

    await waitForState(ipc, "connected");
    await new Promise<void>((resolve) => {
      const check = () => (messages.length >= 1 ? resolve() : setTimeout(check, 10));
      check();
    });

    expect(messages[0]).toEqual({ type: "heartbeat", at: "2024-01-01T00:00:00Z" });
  });

  it("ignores malformed JSON lines without throwing", async () => {
    const messages: DaemonMessage[] = [];

    server.on("connection", (conn) => {
      conn.write("not-json\n");
      conn.write(`${JSON.stringify({ type: "status_update", watchedWindows: 3 })}\n`);
    });

    ipc = makeTcpIpc(port);
    ipc.on("message", (msg: DaemonMessage) => messages.push(msg));
    ipc.start();

    await waitForState(ipc, "connected");
    await new Promise<void>((resolve) => {
      const check = () => (messages.length >= 1 ? resolve() : setTimeout(check, 10));
      check();
    });

    expect(messages).toHaveLength(1);
    expect(messages[0]).toEqual({ type: "status_update", watchedWindows: 3 });
  });

  it("send() writes a newline-terminated JSON message to the server", async () => {
    const received: string[] = [];

    server.on("connection", (conn) => {
      conn.setEncoding("utf8");
      conn.on("data", (chunk: string) => received.push(chunk));
    });

    ipc = makeTcpIpc(port);
    ipc.start();
    await waitForState(ipc, "connected");

    ipc.send({ type: "config_update", blockThreshold: 75 });
    await delay(30);

    const joined = received.join("");
    expect(joined).toContain('"type":"config_update"');
    expect(joined.at(-1)).toBe("\n");
  });

  it("reconnects after the server drops the connection", async () => {
    let connectionCount = 0;
    server.on("connection", (conn) => {
      connectionCount++;
      if (connectionCount === 1) {
        setTimeout(() => conn.destroy(), 20);
      }
    });

    ipc = makeTcpIpc(port);
    ipc.start();

    await waitForState(ipc, "connected");
    await waitForState(ipc, "disconnected");

    // Wait for reconnect (initial backoff = 500ms)
    await new Promise<void>((resolve) => {
      const check = () => (connectionCount >= 2 ? resolve() : setTimeout(check, 50));
      setTimeout(check, 50);
    });

    expect(connectionCount).toBeGreaterThanOrEqual(2);
  }, 3000);

  it("does not reconnect after stop() is called", async () => {
    let connectionCount = 0;
    server.on("connection", (conn) => {
      connectionCount++;
      conn.destroy();
    });

    ipc = makeTcpIpc(port);
    ipc.start();

    await waitForState(ipc, "connecting");
    await delay(150); // let connect + destroy cycle complete
    ipc.stop();
    const countAfterStop = connectionCount;

    await delay(700); // longer than initial 500ms backoff
    expect(connectionCount).toBe(countAfterStop);
    expect(ipc.getState()).toBe("disconnected");
  }, 3000);
});
