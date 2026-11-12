// Error messages that say what failed, why, and what to do next.

import { toast } from "sonner"

import { errorMessage } from "./api"

/** A message as a sentence: trimmed, ending with a full stop. */
export function sentence(message: string): string {
  const text = message.trim()
  if (!text) return ""
  const capital = text.charAt(0).toUpperCase() + text.slice(1)
  return /[.!?…]$/.test(capital) ? capital : `${capital}.`
}

/**
 * The reason of a failure (the backend's message) followed by what to do,
 * e.g. `The agent is not running. Start it in Settings › Agent.`
 */
export function failureDetail(error: unknown, next?: string): string {
  return [sentence(errorMessage(error)), next ? sentence(next) : ""].filter(Boolean).join(" ")
}

/**
 * One line for an inline error: what failed, why and what to do, e.g.
 * `The brightness was not changed: No device is connected.`
 */
export function failureMessage(what: string, error: unknown, next?: string): string {
  return `${what}: ${failureDetail(error, next)}`
}

/** A toast for a failed operation: what failed as title, why and what to do below. */
export function toastFailure(what: string, error: unknown, next?: string) {
  toast.error(what, { description: failureDetail(error, next) })
}
