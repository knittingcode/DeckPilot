import { i18n } from "@/i18n"

import { errorCode, rawErrorMessage } from "./backend-error"

/**
 * What went wrong when a password was checked, for the prompts that ask
 * for it. Backend commands report errors with a stable code (`CommandError`
 * in `src-tauri/src/error.rs`), older builds and agents as plain strings;
 * the prompts never show the backend's text, only the message of the kind
 * it maps to, so internal details (paths, SQL, protocol versions) stay out
 * of the window.
 *
 * - `incorrect`: the password does not open the configuration
 *   (`StorageError::InvalidKey`, mapped in `commands/security.rs`).
 * - `agent`: the background agent could not be reached or answered
 *   unexpectedly (`deckpilot_ipc::ClientError`).
 * - `storage`: the configuration database is missing, damaged, too new or
 *   cannot be read (`StorageError`).
 * - `notPassword`: the configuration opens with the system key instead.
 * - `unknown`: anything else.
 */
export type CredentialFailure = "incorrect" | "agent" | "storage" | "notPassword" | "unknown"

/** The kind of each backend code; a code not listed is matched by its message. */
const CODES: Partial<Record<string, CredentialFailure>> = {
  "invalid.passwordIncorrect": "incorrect",
  "storage.invalidKey": "incorrect",
  "invalid.notPasswordProtected": "notPassword",
  "agent.notRunning": "agent",
  "agent.closed": "agent",
  "agent.io": "agent",
  "agent.protocol": "agent",
  "agent.tooOld": "agent",
  "invalid.agentNotRunning": "agent",
  "storage.database": "storage",
  "storage.document": "storage",
  "storage.unsupportedSchema": "storage",
  "storage.encryptionUnavailable": "storage",
  "storage.sqlCipherIncomplete": "storage",
  "storage.corrupt": "storage",
  "storage.migrationFailed": "storage",
  "storage.io": "storage",
  "storage.credentialsRequired.password": "storage",
  "storage.credentialsRequired.systemKey": "storage",
}

/** The English texts of older builds (and of the agent's own messages). */
const PATTERNS: [CredentialFailure, RegExp][] = [
  ["incorrect", /^the password is not correct$|could not be opened with the provided key/i],
  ["notPassword", /does not use a password/i],
  [
    "agent",
    /agent is not running|connection to the agent|communication with the agent|unexpected message from the agent|running agent .* is too old/i,
  ],
  [
    "storage",
    /^database error|stored document is invalid|schema version .* is newer|database encryption|integrity check|database upgrade could not be verified|^file system error|is needed to open the database/i,
  ],
]

export function credentialFailure(error: unknown): CredentialFailure {
  const code = errorCode(error)
  const known = code === undefined ? undefined : CODES[code]
  if (known) return known
  const message = rawErrorMessage(error)
  return PATTERNS.find(([, pattern]) => pattern.test(message))?.[0] ?? "unknown"
}

/** The message of each kind, in `errors.json` (`credential.*`). */
const MESSAGES = {
  incorrect: "errors:credential.incorrect",
  agent: "errors:credential.agent",
  storage: "errors:credential.storage",
  notPassword: "errors:credential.notPassword",
  unknown: "errors:credential.unknown",
} as const satisfies Record<CredentialFailure, string>

/** The user-facing message of a failed password check, in the window's language. */
export function credentialFailureMessage(error: unknown): string {
  return i18n.t(MESSAGES[credentialFailure(error)])
}
