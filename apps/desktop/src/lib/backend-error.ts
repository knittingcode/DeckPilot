// Errors of the backend commands, in the window's language (docs/i18n.md,
// "Backend errors").

import { i18n } from "@/i18n"

/**
 * What a backend command rejects with (`CommandError` in
 * `src-tauri/src/error.rs`): a stable `code`, the English `message` and the
 * values the translation of the code interpolates.
 */
export interface BackendError {
  code: string
  message: string
  params?: Record<string, string | number>
}

export function isBackendError(error: unknown): error is BackendError {
  return (
    typeof error === "object" &&
    error !== null &&
    typeof (error as { code?: unknown }).code === "string" &&
    typeof (error as { message?: unknown }).message === "string"
  )
}

/** The stable code of a backend error; `undefined` for plain strings and other errors. */
export function errorCode(error: unknown): string | undefined {
  return isBackendError(error) ? error.code : undefined
}

/**
 * The English text of an error as the backend (or another source) reported
 * it, for matching and logs, never translated.
 */
export function rawErrorMessage(error: unknown): string {
  if (typeof error === "string") return error
  if (isBackendError(error)) return error.message
  if (error instanceof Error) return error.message
  return i18n.t("errors:unexpected")
}

/**
 * The translation of a backend code (`errors:<code>`), or `undefined` when
 * the window has none: no key, or a key that is a group of codes.
 */
export function translateErrorCode(
  code: string,
  params?: Record<string, string | number>,
): string | undefined {
  // Codes come from the backend, not from the sources: the dynamic keys
  // allowlist (`scripts/i18n-dynamic-keys.json`) lists their groups.
  const key = `errors:${code}`
  // With the params, so that plural keys (`_one`, `_other`) are found.
  if (!i18n.exists(key, params)) return undefined
  const translate = i18n.t as unknown as (key: string, options: object) => unknown
  const text = translate(key, { ...params, returnObjects: true })
  return typeof text === "string" ? text : undefined
}

/**
 * An error as the user reads it: a backend error in the window's language
 * when its code has a translation, otherwise its English message; plain
 * strings (older agents, tests) and `Error`s as they are.
 */
export function errorMessage(error: unknown): string {
  if (isBackendError(error)) return translateErrorCode(error.code, error.params) ?? error.message
  return rawErrorMessage(error)
}
