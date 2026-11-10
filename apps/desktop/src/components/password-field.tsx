import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

import { MIN_PASSWORD } from "@/lib/passwords"

/** A labelled password input. The value lives in component state only. */
export function PasswordField({
  id,
  label,
  value,
  onChange,
  autoComplete,
  autoFocus = false,
  invalid = false,
}: {
  id: string
  label: string
  value: string
  onChange: (value: string) => void
  autoComplete: "current-password" | "new-password"
  autoFocus?: boolean
  invalid?: boolean
}) {
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="password"
        autoComplete={autoComplete}
        autoFocus={autoFocus}
        aria-invalid={invalid || undefined}
        value={value}
        onChange={(e) => {
          onChange(e.target.value)
        }}
      />
    </div>
  )
}

/** A new password entered twice, with inline validation. */
export function NewPasswordFields({
  password,
  confirmation,
  onPassword,
  onConfirmation,
  autoFocus = false,
  idPrefix = "new-password",
}: {
  password: string
  confirmation: string
  onPassword: (value: string) => void
  onConfirmation: (value: string) => void
  autoFocus?: boolean
  idPrefix?: string
}) {
  const tooShort = password.length > 0 && password.length < MIN_PASSWORD
  const mismatch = confirmation.length > 0 && confirmation !== password
  return (
    <>
      <PasswordField
        id={idPrefix}
        label="Password"
        value={password}
        onChange={onPassword}
        autoComplete="new-password"
        autoFocus={autoFocus}
        invalid={tooShort}
      />
      <PasswordField
        id={`${idPrefix}-confirmation`}
        label="Repeat password"
        value={confirmation}
        onChange={onConfirmation}
        autoComplete="new-password"
        invalid={mismatch}
      />
      {tooShort && (
        <p className="text-sm text-destructive">Use at least {MIN_PASSWORD} characters.</p>
      )}
      {mismatch && <p className="text-sm text-destructive">The passwords do not match.</p>}
    </>
  )
}
