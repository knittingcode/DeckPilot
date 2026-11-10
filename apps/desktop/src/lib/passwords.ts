/** Minimum length of DeckPilot passwords (checked again by the backend). */
export const MIN_PASSWORD = 8

/** Whether a new password and its repetition can be submitted. */
export function newPasswordReady(password: string, confirmation: string): boolean {
  return password.length >= MIN_PASSWORD && password === confirmation
}
