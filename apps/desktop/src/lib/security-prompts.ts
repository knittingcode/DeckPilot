import type { SecurityView } from "./types"

/** The prompt window that unlocks the configuration in its current state. */
export function promptWindow(view: SecurityView): string {
  if (view.database === "missing") return "security.setup"
  if (view.database === "plaintext") return "security.migrate"
  if (view.confirmationRequired) return "security.confirm"
  return "security.unlock"
}
