import { Fingerprint, KeyRound, ShieldAlert } from "lucide-react"
import { type ReactNode, useCallback, useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { CredentialDialog } from "@/components/dialogs/credential-dialog"
import { type TranslationKey, useTranslation } from "@/i18n"
import { api, isAuthenticationRequired } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import { AuthenticationCancelled, type Sensitive, SensitiveContext } from "@/lib/sensitive"
import type { Verification } from "@/lib/types"

interface Pending {
  purpose: string
  resolve: () => void
  reject: (error: unknown) => void
}

/** Provides `useSensitive` and renders the authentication dialog. */
export function SensitiveOperations({
  verification,
  children,
}: {
  verification: Verification
  children: ReactNode
}) {
  const [pending, setPending] = useState<Pending | null>(null)

  const run = useCallback<Sensitive>(async (task, purpose) => {
    try {
      return await task()
    } catch (error) {
      if (!isAuthenticationRequired(error)) throw error
      await new Promise<void>((resolve, reject) => {
        setPending({ purpose, resolve, reject })
      })
      return task()
    }
  }, [])

  return (
    <SensitiveContext value={run}>
      {children}
      {pending && (
        <AuthenticateDialog
          verification={verification}
          purpose={pending.purpose}
          onDone={() => {
            pending.resolve()
            setPending(null)
          }}
          onCancel={() => {
            pending.reject(new AuthenticationCancelled())
            setPending(null)
          }}
        />
      )}
    </SensitiveContext>
  )
}

const COPY = {
  password: {
    title: "security:authenticate.password.title",
    text: "security:authenticate.password.text",
    action: "security:authenticate.password.action",
  },
  touchId: {
    title: "security:authenticate.touchId.title",
    text: "security:authenticate.touchId.text",
    action: "security:authenticate.touchId.action",
  },
  devicePassword: {
    title: "security:authenticate.devicePassword.title",
    text: "security:authenticate.devicePassword.text",
    action: "security:authenticate.devicePassword.action",
  },
  confirmationOnly: {
    title: "security:authenticate.confirmationOnly.title",
    text: "security:authenticate.confirmationOnly.text",
    action: "security:authenticate.confirmationOnly.action",
  },
} as const satisfies Record<Verification, Record<"title" | "text" | "action", TranslationKey>>

/**
 * Asks for the authentication of one sensitive operation: the password
 * (the shared credential template) or the system check.
 */
export function AuthenticateDialog({
  verification,
  purpose,
  onDone,
  onCancel,
}: {
  verification: Verification
  purpose: string
  onDone: () => void
  onCancel: () => void
}) {
  const { t } = useTranslation()
  const copy = COPY[verification]
  if (verification === "password") {
    return (
      <CredentialDialog
        icon={KeyRound}
        title={t(copy.title)}
        description={t("security:authenticate.purpose", { purpose })}
        submitLabel={t(copy.action)}
        fieldId="authenticate-password"
        submit={(password) => api.authenticate(password)}
        onDone={onDone}
        onCancel={onCancel}
      />
    )
  }
  return (
    <SystemAuthentication
      verification={verification}
      purpose={purpose}
      onDone={onDone}
      onCancel={onCancel}
    />
  )
}

/** Touch ID, the login password in the system prompt, or a confirmation. */
function SystemAuthentication({
  verification,
  purpose,
  onDone,
  onCancel,
}: {
  verification: Exclude<Verification, "password">
  purpose: string
  onDone: () => void
  onCancel: () => void
}) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const copy = COPY[verification]
  const submit = () => {
    setBusy(true)
    setError(null)
    api
      .authenticate(null)
      .then(onDone, (e: unknown) => {
        setError(failureMessage(t("errors:security.notVerified"), e, t("errors:next.tryAgain")))
      })
      .finally(() => {
        setBusy(false)
      })
  }
  return (
    <ModalShell
      presentation="dialog"
      size="compact"
      density="compact"
      icon={verification === "touchId" ? Fingerprint : ShieldAlert}
      title={t(copy.title)}
      description={t("security:authenticate.purpose", { purpose })}
      error={error}
      onClose={onCancel}
      onSubmit={submit}
      secondary={{ label: t("common:actions.cancel"), onClick: onCancel, disabled: busy }}
      primary={{ label: t(copy.action), loadingLabel: t("common:status.waiting"), loading: busy }}
    >
      <p className="text-sm text-muted-foreground">{t(copy.text)}</p>
    </ModalShell>
  )
}
