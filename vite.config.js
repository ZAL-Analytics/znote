import { defineConfig } from "vitest/config";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [tailwindcss(), sveltekit()],
  define: { "import.meta.vitest": "undefined" },
  test: {
    includeSource: ["src/**/*.ts"],
    include: ["test/**/*.test.ts"],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // The bundle is loaded from disk by the webview, not over a network, and the editor
  // (CodeMirror) is on screen at startup, so splitting the page chunk buys nothing.
  build: { chunkSizeWarningLimit: 1000 },
  // 2. tauri expects a fixed port; `beforeDevCommand` passes `--port 1420 --strictPort`,
  //    so plain `pnpm dev` keeps the Vite default (5173)
  server: {
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
