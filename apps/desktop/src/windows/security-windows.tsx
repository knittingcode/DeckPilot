import { Fingerprint, KeyRound, LockKeyhole, ShieldCheck, TriangleAlert } from "lucide-react"
import { useEffect, useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { NewPasswordFields, PasswordField } from "@/components/password-field"
import { newPasswordReady } from "@/lib/passwords"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { api, errorMessage } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import type { SecurityView } from "@/lib/types"

/**
 * The prompts that open the configuration. Each runs in its own compact,
 * blocking native window (`security.*`); when it succeeds, the prompt
 * closes and the main window opens. They adapt to the protection mode: a
 * password field with password protection, the system check (Touch ID) or
 * a retry with a system-managed key. Passwords stay in component state
 * only and are sent to the backend command, never logged.
 */

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

type ViewState =
  | { status: "loading" }
  | { status: "error"; error: string }
  | { status: "ready"; data: SecurityView }

/**
 * Loads the security state. While the agent waits for the credential store
 * (which may be showing a permission prompt) it is polled; once the
 * configuration is open, the prompt finishes by itself.
 */
function useSecurityView(): ViewState & { reload: () => void } {
  const [state, setState] = useState<ViewState>({ status: "loading" })
  const [generation, setGeneration] = useState(0)
  useEffect(() => {
    let cancelled = false
    api.securityStatus().then(
      (data) => {
        if (!cancelled) setState({ status: "ready", data })
      },
      (error: unknown) => {
        if (!cancelled) {
          setState({
            status: "error",
            error: failureMessage(
              "The protection status could not be read",
              error,
              "Restart DeckPilot",
            ),
          })
        }
      },
    )
    return () => {
      cancelled = true
    }
  }, [generation])
  const view = state.status === "ready" ? state.data : null
  useEffect(() => {
    if (!view) return
    if (view.unlocked) {
      void finish()
      return
    }
    if (!view.waitingForKeyStore) return
    const timer = setTimeout(() => {
      setGeneration((g) => g + 1)
    }, 1000)
    return () => {
      clearTimeout(timer)
    }
  }, [view])
  return {
    ...state,
    reload: () => {
      setGeneration((g) => g + 1)
    },
  }
}

/** The agent is reading the key; the credential store may ask first. */
function WaitingForKeyStore({ view }: { view: SecurityView }) {
  return (
    <ModalShell
      size="compact"
      blocking
      icon={KeyRound}
      title={`Waiting for the ${view.keyStore}`}
      description={`DeckPilot is reading its key. If the ${view.keyStore} asks whether DeckPilot may use it, allow it.`}
      loading
    />
  )
}

/**
 * The key file shows that DeckPilot was set up here, but the database is
 * gone. Nothing is created until the user decides.
 */
function MissingDatabase({ onChanged }: { onChanged: () => void }) {
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const act = (task: () => Promise<SecurityView>) => {
    setBusy(true)
    setError(null)
    task()
      .then((view) => {
        if (view.unlocked) void finish()
        else onChanged()
      })
      .catch((e: unknown) => {
        setError(errorMessage(e))
      })
      .finally(() => {
        setBusy(false)
      })
  }
  return (
    <ModalShell
      size="compact"
      blocking
      icon={TriangleAlert}
      title="Your configuration is missing"
      description="DeckPilot was set up on this computer, but its configuration database is gone — perhaps it was moved, deleted, or is on a drive that is not connected. Nothing was created or overwritten."
      error={error}
      onSubmit={() => {
        act(confirming ? () => api.startNewConfiguration() : () => api.systemUnlock())
      }}
      secondary={
        confirming
          ? {
              label: "Back",
              onClick: () => {
                setConfirming(false)
              },
            }
          : {
              label: "Start a new configuration…",
              onClick: () => {
                setConfirming(true)
              },
            }
      }
      primary={
        confirming
          ? {
              label: "Start a new configuration",
              loadingLabel: "Creating…",
              loading: busy,
              variant: "destructive",
            }
          : { label: "Try again", loadingLabel: "Trying…", loading: busy }
      }
    >
      {confirming ? (
        <p className="text-sm text-muted-foreground">
          DeckPilot starts with an empty configuration. The old key file is kept next to it, and its
          key stays in the credential store, so the old configuration can still be opened if you
          find its database. Import an encrypted export to restore your profiles.
        </p>
      ) : (
        <p className="text-sm text-muted-foreground">
          Restore <span className="font-mono">deckpilot.db</span> to the data folder (or connect its
          drive) and try again.
        </p>
      )}
    </ModalShell>
  )
}

function Loading({ title, error }: { title: string; error?: string }) {
  return (
    <ModalShell
      size="compact"
      blocking
      icon={LockKeyhole}
      title={title}
      loading={!error}
      error={error ?? null}
    />
  )
}

function NewPasswordForm({
  icon,
  title,
  description,
  submitLabel,
  notice,
  secondary,
  submit,
}: {
  icon: typeof ShieldCheck
  title: string
  description: string
  submitLabel: string
  notice?: React.ReactNode
  secondary?: { label: string; onClick: () => void }
  submit: (password: string, confirmation: string) => Promise<unknown>
}) {
  const [password, setPassword] = useState("")
  const [confirmation, setConfirmation] = useState("")
  const { busy, error, run } = useSubmit()
  const ready = newPasswordReady(password, confirmation)
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
      secondary={secondary}
      primary={{
        label: submitLabel,
        loadingLabel: "Encrypting…",
        loading: busy,
        disabled: !ready,
      }}
    >
      {notice}
      <NewPasswordFields
        password={password}
        confirmation={confirmation}
        onPassword={setPassword}
        onConfirmation={setConfirmation}
        autoFocus
      />
      <p className="text-xs text-muted-foreground">
        The password cannot be recovered. Keep an encrypted export as a backup.
      </p>
    </ModalShell>
  )
}

