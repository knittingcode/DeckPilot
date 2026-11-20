import { Fingerprint, KeyRound, LockKeyhole, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { isCancelled, useSensitive } from "@/lib/sensitive"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { ModalShell } from "@/components/modal-shell"
import { ErrorNotice } from "@/components/page"
import { NewPasswordFields, PasswordField } from "@/components/password-field"
import { newPasswordReady } from "@/lib/passwords"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  InlineStatus,
  Property,
  PropertyList,
  SettingsGroup,
  SettingsRow,
  SettingsSection,
} from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { api } from "@/lib/api"
import { failureMessage, toastFailure } from "@/lib/failure"
import { useSecurity } from "@/lib/security-context"
import type { AutoLock, Reauthentication, SecurityPolicy, SecurityView } from "@/lib/types"
import { type TranslationKey, Trans, useTranslation } from "@/i18n"

const AUTO_LOCK: { value: AutoLock; label: TranslationKey }[] = [
  { value: "never", label: "security:card.autoLock.never" },
  // Sleep is noticed on every platform; the screen lock only where the
  // agent can detect it (see `screenLockDetection`).
  { value: "sleep", label: "security:card.autoLock.sleep" },
  { value: "screenLock", label: "security:card.autoLock.screenLock" },
  { value: "idle15Minutes", label: "security:card.autoLock.idle15Minutes" },
  { value: "idle1Hour", label: "security:card.autoLock.idle1Hour" },
]

const REAUTHENTICATION: { value: Reauthentication; label: TranslationKey }[] = [
  { value: "sensitiveOnly", label: "security:card.reauthentication.sensitiveOnly" },
  { value: "afterUpdate", label: "security:card.reauthentication.afterUpdate" },
  { value: "everyLaunch", label: "security:card.reauthentication.everyLaunch" },
]

/** Whether the user can prove who they are on this computer. */
function canVerify(view: SecurityView): boolean {
  return view.verification !== "confirmationOnly"
}

/** Reports a failed change, unless the user cancelled the verification. */
function reportError(what: string) {
  return (error: unknown) => {
    if (!isCancelled(error)) toastFailure(what, error)
  }
}

/**
 * The protection of the configuration: unlock method, automatic locking,
 * additional authentication, Touch ID, locking, legacy copies and backups.
 * Changes are sensitive operations and ask for authentication first.
 */
export function SecurityCard() {
  const { t } = useTranslation()
  const { view } = useSecurity()
  return (
    <>
      <p className="text-sm text-muted-foreground">{t("security:card.intro")}</p>
      {view.legacyCopies.length > 0 && <LegacyCopies />}
      <UnlockMethod />
      <Policy />
      <Recovery />
    </>
  )
}

function UnlockMethod() {
  const { t } = useTranslation()
  const { view, setView } = useSecurity()
  const sensitive = useSensitive()
  const system = view.protection === "systemManaged"
  const [choosePassword, setChoosePassword] = useState(false)
  return (
    <SettingsSection title={t("security:card.unlockMethod.title")}>
      <SettingsGroup>
        <SettingsRow
          label={
            <span className="flex items-center gap-2">
              {system ? (
                <KeyRound className="size-4 text-muted-foreground" />
              ) : (
                <LockKeyhole className="size-4 text-muted-foreground" />
              )}
              {system
                ? t("security:card.unlockMethod.systemKey", { keyStore: view.keyStore })
                : t("security:password.label")}
            </span>
          }
          description={
            system ? (
              <Trans
                i18nKey="security:card.unlockMethod.systemKeyDescription"
                values={{ keyStore: view.keyStore }}
                components={{ strong: <strong /> }}
              />
            ) : (
              t("security:card.unlockMethod.passwordDescription")
            )
          }
        >
          {system ? (
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setChoosePassword(true)
              }}
            >
              {t("security:card.unlockMethod.usePasswordEllipsis")}
            </Button>
          ) : (
            <ConfirmDialog
              trigger={
                <Button variant="outline" size="sm">
                  {t("security:card.unlockMethod.useSystemKeyEllipsis")}
                </Button>
              }
              title={t("security:card.unlockMethod.useSystemKeyTitle", { keyStore: view.keyStore })}
              description={t("security:card.unlockMethod.useSystemKeyDescription", {
                keyStore: view.keyStore,
              })}
              confirmLabel={t("security:card.unlockMethod.useSystemKey")}
              destructive={false}
              onConfirm={() => {
                sensitive(() => api.useSystemKey(), t("security:card.purpose.protection")).then(
                  (next) => {
                    setView(next)
                    toast.success(
                      t("security:card.unlockMethod.systemKeyDone", { keyStore: view.keyStore }),
                    )
                  },
                  reportError(t("security:card.unlockMethod.protectionFailed")),
                )
              }}
            />
          )}
        </SettingsRow>
        <SystemAuthentication />
        {!system && (
          <div className="py-3">
            <ChangePassword />
          </div>
        )}
      </SettingsGroup>
      {choosePassword && (
        <ChoosePasswordDialog
          onClose={() => {
            setChoosePassword(false)
          }}
        />
      )}
    </SettingsSection>
  )
}

