import { createContext, useContext } from "react"

/** Thrown into the caller when the user closes the authentication dialog. */
export class AuthenticationCancelled extends Error {
  constructor() {
    super("Authentication cancelled")
  }
}

export function isCancelled(error: unknown): boolean {
  return error instanceof AuthenticationCancelled
}

export type Sensitive = <T>(task: () => Promise<T>, purpose: string) => Promise<T>

export const SensitiveContext = createContext<Sensitive | null>(null)

/**
 * Runs a sensitive operation. If the backend asks for authentication, the
 * user authenticates (password, Touch ID or an explicit confirmation) and
 * the operation is retried once. `purpose` completes "Authenticate to …".
 * Provided by `SensitiveOperations`.
 */
export function useSensitive(): Sensitive {
  const value = useContext(SensitiveContext)
  if (!value) throw new Error("useSensitive must be used inside SensitiveOperations")
  return value
}