/**
 * Shown when the system key could not be stored or read, with the reason,
 * a retry and (where it makes sense) the choice of a password instead.
 */
function SystemKeyProblem({
  view,
  title,
  description,
  retryLabel,
  hint,
  onPassword,
}: {
  view: SecurityView
  title: string
  description: string
  retryLabel: string
  hint?: string
  onPassword?: () => void
}) {
  const { busy, error, run } = useSubmit()
  return (
    <ModalShell
      size="compact"
      blocking
      icon={TriangleAlert}
      title={title}
      description={description}
      error={error}
      onSubmit={() => {
        run(() => api.systemUnlock())
      }}
      secondary={onPassword ? { label: "Use a password instead", onClick: onPassword } : undefined}
      primary={{ label: retryLabel, loadingLabel: "Trying…", loading: busy }}
    >
      {view.problem && (
        <Alert>
          <AlertTitle>{view.keyStore}</AlertTitle>
          <AlertDescription className="break-words">{view.problem}</AlertDescription>
        </Alert>
      )}
      {!view.agentRunning && (
        <p className="text-sm text-muted-foreground">
          The DeckPilot background agent is not running. It keeps the key and starts with your
          login; start it from the Dashboard or try again.
        </p>
      )}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
      <button
        type="button"
        className="justify-self-start text-xs text-muted-foreground underline underline-offset-2"
        onClick={() => {
          api.openWindow("app.logs").catch(() => undefined)
        }}
      >
        Show the agent log
      </button>
    </ModalShell>
  )
}

/**
 * First start when the key could not be created in the credential store
 * (normally the agent sets everything up without asking).
 */
export function SetupWindow() {
  const state = useSecurityView()
  const [password, setPassword] = useState(false)
  if (state.status !== "ready")
    return (
      <Loading
        title="Welcome to DeckPilot"
        error={state.status === "error" ? state.error : undefined}
      />
    )
  if (state.data.waitingForKeyStore) return <WaitingForKeyStore view={state.data} />
  if (state.data.databaseMissing) return <MissingDatabase onChanged={state.reload} />
  if (password) {
    return (
      <NewPasswordForm
        icon={ShieldCheck}
        title="Protect DeckPilot with a password"
        description="The configuration is encrypted with a key derived from this password. You enter it after each restart."
        submitLabel="Create encrypted configuration"
        secondary={{
          label: "Back",
          onClick: () => {
            setPassword(false)
          },
        }}
        submit={(value, confirmation) => api.setupPassword(value, confirmation)}
      />
    )
  }
  return (
    <SystemKeyProblem
      view={state.data}
      title="Welcome to DeckPilot"
      description={`DeckPilot encrypts your configuration with a key kept in the ${state.data.keyStore}, but could not store it there.`}
      retryLabel="Try again"
      onPassword={() => {
        setPassword(true)
      }}
    />
  )
}

/**
 * A configuration from DeckPilot 0.3, which was not encrypted, that the
 * agent could not encrypt with a system key by itself.
 */
