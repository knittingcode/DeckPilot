import { Boxes, KeyRound, LockKeyhole, ShieldCheck, TriangleAlert } from "lucide-react"
import { type ReactNode, useCallback, useEffect, useState } from "react"

import { ErrorNotice, Loading } from "@/components/page"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api, errorMessage } from "@/lib/api"
import { SecurityContext } from "@/lib/security-context"
import type { SecurityView } from "@/lib/types"

const MIN_PASSWORD = 8

/**
 * Shows the password screens until the configuration is unlocked: first-run
 * setup, unlock, encrypting a database from an earlier version, and the
 * one-time confirmation after an update.
 */
export function SecurityGate({ children }: { children: ReactNode }) {
  const [view, setView] = useState<SecurityView | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(() => {
    api.securityStatus().then(setView, (e: unknown) => {
      setError(errorMessage(e))
    })
  }, [])

  useEffect(refresh, [refresh])

  if (error) {
    return (
      <Screen icon={TriangleAlert} title="DeckPilot cannot open its configuration">
        <ErrorNotice error={error} />
      </Screen>
    )
  }
  if (!view) {
    return (
      <div className="p-6">
        <Loading />
      </div>
    )
  }
  if (view.unlocked) {
    return <SecurityContext value={{ view, setView, refresh }}>{children}</SecurityContext>
  }
  if (view.database === "missing") return <SetupScreen onDone={setView} />
  if (view.database === "plaintext") return <MigrateScreen onDone={setView} />
  if (view.confirmationRequired) return <ConfirmScreen view={view} onDone={setView} />
  return <UnlockScreen onDone={setView} />
}

function Screen({
  icon: Icon,
  title,
  description,
  children,
}: {
  icon: typeof Boxes
  title: string
  description?: ReactNode
  children: ReactNode
}) {
  return (
    <main className="flex min-h-svh items-center justify-center bg-muted/40 p-4">
      <Card className="w-full max-w-md">
        <CardHeader className="gap-3">
          <div className="flex size-10 items-center justify-center rounded-lg bg-primary text-primary-foreground">
            <Icon className="size-5" aria-hidden />
          </div>
          <CardTitle className="text-xl">{title}</CardTitle>
          {description && <CardDescription>{description}</CardDescription>}
        </CardHeader>
        <CardContent>{children}</CardContent>
      </Card>
    </main>
  )
}

function PasswordField({
  id,
  label,
  value,
  onChange,
  autoComplete,
  autoFocus = false,
}: {
  id: string
  label: string
  value: string
  onChange: (value: string) => void
  autoComplete: "current-password" | "new-password"
  autoFocus?: boolean
}) {
  return (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="password"
        autoComplete={autoComplete}
        autoFocus={autoFocus}
        value={value}
        onChange={(e) => {
          onChange(e.target.value)
        }}
      />
    </div>
  )
}

/** Password and confirmation, checked locally before the backend checks again. */
function NewPasswordForm({
  submitLabel,
  onSubmit,
}: {
  submitLabel: string
  onSubmit: (password: string, confirmation: string) => Promise<void>
}) {
  const [password, setPassword] = useState("")
  const [confirmation, setConfirmation] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const problem =
    password.length > 0 && password.length < MIN_PASSWORD
      ? `Use at least ${MIN_PASSWORD} characters.`
      : confirmation.length > 0 && confirmation !== password
        ? "The passwords do not match."
        : null
  const submit = (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (problem || password.length < MIN_PASSWORD || password !== confirmation) return
    setBusy(true)
    setError(null)
    onSubmit(password, confirmation)
      .catch((e: unknown) => {
        setError(errorMessage(e))
      })
      .finally(() => {
        setBusy(false)
      })
  }
  return (
    <form className="grid gap-4" onSubmit={submit}>
      {error && <ErrorNotice error={error} />}
      <PasswordField
        id="new-password"
        label="Password"
        value={password}
        onChange={setPassword}
        autoComplete="new-password"
        autoFocus
      />
      <PasswordField
        id="new-password-confirmation"
        label="Repeat password"
        value={confirmation}
        onChange={setConfirmation}
        autoComplete="new-password"
      />
      {problem && <p className="text-sm text-destructive">{problem}</p>}
      <Button
        type="submit"
        disabled={busy || password.length < MIN_PASSWORD || password !== confirmation}
      >
        {busy ? "Encrypting…" : submitLabel}
      </Button>
      <p className="text-xs text-muted-foreground">
        The password cannot be recovered. Without it, the configuration cannot be opened; keep an
        encrypted export as a backup.
      </p>
    </form>
  )
}

