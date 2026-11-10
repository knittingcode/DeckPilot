import { Fingerprint, KeyRound, LockKeyhole, ShieldCheck, TriangleAlert } from "lucide-react"
import { type ReactNode, useState } from "react"
import { toast } from "sonner"

import { isCancelled, useSensitive } from "@/lib/sensitive"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { ModalShell } from "@/components/modal-shell"
import { ErrorNotice } from "@/components/page"
import { NewPasswordFields, PasswordField } from "@/components/password-field"
import { newPasswordReady } from "@/lib/passwords"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Separator } from "@/components/ui/separator"
import { api, errorMessage } from "@/lib/api"
import { useSecurity } from "@/lib/security-context"
import type { AutoLock, Reauthentication, SecurityPolicy, SecurityView } from "@/lib/types"

const AUTO_LOCK: { value: AutoLock; label: string }[] = [
  { value: "never", label: "Never" },
  // DeckPilot notices sleep on every platform. Detecting a locked screen is
  // not implemented, so it is not offered.
  { value: "sleep", label: "When the computer sleeps" },
  { value: "idle15Minutes", label: "After 15 minutes of inactivity" },
  { value: "idle1Hour", label: "After 1 hour of inactivity" },
]

const REAUTHENTICATION: { value: Reauthentication; label: string }[] = [
  { value: "sensitiveOnly", label: "Only for sensitive operations" },
  { value: "afterUpdate", label: "After updates and for sensitive operations" },
  { value: "everyLaunch", label: "Every time DeckPilot starts" },
]

/** Whether the user can prove who they are on this computer. */
function canVerify(view: SecurityView): boolean {
  return view.verification !== "confirmationOnly"
}

function reportError(error: unknown) {
  if (!isCancelled(error)) toast.error(errorMessage(error))
}

/**
 * The protection of the configuration: unlock method, automatic locking,
 * additional authentication, Touch ID, locking, legacy copies and backups.
 * Changes are sensitive operations and ask for authentication first.
 */
export function SecurityCard() {
  const { view } = useSecurity()
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="size-4" /> Security
        </CardTitle>
        <CardDescription>
          Your configuration is always encrypted on this computer (SQLCipher, AES-256). These
          settings decide how its key is protected and when DeckPilot asks who you are.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-6">
        {view.legacyCopies.length > 0 && <LegacyCopies />}
        <UnlockMethod />
        <Separator />
        <Policy />
        <SystemAuthentication />
        <Separator />
        <Recovery />
      </CardContent>
    </Card>
  )
}

function Section({
  title,
  description,
  children,
}: {
  title: string
  description?: ReactNode
  children?: ReactNode
}) {
  return (
    <section className="grid gap-3">
      <div className="grid gap-1">
        <h3 className="text-sm font-medium">{title}</h3>
        {description && <p className="text-sm text-muted-foreground">{description}</p>}
      </div>
      {children}
    </section>
  )
}

function UnlockMethod() {
  const { view, setView } = useSecurity()
  const sensitive = useSensitive()
  const system = view.protection === "systemManaged"
  const [choosePassword, setChoosePassword] = useState(false)
  return (
    <Section
      title="Unlock method"
      description={
        system ? (
          <>
            A random key kept in the <strong>{view.keyStore}</strong> and unlocked by your login.
            DeckPilot opens without a password, also after restarts and updates.
          </>
        ) : (
          <>
            A key derived from your DeckPilot password (Argon2id). The password is not stored; you
            enter it after each restart of the computer.
          </>
        )
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="secondary">
          {system ? <KeyRound /> : <LockKeyhole />}
          {system ? `System key · ${view.keyStore}` : "Password"}
        </Badge>
        {system ? (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setChoosePassword(true)
            }}
          >
            Use a password instead…
          </Button>
        ) : (
          <ConfirmDialog
            trigger={
              <Button variant="outline" size="sm">
                Use the system key instead…
              </Button>
            }
            title={`Keep the key in the ${view.keyStore}?`}
            description={`DeckPilot creates a new random key, stores it in the ${view.keyStore} and re-encrypts the configuration with it. Your password is no longer needed. If storing the key fails, nothing changes.`}
            confirmLabel="Use the system key"
            destructive={false}
            onConfirm={() => {
              sensitive(() => api.useSystemKey(), "change how the configuration is protected").then(
                (next) => {
                  setView(next)
                  toast.success(`The key is now kept in the ${view.keyStore}`)
                },
                reportError,
              )
            }}
          />
        )}
      </div>
      {!system && <ChangePassword />}
      {choosePassword && (
        <ChoosePasswordDialog
          onClose={() => {
            setChoosePassword(false)
          }}
        />
      )}
    </Section>
  )
}

