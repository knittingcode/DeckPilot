import { KeyRound, LockKeyhole, ShieldCheck } from "lucide-react"
import { useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api, errorMessage } from "@/lib/api"

/**
 * The password prompts. Each runs in its own compact, blocking native
 * window (`security.*`); when it succeeds, the prompt closes and the main
 * window opens. Passwords stay in component state only and are sent to the
 * backend command, never logged.
 */

const MIN_PASSWORD = 8

function PasswordField({
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

async function finish() {
  await api.finishSecurityPrompt()
}

/** Shared state of the forms: busy flag and error. */
function useSubmit() {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = (task: () => Promise<unknown>, onError?: () => void) => {
    setBusy(true)
    setError(null)
    task()
      .then(finish)
      .catch((e: unknown) => {
        setError(errorMessage(e))
        onError?.()
      })
      .finally(() => {
        setBusy(false)
      })
  }
  return { busy, error, run }
}

function NewPasswordWindow({
  icon,
  title,
  description,
  submitLabel,
  notice,
  submit,
}: {
  icon: typeof ShieldCheck
  title: string
  description: string
  submitLabel: string
  notice?: React.ReactNode
  submit: (password: string, confirmation: string) => Promise<unknown>
}) {
  const [password, setPassword] = useState("")
  const [confirmation, setConfirmation] = useState("")
  const { busy, error, run } = useSubmit()
  const tooShort = password.length > 0 && password.length < MIN_PASSWORD
  const mismatch = confirmation.length > 0 && confirmation !== password
  const ready = password.length >= MIN_PASSWORD && password === confirmation
  return (
    <ModalShell
      size="compact"
      blocking
      icon={icon}
      title={title}
      description={description}
      error={error}
      onSubmit={() => {
        if (ready) run(() => submit(password, confirmation))
      }}
      primary={{
        label: submitLabel,
        loadingLabel: "Encrypting…",
        loading: busy,
        disabled: !ready,
      }}
    >
      {notice}
      <PasswordField
        id="new-password"
        label="Password"
        value={password}
        onChange={setPassword}
        autoComplete="new-password"
        autoFocus
        invalid={tooShort}
      />
      <PasswordField
        id="new-password-confirmation"
        label="Repeat password"
        value={confirmation}
        onChange={setConfirmation}
        autoComplete="new-password"
        invalid={mismatch}
      />
      {tooShort && (
        <p className="text-sm text-destructive">Use at least {MIN_PASSWORD} characters.</p>
      )}
      {mismatch && <p className="text-sm text-destructive">The passwords do not match.</p>}
      <p className="text-xs text-muted-foreground">
        The password cannot be recovered. Keep an encrypted export as a backup.
      </p>
    </ModalShell>
  )
}

export function SetupWindow() {
  return (
    <NewPasswordWindow
      icon={ShieldCheck}
      title="Welcome to DeckPilot"
      description="Choose a password to encrypt your configuration on this computer."
      submitLabel="Create encrypted configuration"
      submit={(password, confirmation) => api.setupPassword(password, confirmation)}
    />
  )
}

export function MigrateWindow() {
  return (
    <NewPasswordWindow
      icon={KeyRound}
      title="Encrypt your configuration"
      description="It was saved by an earlier DeckPilot without encryption. Choose a password; everything is kept."
      submitLabel="Encrypt configuration"
      notice={
        <Alert>
          <AlertTitle>The unencrypted original is kept</AlertTitle>
          <AlertDescription>
            Until you delete it in Settings, anyone with access to your files can read it.
          </AlertDescription>
        </Alert>
      }
      submit={(password, confirmation) => api.encryptLegacyDatabase(password, confirmation)}
    />
  )
}

function PasswordWindow({
  icon,
  title,
  description,
  submitLabel,
  submit,
}: {
  icon: typeof LockKeyhole
  title: string
  description: string
  submitLabel: string
  submit: (password: string) => Promise<unknown>
}) {
  const [password, setPassword] = useState("")
  const { busy, error, run } = useSubmit()
  return (
    <ModalShell
      size="compact"
      blocking
      icon={icon}
      title={title}
      description={description}
      error={error}
      onSubmit={() => {
        if (password)
          run(
            () => submit(password),
            () => {
              setPassword("")
            },
          )
      }}
      primary={{
        label: submitLabel,
        loadingLabel: "Checking…",
        loading: busy,
        disabled: !password,
      }}
    >
      <PasswordField
        id="password"
        label="Password"
        value={password}
        onChange={setPassword}
        autoComplete="current-password"
        autoFocus
        invalid={Boolean(error)}
      />
    </ModalShell>
  )
}

export function UnlockWindow() {
  return (
    <PasswordWindow
      icon={LockKeyhole}
      title="DeckPilot is locked"
      description="Enter your password. Your Stream Deck resumes as soon as it is unlocked."
      submitLabel="Unlock"
      submit={(password) => api.unlock(password)}
    />
  )
}

export function ConfirmWindow() {
  return (
    <PasswordWindow
      icon={ShieldCheck}
      title="DeckPilot was updated"
      description="Confirm your password once to continue. It is not asked again until the next update."
      submitLabel="Continue"
      submit={(password) => api.confirmPassword(password)}
    />
  )
}
