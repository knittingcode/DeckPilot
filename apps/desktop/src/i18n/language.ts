// The window's language: the stored preference resolved against the
// operating system's languages, kept in step across windows.

import { useSyncExternalStore } from "react"

import { LANGUAGE_EVENT, announce, onAnnouncement } from "@/lib/window-sync"

import type { Language } from "./config"
import { applyLanguage } from "./index"
import {
  type LanguagePreference,
  isLanguagePreference,
  readLanguagePreference,
  writeLanguagePreference,
} from "./preference"
import { resolveLocale } from "./resolve"
import { systemLocales } from "./system-locale"

let preference: LanguagePreference = readLanguagePreference()
const listeners = new Set<() => void>()
// Only the latest request applies; an earlier, slower lookup of the
// system's languages never overrides a later choice.
let generation = 0

function setPreferenceState(next: LanguagePreference) {
  if (next === preference) return
  preference = next
  for (const listener of listeners) listener()
}

/** The language `preference` stands for on this computer. */
export async function effectiveLanguage(
  value: LanguagePreference,
  locales: () => Promise<string[]> = systemLocales,
): Promise<Language> {
  return value === "system" ? resolveLocale(await locales()) : value
}

async function show(value: LanguagePreference) {
  const request = ++generation
  const language = await effectiveLanguage(value)
  if (request === generation) await applyLanguage(language)
}

/**
 * Shows the stored language. `main.tsx` awaits it before the first render,
 * so no window starts in English and switches (the system's languages are
 * read in a few milliseconds).
 */
export async function initLanguage() {
  setPreferenceState(readLanguagePreference())
  await show(preference)
}

/** Stores the preference and shows it in this and every other window. */
export async function setLanguagePreference(next: LanguagePreference) {
  writeLanguagePreference(next)
  setPreferenceState(next)
  announce(LANGUAGE_EVENT, { preference: next })
  await show(next)
}

/**
 * Follows changes made elsewhere: another window's choice (announced, see
 * `window-sync.ts`) and, while the preference is `system`, a change of the
 * system's languages, read again whenever the window gains the focus
 * (Windows reports a new list to running apps; macOS and Linux give an app
 * its languages when it starts, so there a system change applies at the
 * next start; docs/i18n.md). Returns the function that stops following.
 */
export function startLanguageSync(): () => void {
  const stopAnnouncements = onAnnouncement(LANGUAGE_EVENT, (payload) => {
    if (!isLanguagePreference(payload.preference)) return
    setPreferenceState(payload.preference)
    void show(payload.preference)
  })
  const refresh = () => {
    // The stored choice is the truth: an announcement may have been missed.
    setPreferenceState(readLanguagePreference())
    void show(preference)
  }
  if (typeof window === "undefined") return stopAnnouncements
  window.addEventListener("focus", refresh)
  // Browsers (and the tests) report a change of the user's languages.
  window.addEventListener("languagechange", refresh)
  return () => {
    stopAnnouncements()
    window.removeEventListener("focus", refresh)
    window.removeEventListener("languagechange", refresh)
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/** The stored preference, updated when it changes in any window. */
export function useLanguagePreference(): LanguagePreference {
  return useSyncExternalStore(
    subscribe,
    () => preference,
    () => preference,
  )
}
