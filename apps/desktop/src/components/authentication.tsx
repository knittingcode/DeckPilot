import { Fingerprint, KeyRound, ShieldAlert } from "lucide-react"
import { type ReactNode, useCallback, useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { PasswordField } from "@/components/password-field"
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

const COPY: Record<Verification, { title: string; text: string; action: string }> = {
  password: {
    title: "Enter your password",
    text: "Your DeckPilot password confirms this change.",
    action: "Confirm",
  },
  touchId: {
    title: "Confirm it's you",
    text: "Use Touch ID or your Mac's login password.",
    action: "Continue with Touch ID",
  },
  devicePassword: {
    title: "Confirm it's you",
    text: "Your computer asks for your login password.",
    action: "Continue",
  },
  confirmationOnly: {
    title: "Confirm this change",
    text: "This computer offers no system authentication, so DeckPilot can only ask you to confirm. Set a DeckPilot password in Settings if others use this computer.",
    action: "Confirm",
  },
}

/** Asks for the authentication of one sensitive operation. */
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
  const [password, setPassword] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const copy = COPY[verification]
  const needsPassword = verification === "password"
  const submit = () => {
    if (needsPassword && !password) return
    setBusy(true)
    setError(null)
    api
      .authenticate(needsPassword ? password : null)
      .then(onDone, (e: unknown) => {
        setError(
          failureMessage(
            "You were not verified",
            e,
            needsPassword ? "Check the password and try again" : "Try again",
          ),
        )
        setPassword("")
      })
      .finally(() => {
        setBusy(false)
      })
  }
  const icon =
    verification === "touchId" ? Fingerprint : verification === "password" ? KeyRound : ShieldAlert
  return (
    <ModalShell
      presentation="dialog"
      size="compact"
      icon={icon}
      title={copy.title}
      description={`Authenticate to ${purpose}.`}
      error={error}
      onClose={onCancel}
      onSubmit={submit}
      secondary={{ label: "Cancel", onClick: onCancel }}
      primary={{
        label: copy.action,
        loadingLabel: "Waiting…",
        loading: busy,
        disabled: needsPassword && !password,
      }}
    >
      <p className="text-sm text-muted-foreground">{copy.text}</p>
      {needsPassword && (
        <PasswordField
          id="authenticate-password"
          label="Password"
          value={password}
          onChange={setPassword}
          autoComplete="current-password"
          autoFocus
          invalid={Boolean(error)}
        />
      )}
    </ModalShell>
  )
}