export function MigrateWindow() {
  const state = useSecurityView()
  const [password, setPassword] = useState(false)
  const notice = (
    <Alert>
      <AlertTitle>The unencrypted original is kept</AlertTitle>
      <AlertDescription>
        Until you delete it in Settings, anyone with access to your files can read it.
      </AlertDescription>
    </Alert>
  )
  if (state.status !== "ready")
    return (
      <Loading
        title="Encrypt your configuration"
        error={state.status === "error" ? state.error : undefined}
      />
    )
  if (state.data.waitingForKeyStore) return <WaitingForKeyStore view={state.data} />
  if (password) {
    return (
      <NewPasswordForm
        icon={KeyRound}
        title="Encrypt your configuration"
        description="It was saved by an earlier DeckPilot without encryption. Choose a password; everything is kept."
        submitLabel="Encrypt configuration"
        notice={notice}
        secondary={{
          label: "Back",
          onClick: () => {
            setPassword(false)
          },
        }}
        submit={(value, confirmation) => api.encryptLegacyDatabase(value, confirmation)}
      />
    )
  }
  return (
    <SystemKeyProblem
      view={state.data}
      title="Encrypt your configuration"
      description={`It was saved by an earlier DeckPilot without encryption. DeckPilot encrypts it with a key kept in the ${state.data.keyStore}, but could not store the key.`}
      retryLabel="Try again"
      onPassword={() => {
        setPassword(true)
      }}
    />
  )
}

function PasswordPrompt({
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

/** One button that runs the system check (or simply continues). */
function SystemPrompt({
  view,
  title,
  description,
  submit,
}: {
  view: SecurityView
  title: string
  description: string
  submit: () => Promise<unknown>
}) {
  const { busy, error, run } = useSubmit()
  const touchId = view.verification === "touchId"
  return (
    <ModalShell
      size="compact"
      blocking
      icon={touchId ? Fingerprint : LockKeyhole}
      title={title}
      description={description}
      error={error}
      onSubmit={() => {
        run(submit)
      }}
      primary={{
        label: touchId ? "Unlock with Touch ID" : "Unlock",
        loadingLabel: "Waiting…",
        loading: busy,
      }}
    >
      {touchId && (
        <p className="text-sm text-muted-foreground">
          You can also use your Mac&apos;s login password in the system prompt.
        </p>
      )}
    </ModalShell>
  )
}

export function UnlockWindow() {
  const state = useSecurityView()
  if (state.status !== "ready")
    return (
      <Loading
        title="DeckPilot is locked"
        error={state.status === "error" ? state.error : undefined}
      />
    )
  const view = state.data
  if (view.waitingForKeyStore) return <WaitingForKeyStore view={view} />
  if (view.protection === "password") {
    return (
      <PasswordPrompt
        icon={LockKeyhole}
        title="DeckPilot is locked"
        description="Enter your password. Your Stream Deck resumes as soon as it is unlocked."
        submitLabel="Unlock"
        submit={(password) => api.unlock(password)}
      />
    )
  }
  if (view.lockReason === "keyUnavailable" || view.problem || !view.agentRunning) {
    return (
      <SystemKeyProblem
        view={view}
        title="DeckPilot cannot open its configuration"
        description={`Its key is kept in the ${view.keyStore}, which did not provide it. Unlock the ${view.keyStore} or sign in again, then try again. Nothing was changed.`}
        retryLabel="Try again"
        hint="If the key was deleted (for example when the keychain was reset), this configuration cannot be opened anymore. Restore your encrypted export on a new configuration; see the security documentation."
      />
    )
  }
  const reason =
    view.lockReason === "idle"
      ? "It was locked after a period without activity."
      : view.lockReason === "sleep"
        ? "It was locked when the computer went to sleep."
        : view.lockReason === "screenLocked"
          ? "It was locked when the screen was locked."
          : "It was locked."
  return (
    <SystemPrompt
      view={view}
      title="DeckPilot is locked"
      description={`${reason} Your Stream Deck resumes as soon as it is unlocked.`}
      submit={() => api.systemUnlock()}
    />
  )
}

/** The authentication the security policy asks for after an update or at launch. */
export function ConfirmWindow() {
  const state = useSecurityView()
  if (state.status !== "ready")
    return (
      <Loading
        title="Confirm it's you"
        error={state.status === "error" ? state.error : undefined}
      />
    )
  const view = state.data
  if (view.protection === "password") {
    return (
      <PasswordPrompt
        icon={ShieldCheck}
        title="Confirm it's you"
        description="Your security settings ask for your password when DeckPilot starts or after an update."
        submitLabel="Continue"
        submit={(password) => api.confirmIdentity(password)}
      />
    )
  }
  return (
    <SystemPrompt
      view={view}
      title="Confirm it's you"
      description="Your security settings ask for authentication when DeckPilot starts or after an update."
      submit={() => api.confirmIdentity(null)}
    />
  )
}
