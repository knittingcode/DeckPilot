import type { LucideIcon } from "lucide-react"
import { type ReactNode, useEffect, useRef, useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { PasswordField } from "@/components/password-field"
import { useTranslation } from "@/i18n"
import { credentialFailure, credentialFailureMessage } from "@/lib/credential-errors"

/**
 * The password template: unlock, confirm-it's-you, and the authentication
 * of a sensitive operation. Host-owned only — plugins can never open it,
 * and the password never leaves this component except to `submit`.
 *
 * - The field is masked by default, with a show/hide button, and focused.
 * - Failures are shown inline below the field, mapped to a short message
 *   (`credential-errors.ts`); a wrong password is cleared, other failures
 *   keep it so the user can retry.
 * - While `submit` runs the field and button are disabled and a second
 *   Return is ignored; afterwards the field has the focus again.
 * - `presentation="window"` without `onCancel` is blocking (security
 *   windows); with `onCancel`, Escape and Cancel call it.
 */
export function CredentialDialog({
  presentation = "dialog",
  icon,
  title,
  description,
  submitLabel,
  busyLabel,
  submit,
  onDone,
  onCancel,
  fieldId = "password",
  children,
}: {
  presentation?: "window" | "dialog"
  icon: LucideIcon
  title: string
  description: ReactNode
  submitLabel: string
  busyLabel?: string
  submit: (password: string) => Promise<unknown>
  onDone: () => void
  onCancel?: () => void
  fieldId?: string
  /** Extra explanation below the field. */
  children?: ReactNode
}) {
  const { t } = useTranslation()
  const [password, setPassword] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const input = useRef<HTMLInputElement>(null)
  const running = useRef(false)
  useEffect(() => {
    if (!busy) input.current?.focus()
  }, [busy])
  return (
    <ModalShell
      presentation={presentation}
      size="compact"
      density="compact"
      blocking={!onCancel}
      icon={icon}
      title={title}
      description={description}
      onClose={onCancel}
      onSubmit={() => {
        if (!password || running.current) return
        running.current = true
        setBusy(true)
        setError(null)
        submit(password)
          .then(onDone)
          .catch((e: unknown) => {
            setError(credentialFailureMessage(e))
            if (credentialFailure(e) === "incorrect") setPassword("")
          })
          .finally(() => {
            running.current = false
            setBusy(false)
          })
      }}
      secondary={
        onCancel
          ? { label: t("common:actions.cancel"), onClick: onCancel, disabled: busy }
          : undefined
      }
      primary={{
        label: submitLabel,
        loadingLabel: busyLabel ?? t("security:password.checking"),
        loading: busy,
        disabled: !password,
      }}
    >
      <PasswordField
        id={fieldId}
        label={t("security:password.label")}
        value={password}
        onChange={setPassword}
        autoComplete="current-password"
        autoFocus
        revealable
        disabled={busy}
        error={error}
        reserveError
        inputRef={input}
      />
      {children}
    </ModalShell>
  )
}
