import { Fingerprint, KeyRound, LockKeyhole, ShieldCheck, TriangleAlert } from "lucide-react"
import { useEffect, useState } from "react"

import { CredentialDialog } from "@/components/dialogs/credential-dialog"
import { ModalShell } from "@/components/modal-shell"
import { NewPasswordFields } from "@/components/password-field"
import { newPasswordReady } from "@/lib/passwords"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Trans, i18n, useTranslation } from "@/i18n"
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
              i18n.t("errors:security.statusUnreadable"),
              error,
              i18n.t("errors:next.restart"),
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
  const { t } = useTranslation()
  return (
    <ModalShell
      size="compact"
      blocking
      icon={KeyRound}
      title={t("security:keyStore.waitingTitle", { keyStore: view.keyStore })}
      description={t("security:keyStore.waitingDescription", { keyStore: view.keyStore })}
      loading
    />
  )
}

/**
 * The key file shows that DeckPilot was set up here, but the database is
 * gone. Nothing is created until the user decides.
 */
function MissingDatabase({ onChanged }: { onChanged: () => void }) {
  const { t } = useTranslation()
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
      title={t("security:missingDatabase.title")}
      description={t("security:missingDatabase.description")}
      error={error}
      onSubmit={() => {
        act(confirming ? () => api.startNewConfiguration() : () => api.systemUnlock())
      }}
      secondary={
        confirming
          ? {
              label: t("common:actions.back"),
              onClick: () => {
                setConfirming(false)
              },
            }
          : {
              label: t("security:missingDatabase.startNewEllipsis"),
              onClick: () => {
                setConfirming(true)
              },
            }
      }
      primary={
        confirming
          ? {
              label: t("security:missingDatabase.startNew"),
              loadingLabel: t("security:missingDatabase.creating"),
              loading: busy,
              variant: "destructive",
            }
          : {
              label: t("common:actions.tryAgain"),
              loadingLabel: t("common:status.trying"),
              loading: busy,
            }
      }
    >
      {confirming ? (
        <p className="text-sm text-muted-foreground">
          {t("security:missingDatabase.startNewExplanation")}
        </p>
      ) : (
        <p className="text-sm text-muted-foreground">
          <Trans
            i18nKey="security:missingDatabase.restore"
            values={{ file: "deckpilot.db" }}
            components={{ file: <span className="font-mono" /> }}
          />
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
  const { t } = useTranslation()
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
        loadingLabel: t("security:newPassword.encrypting"),
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
      <p className="text-xs text-muted-foreground">{t("security:newPassword.notRecoverable")}</p>
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
  const { t } = useTranslation()
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
      secondary={
        onPassword ? { label: t("security:systemKey.usePassword"), onClick: onPassword } : undefined
      }
      primary={{ label: retryLabel, loadingLabel: t("common:status.trying"), loading: busy }}
    >
      {view.problem && (
        <Alert>
          <AlertTitle>{view.keyStore}</AlertTitle>
          <AlertDescription className="break-words">{view.problem}</AlertDescription>
        </Alert>
      )}
      {!view.agentRunning && (
        <p className="text-sm text-muted-foreground">{t("security:systemKey.agentNotRunning")}</p>
      )}
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
      <button
        type="button"
        className="justify-self-start text-xs text-muted-foreground underline underline-offset-2"
        onClick={() => {
          api.openWindow("app.logs").catch(() => undefined)
        }}
      >
        {t("security:systemKey.showLog")}
      </button>
    </ModalShell>
  )
}

/**
 * First start when the key could not be created in the credential store
 * (normally the agent sets everything up without asking).
 */
export function SetupWindow() {
  const { t } = useTranslation()
  const state = useSecurityView()
  const [password, setPassword] = useState(false)
  if (state.status !== "ready")
    return (
      <Loading
        title={t("security:setup.welcome")}
        error={state.status === "error" ? state.error : undefined}
      />
    )
  if (state.data.waitingForKeyStore) return <WaitingForKeyStore view={state.data} />
  if (state.data.databaseMissing) return <MissingDatabase onChanged={state.reload} />
  if (password) {
    return (
      <NewPasswordForm
        icon={ShieldCheck}
        title={t("security:setup.passwordTitle")}
        description={t("security:setup.passwordDescription")}
        submitLabel={t("security:setup.create")}
        secondary={{
          label: t("common:actions.back"),
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
      title={t("security:setup.welcome")}
      description={t("security:setup.keyStoreFailed", { keyStore: state.data.keyStore })}
      retryLabel={t("common:actions.tryAgain")}
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
  const { t } = useTranslation()
  const state = useSecurityView()
  const [password, setPassword] = useState(false)
  const notice = (
    <Alert>
      <AlertTitle>{t("security:migrate.originalKept")}</AlertTitle>
      <AlertDescription>{t("security:migrate.originalReadable")}</AlertDescription>
    </Alert>
  )
  if (state.status !== "ready")
    return (
      <Loading
        title={t("security:migrate.title")}
        error={state.status === "error" ? state.error : undefined}
      />
    )
  if (state.data.waitingForKeyStore) return <WaitingForKeyStore view={state.data} />
  if (password) {
    return (
      <NewPasswordForm
        icon={KeyRound}
        title={t("security:migrate.title")}
        description={t("security:migrate.passwordDescription")}
        submitLabel={t("security:migrate.encrypt")}
        notice={notice}
        secondary={{
          label: t("common:actions.back"),
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
      title={t("security:migrate.title")}
      description={t("security:migrate.keyStoreFailed", { keyStore: state.data.keyStore })}
      retryLabel={t("common:actions.tryAgain")}
      onPassword={() => {
        setPassword(true)
      }}
    />
  )
}

/** The password check of the unlock and confirmation windows (blocking). */
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
  return (
    <CredentialDialog
      presentation="window"
      icon={icon}
      title={title}
      description={description}
      submitLabel={submitLabel}
      submit={submit}
      onDone={() => void finish()}
    />
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
  const { t } = useTranslation()
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
        label: touchId ? t("security:unlock.withTouchId") : t("security:unlock.action"),
        loadingLabel: t("common:status.waiting"),
        loading: busy,
      }}
    >
      {touchId && (
        <p className="text-sm text-muted-foreground">{t("security:unlock.macPassword")}</p>
      )}
    </ModalShell>
  )
}

export function UnlockWindow() {
  const { t } = useTranslation()
  const state = useSecurityView()
  if (state.status !== "ready")
    return (
      <Loading
        title={t("security:locked.title")}
        error={state.status === "error" ? state.error : undefined}
      />
    )
  const view = state.data
  if (view.waitingForKeyStore) return <WaitingForKeyStore view={view} />
  if (view.protection === "password") {
    return (
      <PasswordPrompt
        icon={LockKeyhole}
        title={t("security:locked.title")}
        description={t("security:unlock.passwordDescription")}
        submitLabel={t("security:unlock.action")}
        submit={(password) => api.unlock(password)}
      />
    )
  }
  if (view.lockReason === "keyUnavailable" || view.problem || !view.agentRunning) {
    return (
      <SystemKeyProblem
        view={view}
        title={t("security:cannotOpen.title")}
        description={t("security:cannotOpen.keyUnavailable", { keyStore: view.keyStore })}
        retryLabel={t("common:actions.tryAgain")}
        hint={t("security:cannotOpen.keyDeleted")}
      />
    )
  }
  const description =
    view.lockReason === "idle"
      ? t("security:unlock.reason.idle")
      : view.lockReason === "sleep"
        ? t("security:unlock.reason.sleep")
        : view.lockReason === "screenLocked"
          ? t("security:unlock.reason.screenLocked")
          : t("security:unlock.reason.other")
  return (
    <SystemPrompt
      view={view}
      title={t("security:locked.title")}
      description={description}
      submit={() => api.systemUnlock()}
    />
  )
}

/** The authentication the security policy asks for after an update or at launch. */
export function ConfirmWindow() {
  const { t } = useTranslation()
  const state = useSecurityView()
  if (state.status !== "ready")
    return (
      <Loading
        title={t("security:confirm.title")}
        error={state.status === "error" ? state.error : undefined}
      />
    )
  const view = state.data
  if (view.protection === "password") {
    return (
      <PasswordPrompt
        icon={ShieldCheck}
        title={t("security:confirm.title")}
        description={t("security:confirm.passwordDescription")}
        submitLabel={t("common:actions.continue")}
        submit={(password) => api.confirmIdentity(password)}
      />
    )
  }
  return (
    <SystemPrompt
      view={view}
      title={t("security:confirm.title")}
      description={t("security:confirm.systemDescription")}
      submit={() => api.confirmIdentity(null)}
    />
  )
}