/** System key → password. */
function ChoosePasswordDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation()
  const { setView } = useSecurity()
  const sensitive = useSensitive()
  const [password, setPassword] = useState("")
  const [confirmation, setConfirmation] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const ready = newPasswordReady(password, confirmation)
  const submit = () => {
    if (!ready) return
    setBusy(true)
    setError(null)
    sensitive(() => api.usePassword(password, confirmation), t("security:card.purpose.protection"))
      .then((next) => {
        setView(next)
        toast.success(t("security:card.choosePassword.done"))
        onClose()
      })
      .catch((e: unknown) => {
        if (!isCancelled(e)) setError(failureMessage(t("security:card.choosePassword.failed"), e))
      })
      .finally(() => {
        setBusy(false)
      })
  }
  return (
    <ModalShell
      presentation="dialog"
      size="compact"
      icon={LockKeyhole}
      title={t("security:setup.passwordTitle")}
      description={t("security:card.choosePassword.description")}
      error={error}
      onClose={onClose}
      onSubmit={submit}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose }}
      primary={{
        label: t("security:card.choosePassword.action"),
        loadingLabel: t("security:card.reEncrypting"),
        loading: busy,
        disabled: !ready,
      }}
    >
      <NewPasswordFields
        password={password}
        confirmation={confirmation}
        onPassword={setPassword}
        onConfirmation={setConfirmation}
        autoFocus
        idPrefix="choose-password"
      />
      <p className="text-xs text-muted-foreground">{t("security:newPassword.notRecoverable")}</p>
    </ModalShell>
  )
}

