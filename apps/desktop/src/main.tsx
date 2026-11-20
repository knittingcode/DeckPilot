import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import { App } from "./App"
import { initLanguage, startLanguageSync } from "./i18n/language"
import "./index.css"
import { startAgentLocaleSync } from "./plugins/agent-locale"
import { WindowApp } from "./windows/window-app"

const root = document.getElementById("root")
if (!root) throw new Error("root element missing")

// Native windows other than the main one load the same bundle with
// `?window=<identifier>`.
const windowId = new URLSearchParams(window.location.search).get("window")
if (windowId) document.documentElement.dataset.window = windowId

// Development only: `?showcase=dialogs` renders the Dialog Showcase on its
// own, so it can be reviewed in any browser. Vite
// removes this branch and the showcase from builds.
const showcase =
  import.meta.env.DEV && new URLSearchParams(window.location.search).get("showcase") === "dialogs"

// Development only: the layout probe of `vite dev` (dev/layout-probe-plugin.ts),
// answered by the main window.
if (import.meta.env.DEV && !windowId && !showcase) {
  void import("./dev/layout-probe").then(({ startLayoutProbe }) => {
    startLayoutProbe()
  })
}

// The window's language is known before its first render (docs/i18n.md),
// so a native window such as the unlock prompt never shows English first.
// Reading the system's languages takes milliseconds and cannot fail.
async function start(element: HTMLElement) {
  await initLanguage()
  startLanguageSync()
  // The main window tells the agent the app's language (plugin texts, keys
  // and tray follow it).
  if (!windowId && !showcase) startAgentLocaleSync()
  if (showcase) {
    const { StandaloneShowcase } = await import("./dev/standalone-showcase")
    createRoot(element).render(
      <StrictMode>
        <StandaloneShowcase />
      </StrictMode>,
    )
    return
  }
  createRoot(element).render(
    <StrictMode>{windowId ? <WindowApp id={windowId} /> : <App />}</StrictMode>,
  )
}

void start(root)
