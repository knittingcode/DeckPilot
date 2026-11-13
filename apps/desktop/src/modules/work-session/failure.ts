// Error messages that say what failed, why, and what to do next: the
// pattern of #71/#74 (`lib/failure.ts` there). Switch to that module once it
// has landed; the functions have the same names and behaviour.

import { toast } from "sonner"

import { errorMessage } from "@/lib/api"

/** A message as a sentence: trimmed, ending with a full stop. */
export function sentence(message: string): string {
  const text = message.trim()
  if (!text) return ""
  const capital = text.charAt(0).toUpperCase() + text.slice(1)
  return /[.!?…]$/.test(capital) ? capital : `${capital}.`
}

/** The reason of a failure (the backend's message) followed by what to do. */
export function failureDetail(error: unknown, next?: string): string {
  return [sentence(errorMessage(error)), next ? sentence(next) : ""].filter(Boolean).join(" ")
}

/** One line for an inline error: what failed, why and what to do. */
export function failureMessage(what: string, error: unknown, next?: string): string {
  return `${what}: ${failureDetail(error, next)}`
}

/** A toast for a failed operation: what failed as title, why and what to do below. */
export function toastFailure(what: string, error: unknown, next?: string) {
  toast.error(what, { description: failureDetail(error, next) })
}