function Policy() {
  const { t } = useTranslation()
  const { view, setView } = useSecurity()
  const sensitive = useSensitive()
  const policy = view.policy ?? { autoLock: "never", reauthentication: "sensitiveOnly" }
  const verifiable = canVerify(view)
  const save = (next: SecurityPolicy) => {
    sensitive(() => api.setSecurityPolicy(next), t("security:card.purpose.policy")).then(
      (updated) => {
        setView(updated)
        toast.success(t("security:card.policy.saved"))
      },
      reportError(t("security:card.policy.notSaved")),
    )
  }
  return (
    <SettingsSection
      title={t("security:card.policy.title")}
      description={verifiable ? undefined : t("security:card.policy.unavailable")}
    >
      <SettingsGroup>
        <SettingsRow
          label={t("security:card.policy.autoLock")}
          htmlFor="auto-lock"
          description={
            <>
              {t("security:card.policy.autoLockDescription")}
              {!view.screenLockDetection && ` ${t("security:card.policy.noScreenLock")}`}
            </>
          }
        >
          <Select
            value={policy.autoLock}
            disabled={!verifiable}
            onValueChange={(value) => {
              save({ ...policy, autoLock: value as AutoLock })
            }}
          >
            <SelectTrigger id="auto-lock" size="sm" className="w-80 max-w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {AUTO_LOCK.map((option) => (
                <SelectItem
                  key={option.value}
                  value={option.value}
                  disabled={option.value === "screenLock" && !view.screenLockDetection}
                >
                  {t(option.label)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingsRow>
        <SettingsRow
          label={t("security:card.policy.reauthentication")}
          htmlFor="reauthentication"
          description={t("security:card.policy.reauthenticationDescription")}
        >
          <Select
            value={policy.reauthentication}
            disabled={!verifiable}
            onValueChange={(value) => {
              save({ ...policy, reauthentication: value as Reauthentication })
            }}
          >
            <SelectTrigger id="reauthentication" size="sm" className="w-80 max-w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {REAUTHENTICATION.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {t(option.label)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingsRow>
        {verifiable && <LockNow />}
      </SettingsGroup>
    </SettingsSection>
  )
}

function LockNow() {
  const { t } = useTranslation()
  const { setView } = useSecurity()
  return (
    <SettingsRow
      label={t("security:card.lockNow.label")}
      description={t("security:card.lockNow.description")}
    >
      <Button
        variant="outline"
        size="sm"
        onClick={() => {
          api.lock().then(setView, reportError(t("security:card.lockNow.failed")))
        }}
      >
        <LockKeyhole /> {t("security:card.lockNow.label")}
      </Button>
    </SettingsRow>
  )
}

/** Shown only where the platform offers it (Touch ID on macOS). */
function SystemAuthentication() {
  const { t } = useTranslation()
  const { view } = useSecurity()
  if (view.protection !== "systemManaged") return null
  if (view.verification === "touchId" || view.verification === "devicePassword") {
    const touchId = view.verification === "touchId"
    return (
      <SettingsRow
        label={
          <span className="flex items-center gap-2">
            <Fingerprint className="size-4 text-muted-foreground" />
            {touchId
              ? t("security:card.systemAuthentication.touchId")
              : t("security:card.systemAuthentication.title")}
          </span>
        }
        description={
          touchId
            ? t("security:card.systemAuthentication.touchIdDescription")
            : t("security:card.systemAuthentication.description")
        }
      >
        <InlineStatus tone="positive">
          {t("security:card.systemAuthentication.available")}
        </InlineStatus>
      </SettingsRow>
    )
  }
  return null
}

function Recovery() {
  const { t } = useTranslation()
  const { view } = useSecurity()
  return (
    <SettingsSection
      title={t("security:card.recovery.title")}
      description={
        <>
          {view.protection === "systemManaged"
            ? t("security:card.recovery.systemKey", { keyStore: view.keyStore })
            : t("security:card.recovery.password")}{" "}
          <Trans i18nKey="security:card.recovery.keepExport" components={{ strong: <strong /> }} />
        </>
      }
    >
      <PropertyList className="border-t pt-3">
        <Property label={t("security:card.recovery.backupDir")} mono>
          {view.backupDir}
        </Property>
      </PropertyList>
    </SettingsSection>
  )
}

function LegacyCopies() {
  const { t } = useTranslation()
  const { view, setView } = useSecurity()
  const sensitive = useSensitive()
  return (
    <Alert>
      <TriangleAlert />
      <AlertTitle>{t("security:card.legacy.title")}</AlertTitle>
      <AlertDescription className="grid gap-2">
        <p>{t("security:card.legacy.description")}</p>
        <ul className="font-mono text-xs break-all">
          {view.legacyCopies.map((path) => (
            <li key={path}>{path}</li>
          ))}
        </ul>
        <p className="text-xs">{t("security:card.legacy.recoverable")}</p>
        <ConfirmDialog
          trigger={
            <Button variant="outline" size="sm" className="justify-self-start">
              {t("security:card.legacy.delete")}
            </Button>
          }
          title={t("security:card.legacy.confirmTitle")}
          description={t("security:card.legacy.confirmDescription")}
          onConfirm={() => {
            sensitive(
              () => api.removeUnencryptedCopies(),
              t("security:card.purpose.deleteCopy"),
            ).then(
              (next) => {
                setView(next)
                toast.success(t("security:card.legacy.deleted"))
              },
              reportError(t("security:card.legacy.notDeleted")),
            )
          }}
        />
      </AlertDescription>
    </Alert>
  )
}

function ChangePassword() {
  const { t } = useTranslation()
  const { setView } = useSecurity()
  const [current, setCurrent] = useState("")
  const [password, setPassword] = useState("")
  const [confirmation, setConfirmation] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const ready = current.length > 0 && newPasswordReady(password, confirmation)
  const submit = (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!ready) return
    setBusy(true)
    setError(null)
    api
      .changePassword(current, password, confirmation)
      .then((next) => {
        setView(next)
        setCurrent("")
        setPassword("")
        setConfirmation("")
        toast.success(t("security:card.changePassword.done"))
      })
      .catch((e: unknown) => {
        setError(
          failureMessage(
            t("security:card.changePassword.failed"),
            e,
            t("security:card.changePassword.next"),
          ),
        )
      })
      .finally(() => {
        setBusy(false)
      })
  }
  return (
    <form className="grid max-w-md gap-3" onSubmit={submit}>
      <h4 className="text-sm leading-5 font-medium">{t("security:card.changePassword.title")}</h4>
      {error && <ErrorNotice error={error} />}
      <PasswordField
        id="current-password"
        label={t("security:card.changePassword.current")}
        value={current}
        onChange={setCurrent}
        autoComplete="current-password"
      />
      <NewPasswordFields
        password={password}
        confirmation={confirmation}
        onPassword={setPassword}
        onConfirmation={setConfirmation}
        idPrefix="changed-password"
      />
      <Button type="submit" className="justify-self-start" disabled={!ready || busy}>
        {busy ? t("security:card.reEncrypting") : t("security:card.changePassword.title")}
      </Button>
    </form>
  )
}
