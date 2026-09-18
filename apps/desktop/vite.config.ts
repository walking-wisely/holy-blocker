import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  root: "src/renderer",
  build: {
    outDir: "../../dist/renderer",
    emptyOutDir: true,
  },
  server: {
    // Portless (see `pnpm dev` script) injects PORT per worktree; fall back
    // to 5173 for a plain, non-Portless run.
    port: process.env.PORT ? Number(process.env.PORT) : 5173,
  },
});
