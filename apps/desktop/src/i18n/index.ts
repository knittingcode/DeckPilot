/**
 * DeckPilot's translations (docs/i18n.md): one i18next instance for the
 * window, with every language bundled. It is initialised in English when
 * this module loads, synchronously, so any component (and any test) can
 * translate right away; `main.tsx` switches to the user's language before
 * the first render (`initLanguage`).
 *
 * Components use `useTranslation` and `Trans` from here, which guarantees
 * the instance exists. Changing the language re-renders what uses them; it
 * never remounts the app, so unsaved edits and open dialogs stay.
 */

import i18next from "i18next"
import { initReactI18next } from "react-i18next"

import {
  FALLBACK_LANGUAGE,
  type Language,
  NAMESPACES,
  type Namespace,
  SUPPORTED_LANGUAGES,
} from "./config"
import { type en, resources } from "./resources"

export const i18n = i18next.createInstance()

type PluralSuffix = "_zero" | "_one" | "_two" | "_few" | "_many" | "_other"
type BaseKey<K extends string> = K extends `${infer Base}${PluralSuffix}` ? Base : K
type Leaves<T, Prefix extends string = ""> = {
  [K in keyof T & string]: T[K] extends string
    ? `${Prefix}${BaseKey<K>}`
    : Leaves<T[K], `${Prefix}${K}.`>
}[keyof T & string]

/**
 * Any key of any namespace, `"<namespace>:<key>"` (plural keys without
 * their suffix): for keys kept in data and translated later with `t`.
 */
export type TranslationKey = {
  [N in Namespace]: `${N}:${Leaves<(typeof en)[N]>}`
}[Namespace]

void i18n.use(initReactI18next).init({
  resources,
  lng: FALLBACK_LANGUAGE,
  fallbackLng: FALLBACK_LANGUAGE,
  supportedLngs: SUPPORTED_LANGUAGES,
  // `pl-PL` is shown as `pl`; only language-wide resources exist.
  nonExplicitSupportedLngs: true,
  load: "languageOnly",
  ns: NAMESPACES,
  defaultNS: "common",
  // React escapes what it renders.
  interpolation: { escapeValue: false },
  returnNull: false,
  // Everything is bundled: initialise synchronously, never suspend.
  initAsync: false,
  react: { useSuspense: false },
})

/** The language the window shows. */
export function currentLanguage(): Language {
  const language = i18n.resolvedLanguage ?? i18n.language
  return (SUPPORTED_LANGUAGES as readonly string[]).includes(language)
    ? (language as Language)
    : FALLBACK_LANGUAGE
}

/** Shows `language` in this window: the texts and `<html lang>`. */
export async function applyLanguage(language: Language) {
  if (typeof document !== "undefined") document.documentElement.lang = language
  if (i18n.language !== language) await i18n.changeLanguage(language)
}

export { Trans, useTranslation } from "react-i18next"
export type { Language } from "./config"
