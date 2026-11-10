import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import { App } from "./App"
import "./index.css"
import { WindowApp } from "./windows/window-app"

const root = document.getElementById("root")
if (!root) throw new Error("root element missing")

// Native windows other than the main one load the same bundle with
// `?window=<identifier>`.
const windowId = new URLSearchParams(window.location.search).get("window")

createRoot(root).render(<StrictMode>{windowId ? <WindowApp id={windowId} /> : <App />}</StrictMode>)