function SetupScreen({ onDone }: { onDone: (view: SecurityView) => void }) {
  return (
    <Screen
      icon={ShieldCheck}
      title="Welcome to DeckPilot"
      description="Choose a password to encrypt your configuration on this computer. You enter it after restarting the computer and once after each update."
    >
      <NewPasswordForm
        submitLabel="Create encrypted configuration"
        onSubmit={async (password, confirmation) => {
          onDone(await api.setupPassword(password, confirmation))
        }}
      />
    </Screen>
  )
}

function MigrateScreen({ onDone }: { onDone: (view: SecurityView) => void }) {
  return (
    <Screen
      icon={KeyRound}
      title="Encrypt your configuration"
      description="This configuration was saved by an earlier DeckPilot without encryption. Choose a password to encrypt it; your profiles, actions, workflows, layouts and settings are kept."
    >
      <Alert className="mb-4">
        <TriangleAlert />
        <AlertTitle>The unencrypted original is kept</AlertTitle>
        <AlertDescription>
          It stays next to the new database until you delete it in Settings, so nothing is lost if
          something goes wrong. Until then, anyone with access to your files can read it.
        </AlertDescription>
      </Alert>
      <NewPasswordForm
        submitLabel="Encrypt configuration"
        onSubmit={async (password, confirmation) => {
          onDone(await api.encryptLegacyDatabase(password, confirmation))
        }}
      />
    </Screen>
  )
}

function SinglePasswordForm({
  submitLabel,
  onSubmit,
}: {
  submitLabel: string
  onSubmit: (password: string) => Promise<void>
}) {
  const [password, setPassword] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const submit = (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!password) return
    setBusy(true)
    setError(null)
    onSubmit(password)
      .catch((e: unknown) => {
        setError(errorMessage(e))
        setPassword("")
      })
      .finally(() => {
        setBusy(false)
      })
  }
  return (
    <form className="grid gap-4" onSubmit={submit}>
      <PasswordField
        id="password"
        label="Password"
        value={password}
        onChange={setPassword}
        autoComplete="current-password"
        autoFocus
      />
      {error && (
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      )}
      <Button type="submit" disabled={busy || !password}>
        {busy ? "Checking…" : submitLabel}
      </Button>
    </form>
  )
}

function UnlockScreen({ onDone }: { onDone: (view: SecurityView) => void }) {
  return (
    <Screen
      icon={LockKeyhole}
      title="DeckPilot is locked"
      description="Enter your password to open the configuration. Your Stream Deck resumes as soon as it is unlocked."
    >
      <SinglePasswordForm
        submitLabel="Unlock"
        onSubmit={async (password) => {
          onDone(await api.unlock(password))
        }}
      />
    </Screen>
  )
}

function ConfirmScreen({
  view,
  onDone,
}: {
  view: SecurityView
  onDone: (view: SecurityView) => void
}) {
  return (
    <Screen
      icon={ShieldCheck}
      title={`DeckPilot ${view.appVersion} is installed`}
      description="Confirm your password once to continue. It is not asked again until the next update."
    >
      <SinglePasswordForm
        submitLabel="Continue"
        onSubmit={async (password) => {
          onDone(await api.confirmPassword(password))
        }}
      />
    </Screen>
  )
}
