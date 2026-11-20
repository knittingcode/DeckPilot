/**
 * Development only: lets a local script measure the layout of the running
 * app (the real WKWebView/WebView2/WebKitGTK of `tauri dev`) and select a key
 * or scroll, through the Vite dev server. Used by `scripts/layout-check.mjs`
 * and for UI reviews; see docs/development.md, "Layout probe".
 *
 * - Exists only in `vite dev` (not builds, not Vitest).
 * - The endpoint needs a random token, written to `$DECKPILOT_DEV_DIR` (or
 *   `<repo>/.dev`) as `layout-probe.token` with mode 600, and the custom
 *   header `x-deckpilot-probe`: a web page in a browser cannot send that
 *   header to localhost without a CORS preflight, which this server never
 *   allows.
 * - Only the fixed commands below exist; there is no way to run code in the
 *   app. Arguments are validated here and again in the app.
 */
import { randomBytes } from "node:crypto"
import { mkdirSync, writeFileSync } from "node:fs"
import path from "node:path"

import type { Plugin } from "vite"

export const PROBE_COMMANDS = [
  "measure",
  "keys",
  "select",
  "scroll",
  "navigate",
  "canvas",
  "page",
  "dock",
  "gestures",
  "title",
  "live",
  "dblclick",
  "edit",
  "keymenu",
] as const
const VIEWS = ["designer", "marketplace", "settings", "plugin"]

const SELECT_KEY = /^[\w:.-]{1,64}$/
const SLOTS = ["sidebar-inset", "device-editor-main", "inspector-scroll", "library-scroll"]
const CANVAS_ACTIONS = [
  "zoom-in",
  "zoom-out",
  "fit",
  "actual",
  "pan",
  "pointer-focus",
  "drag",
  "state",
]
const PAGES = ["next", "previous"]
const DOCK = ["open", "close", "escape", "rename", "rename-cancel"]
const TITLE = ["open", "close", "escape"]
const EDITS = ["clear", "undo", "redo", "revert", "save"]

function validate(command: unknown, args: unknown): string | null {
  if (typeof command !== "string" || !(PROBE_COMMANDS as readonly string[]).includes(command)) {
    return "unknown command"
  }
  const a = (args ?? {}) as Record<string, unknown>
  if (command === "select" && (typeof a.control !== "string" || !SELECT_KEY.test(a.control))) {
    return "select needs a control id"
  }
  if (
    command === "scroll" &&
    (typeof a.slot !== "string" ||
      !SLOTS.includes(a.slot) ||
      !["top", "bottom"].includes(String(a.to)))
  ) {
    return "scroll needs a known slot and top|bottom"
  }
  if (command === "canvas" && !CANVAS_ACTIONS.includes(String(a.action))) {
    return `canvas needs an action: ${CANVAS_ACTIONS.join(", ")}`
  }
  const bounded = (v: unknown) => typeof v === "number" && Number.isFinite(v) && Math.abs(v) <= 2000
  if (
    command === "canvas" &&
    a.action === "drag" &&
    (!bounded(a.dx) || !bounded(a.dy) || ![0, 1].includes(Number(a.button ?? 0)))
  ) {
    return "drag needs dx, dy (±2000) and button 0|1"
  }
  if (command === "page" && !PAGES.includes(String(a.to))) {
    return "page needs next|previous"
  }
  if (command === "dock" && !DOCK.includes(String(a.to))) {
    return "dock needs open|close|escape|rename|rename-cancel"
  }
  if (command === "title" && !TITLE.includes(String(a.to))) {
    return "title needs open|close|escape"
  }
  if (command === "dblclick" && (typeof a.control !== "string" || !SELECT_KEY.test(a.control))) {
    return "dblclick needs a control id"
  }
  if (command === "edit" && !EDITS.includes(String(a.to))) {
    return `edit needs ${EDITS.join("|")}`
  }
  if (command === "gestures" && !PAGES.includes(String(a.to))) {
    return "gestures needs next|previous"
  }
  if (command === "navigate" && !VIEWS.includes(String(a.view))) {
    return `navigate needs a view: ${VIEWS.join(", ")}`
  }
  return null
}

export function layoutProbe(): Plugin {
  return {
    name: "deckpilot-layout-probe",
    // `vite dev` only: never in builds, and not in Vitest, which starts its
    // own server with this config and would replace the session's token.
    apply: (_config, env) => env.command === "serve" && !process.env.VITEST,
    configureServer(server) {
      const token = randomBytes(24).toString("hex")
      const devDir = process.env.DECKPILOT_DEV_DIR ?? path.resolve(server.config.root, "../../.dev")
      mkdirSync(devDir, { recursive: true })
      writeFileSync(path.join(devDir, "layout-probe.token"), token, { mode: 0o600 })

      const pending = new Map<string, (result: unknown) => void>()
      server.ws.on("deckpilot:probe-result", (data: { id?: string; result?: unknown }) => {
        if (typeof data.id === "string") pending.get(data.id)?.(data.result)
      })

      server.middlewares.use("/__deckpilot/probe", (req, res) => {
        const reply = (status: number, body: unknown) => {
          res.statusCode = status
          res.setHeader("content-type", "application/json")
          res.end(JSON.stringify(body))
        }
        if (req.method !== "POST" || req.headers["x-deckpilot-probe"] !== token) {
          reply(403, { error: "forbidden" })
          return
        }
        let raw = ""
        req.on("data", (chunk: Buffer) => {
          raw += chunk.toString()
          if (raw.length > 4096) req.destroy()
        })
        req.on("end", () => {
          let body: { command?: unknown; args?: unknown }
          try {
            body = JSON.parse(raw) as typeof body
          } catch {
            reply(400, { error: "invalid JSON" })
            return
          }
          const problem = validate(body.command, body.args)
          if (problem) {
            reply(400, { error: problem })
            return
          }
          const id = randomBytes(8).toString("hex")
          const timer = setTimeout(() => {
            pending.delete(id)
            reply(504, { error: "the app did not answer (is the main window open?)" })
          }, 5000)
          pending.set(id, (result) => {
            clearTimeout(timer)
            pending.delete(id)
            reply(200, result)
          })
          server.ws.send("deckpilot:probe", { id, command: body.command, args: body.args ?? {} })
        })
      })
    },
  }
}
