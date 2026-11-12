import path from "node:path"

import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vitest/config"

const host = process.env.TAURI_DEV_HOST
// A second development session (e.g. the integration preview) uses its own
// port, with a Tauri configuration whose `devUrl` matches it
// (DECKPILOT_TAURI_CONFIG, see docs/development.md).
const port = Number(process.env.DECKPILOT_VITE_PORT ?? 1420)

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(import.meta.dirname, "./src") },
  },
  clearScreen: false,
  server: {
    port,
    strictPort: true,
    host: host ?? false,
    hmr: host ? { protocol: "ws", host, port: port + 1 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari15",
    minify: process.env.TAURI_ENV_DEBUG ? false : "oxc",
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
  },
})
