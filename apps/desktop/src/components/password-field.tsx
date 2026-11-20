import { Eye, EyeOff } from "lucide-react"
import { type Ref, useState } from "react"

import { FieldError } from "@/components/field-error"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useTranslation } from "@/i18n"

import { MIN_PASSWORD } from "@/lib/passwords"

/**
 * A labelled password input. The value lives in component state only; it
 * is never logged or persisted. Masked by default; with `revealable` a
 * button shows and hides it. With `error`, the message appears inline
 * below the field (see `FieldError`); `reserveError` keeps its line so
 * the layout does not jump when it appears.
 */
export function PasswordField({
  id,
  label,
  value,
  onChange,
  autoComplete,
  autoFocus = false,
  invalid = false,
  disabled = false,
  revealable = false,
  error,
  reserveError = false,
  inputRef,
}: {
  id: string
  label: string
  value: string
  onChange: (value: string) => void
  autoComplete: "current-password" | "new-password"
  autoFocus?: boolean
  invalid?: boolean
  disabled?: boolean
  revealable?: boolean
  error?: string | null
  reserveError?: boolean
  inputRef?: Ref<HTMLInputElement>
}) {
  const { t } = useTranslation()
  const [revealed, setRevealed] = useState(false)
  const errorId = `${id}-error`
  const showsError = Boolean(error)
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      <div className="relative">
        <Input
          ref={inputRef}
          id={id}
          type={revealable && revealed ? "text" : "password"}
          autoComplete={autoComplete}
          autoFocus={autoFocus}
          // A revealed password is still a password: no spelling or
          // capitalisation help that could keep or change it.
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          disabled={disabled}
          aria-invalid={invalid || showsError || undefined}
          aria-describedby={showsError ? errorId : undefined}
          className={revealable ? "pr-9" : undefined}
          value={value}
          onChange={(e) => {
            onChange(e.target.value)
          }}
        />
        {revealable && (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="absolute top-0.5 right-0.5 size-8 text-muted-foreground hover:text-foreground"
            aria-label={revealed ? t("security:password.hide") : t("security:password.show")}
            aria-controls={id}
            aria-pressed={revealed}
            disabled={disabled}
            onClick={() => {
              setRevealed((r) => !r)
            }}
          >
            {revealed ? <EyeOff aria-hidden /> : <Eye aria-hidden />}
          </Button>
        )}
      </div>
      <FieldError id={errorId} message={error} reserve={reserveError} className="-mt-0.5" />
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
  const { t } = useTranslation()
  const tooShort = password.length > 0 && password.length < MIN_PASSWORD
  const mismatch = confirmation.length > 0 && confirmation !== password
  return (
    <>
      <PasswordField
        id={idPrefix}
        label={t("security:password.label")}
        value={password}
        onChange={onPassword}
        autoComplete="new-password"
        autoFocus={autoFocus}
        invalid={tooShort}
      />
      <PasswordField
        id={`${idPrefix}-confirmation`}
        label={t("security:password.repeat")}
        value={confirmation}
        onChange={onConfirmation}
        autoComplete="new-password"
        invalid={mismatch}
      />
      {tooShort && (
        <p className="text-sm text-destructive">
          {t("security:password.tooShort", { count: MIN_PASSWORD })}
        </p>
      )}
      {mismatch && <p className="text-sm text-destructive">{t("security:password.mismatch")}</p>}
    </>
  )
}
