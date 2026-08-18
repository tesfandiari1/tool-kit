import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],
  resolve: {
    // Order is load-bearing: Vite matches aliases by prefix, and "@" is a
    // prefix of "@ui". Listed the other way round, `@ui/x` resolves to
    // `src` + `ui/x` with the slash eaten, and fails to resolve.
    alias: {
      // The design system gets its own alias, not because the path is long but
      // because it marks a boundary: `@ui` may be imported by anything, and
      // imports nothing of ours. Extracting it to `packages/ui` later is then
      // a one-line change here.
      "@ui": fileURLToPath(new URL("./src/ui", import.meta.url)),
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
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
  test: {
    // .tsx as well as .ts: the glob is the only thing deciding whether a test
    // runs at all, and a component test added later would otherwise be
    // collected by nothing and pass by never running.
    include: ["src/**/*.test.{ts,tsx}"],
    environment: "node",
  },
}));
