import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import { App } from "./App"
import "./index.css"

// Follow the operating system's light or dark appearance.
const darkScheme = window.matchMedia("(prefers-color-scheme: dark)")
const applyScheme = () => document.documentElement.classList.toggle("dark", darkScheme.matches)
applyScheme()
darkScheme.addEventListener("change", applyScheme)

const root = document.getElementById("root")
if (!root) throw new Error("root element missing")

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