/** System key → password. */
function ChoosePasswordDialog({ onClose }: { onClose: () => void }) {
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
    sensitive(
      () => api.usePassword(password, confirmation),
      "change how the configuration is protected",
    )
      .then((next) => {
        setView(next)
        toast.success("The configuration is now protected by your password")
        onClose()
      })
      .catch((e: unknown) => {
        if (!isCancelled(e)) setError(errorMessage(e))
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
      title="Protect DeckPilot with a password"
      description="The configuration is re-encrypted with a key derived from this password. You will enter it after each restart of the computer."
      error={error}
      onClose={onClose}
      onSubmit={submit}
      secondary={{ label: "Cancel", onClick: onClose }}
      primary={{
        label: "Use this password",
        loadingLabel: "Re-encrypting…",
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
      <p className="text-xs text-muted-foreground">
        The password cannot be recovered. Keep an encrypted export as a backup.
      </p>
    </ModalShell>
  )
}

function Policy() {
  const { view, setView } = useSecurity()
  const sensitive = useSensitive()
  const policy = view.policy ?? { autoLock: "never", reauthentication: "sensitiveOnly" }
  const verifiable = canVerify(view)
  const save = (next: SecurityPolicy) => {
    sensitive(() => api.setSecurityPolicy(next), "change your security settings").then(
      (updated) => {
        setView(updated)
        toast.success("Security settings saved")
      },
      reportError,
    )
  }
  return (
    <Section
      title="Locking and authentication"
      description={
        verifiable
          ? undefined
          : "These options need a DeckPilot password or system authentication, which this computer does not offer with the system key."
      }
    >
      <div className="grid max-w-md gap-4">
        <div className="grid gap-2">
          <Label htmlFor="auto-lock">Lock automatically</Label>
          <Select
            value={policy.autoLock}
            disabled={!verifiable}
            onValueChange={(value) => {
              save({ ...policy, autoLock: value as AutoLock })
            }}
          >
            <SelectTrigger id="auto-lock" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {AUTO_LOCK.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            While locked, your Stream Deck shows &quot;Locked&quot; and runs no actions.
          </p>
        </div>
        <div className="grid gap-2">
          <Label htmlFor="reauthentication">Ask for authentication</Label>
          <Select
            value={policy.reauthentication}
            disabled={!verifiable}
            onValueChange={(value) => {
              save({ ...policy, reauthentication: value as Reauthentication })
            }}
          >
            <SelectTrigger id="reauthentication" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {REAUTHENTICATION.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">
            Sensitive operations — exports, changes to encryption, passwords and these settings,
            deleting the unencrypted copy — always ask. Switching profiles or projects, device
            actions and workflows never do.
          </p>
        </div>
      </div>
      {verifiable && <LockNow />}
    </Section>
  )
}

function LockNow() {
  const { setView } = useSecurity()
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button
        variant="outline"
        onClick={() => {
          api.lock().then(setView, reportError)
        }}
      >
        <LockKeyhole /> Lock now
      </Button>
      <p className="text-xs text-muted-foreground">
        Stops actions on your Stream Deck until you unlock DeckPilot again.
      </p>
    </div>
  )
}

/** Shown only where the platform offers it (Touch ID on macOS). */
function SystemAuthentication() {
  const { view } = useSecurity()
  if (view.protection !== "systemManaged") return null
  if (view.verification === "touchId" || view.verification === "devicePassword") {
    const touchId = view.verification === "touchId"
    return (
      <Alert>
        <Fingerprint />
        <AlertTitle>{touchId ? "Touch ID" : "System authentication"}</AlertTitle>
        <AlertDescription>
          {touchId
            ? "Touch ID (or your login password) unlocks DeckPilot after it was locked and confirms sensitive operations. macOS checks it; DeckPilot never sees your fingerprint or password."
            : "Your login password unlocks DeckPilot after it was locked and confirms sensitive operations. The operating system checks it."}
        </AlertDescription>
      </Alert>
    )
  }
  return null
}

function Recovery() {
  const { view } = useSecurity()
  return (
    <Section title="Backup and recovery">
      <p className="text-sm text-muted-foreground">
        {view.protection === "systemManaged"
          ? `The key in the ${view.keyStore} protects the configuration on this computer only. It is not a backup: if your user account, the ${view.keyStore} or this computer is lost or reset, the configuration cannot be opened.`
          : "There is no way to recover a forgotten password."}{" "}
        Keep an <strong>encrypted export</strong> (below) as your backup; it has its own password
        and can be imported on any computer.
      </p>
      <p className="text-xs text-muted-foreground">
        Encrypted backups made before database upgrades:{" "}
        <span className="font-mono break-all select-text">{view.backupDir}</span>
      </p>
    </Section>
  )
}

function LegacyCopies() {
  const { view, setView } = useSecurity()
  const sensitive = useSensitive()
  return (
    <Alert>
      <TriangleAlert />
      <AlertTitle>An unencrypted copy of your old configuration remains</AlertTitle>
      <AlertDescription className="grid gap-2">
        <p>
          It was kept when the configuration was encrypted, in case anything went wrong. Anyone with
          access to your files can read it.
        </p>
        <ul className="font-mono text-xs break-all">
          {view.legacyCopies.map((path) => (
            <li key={path}>{path}</li>
          ))}
        </ul>
        <p className="text-xs">
          Deleting a file does not guarantee that its contents cannot be recovered, especially on
          SSDs. See the security documentation for a thorough cleanup.
        </p>
        <ConfirmDialog
          trigger={
            <Button variant="outline" size="sm" className="justify-self-start">
              Delete unencrypted copy
            </Button>
          }
          title="Delete the unencrypted copy?"
          description="Your encrypted configuration is not affected. Deleting the file removes it from the file system; on SSDs and in backups its contents may still be recoverable."
          onConfirm={() => {
            sensitive(() => api.removeUnencryptedCopies(), "delete the unencrypted copy").then(
              (next) => {
                setView(next)
                toast.success("Unencrypted copy deleted")
              },
              reportError,
            )
          }}
        />
      </AlertDescription>
    </Alert>
  )
}

function ChangePassword() {
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
        toast.success("Password changed")
      })
      .catch((e: unknown) => {
        setError(errorMessage(e))
      })
      .finally(() => {
        setBusy(false)
      })
  }
  return (
    <form className="grid max-w-md gap-3" onSubmit={submit}>
      <h4 className="text-sm font-medium">Change password</h4>
      {error && <ErrorNotice error={error} />}
      <PasswordField
        id="current-password"
        label="Current password"
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
        {busy ? "Re-encrypting…" : "Change password"}
      </Button>
    </form>
  )
}
