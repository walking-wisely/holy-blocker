import { contextBridge, ipcRenderer } from "electron";

contextBridge.exposeInMainWorld("holyBlocker", {
  getDaemonStatus: () => ipcRenderer.invoke("daemon:get-status"),
});
