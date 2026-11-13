/**
 * What a page should show first when another page opens it, e.g. the
 * Action Library opening the workflow or layout that uses an action.
 *
 * The opening page sets the target and navigates; the opened page reads it
 * once when it mounts (`useFocusTarget`) and clears it.
 */

import { useEffect, useState } from "react"

export type FocusTarget =
  { type: "workflow"; workflow: string } | { type: "layout"; layout: string; page?: string }

let pending: FocusTarget | null = null

export function requestFocus(target: FocusTarget) {
  pending = target
}

export function peekFocus<T extends FocusTarget["type"]>(
  type: T,
): Extract<FocusTarget, { type: T }> | null {
  return pending?.type === type ? (pending as Extract<FocusTarget, { type: T }>) : null
}

export function clearFocus(type: FocusTarget["type"]) {
  if (pending?.type === type) pending = null
}

/** The target for this page, read when it mounts. */
export function useFocusTarget<T extends FocusTarget["type"]>(type: T) {
  // Read in the initializer and cleared after mounting: initializers may run twice.
  const [target] = useState(() => peekFocus(type))
  useEffect(() => {
    clearFocus(type)
  }, [type])
  return target
}
