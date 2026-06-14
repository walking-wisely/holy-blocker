import path from "node:path";
import { fileURLToPath } from "node:url";
import { app, BrowserWindow } from "electron";
import { DaemonIpc } from "./daemon-ipc.js";
import { registerIpcHandlers } from "./ipc-handlers.js";

const isDev = process.env.VITE_DEV_SERVER_URL !== undefined;
const currentDir = path.dirname(fileURLToPath(import.meta.url));

const daemonIpc = new DaemonIpc();
registerIpcHandlers(daemonIpc);
daemonIpc.start();

async function createWindow() {
  const window = new BrowserWindow({
    width: 1080,
    height: 720,
    minWidth: 900,
    minHeight: 600,
    title: "Holy Blocker",
    webPreferences: {
      preload: path.join(currentDir, "../preload/preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  if (isDev) {
    await window.loadURL(process.env.VITE_DEV_SERVER_URL as string);
  } else {
    await window.loadFile(path.join(currentDir, "../renderer/index.html"));
  }
}

app.whenReady().then(createWindow);

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    daemonIpc.stop();
    app.quit();
  }
});

app.on("activate", () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    void createWindow();
  }
});
