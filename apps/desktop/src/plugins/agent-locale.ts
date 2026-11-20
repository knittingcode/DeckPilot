// The app's language for the agent (IPC protocol 13, `setLocale`): plugin
// texts (`t:` keys of a plugin's own verified locale files), the texts the
// agent draws on keys and its tray then follow the window's language. Only
// the language tag crosses (`en`, `pl`); the agent announces the plugins
// whose texts changed with `changed` plugin events, which the plugin views
// already reload on. An agent of protocol 12 or older is not told and keeps
// English (`docs/i18n.md`, `docs/compatibility.md`).

import { invoke } from "@tauri-apps/api/core"

import { currentLanguage, i18n } from "@/i18n"
import { inTauri } from "@/lib/window-sync"

/** Backend command that stores the language and tells the agent. */
export const SET_AGENT_LOCALE = "set_agent_locale"

function send(tag: string) {
  // Best effort: the backend sends it again whenever it connects.
  invoke(SET_AGENT_LOCALE, { tag }).catch(() => undefined)
}

/**
 * Tells the agent the window's language now and after every change.
 * Returns the function that stops following. Outside the app (a plain
 * browser, the tests) it does nothing.
 */
export function startAgentLocaleSync(): () => void {
  if (!inTauri()) return () => undefined
  send(currentLanguage())
  const onChange = () => {
    send(currentLanguage())
  }
  i18n.on("languageChanged", onChange)
  return () => {
    i18n.off("languageChanged", onChange)
  }
}
