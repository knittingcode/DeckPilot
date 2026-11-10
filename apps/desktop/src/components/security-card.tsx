import { LockKeyhole, ShieldCheck, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { ErrorNotice } from "@/components/page"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api, errorMessage } from "@/lib/api"
import { useSecurity } from "@/lib/security-context"

/** Encryption status, password change, locking and legacy copies. */
export function SecurityCard() {
  const { view, setView } = useSecurity()
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="size-4" /> Security
        </CardTitle>
        <CardDescription>
          Your configuration is encrypted on this computer (SQLCipher, with a key derived from your
          password using Argon2id). The password is not stored anywhere.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-6">
        {view.legacyCopies.length > 0 && <LegacyCopies />}
        <ChangePassword />
        <div className="flex flex-wrap items-center gap-3">
          <Button
            variant="outline"
            onClick={() => {
              api.lock().then(setView, (e: unknown) => toast.error(errorMessage(e)))
            }}
          >
            <LockKeyhole /> Lock now
          </Button>
          <p className="text-xs text-muted-foreground">
            Stops actions on your Stream Deck until the password is entered again.
          </p>
        </div>
        <p className="text-xs text-muted-foreground">
          Encrypted backups made before database upgrades:{" "}
          <span className="font-mono break-all select-text">{view.backupDir}</span>
        </p>
      </CardContent>
    </Card>
  )
}

function LegacyCopies() {
  const { view, setView } = useSecurity()
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
            api.removeUnencryptedCopies().then(
              (next) => {
                setView(next)
                toast.success("Unencrypted copy deleted")
              },
              (e: unknown) => toast.error(errorMessage(e)),
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
  const ready = current.length > 0 && password.length >= 8 && password === confirmation
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
  const field = (
    id: string,
    label: string,
    value: string,
    set: (value: string) => void,
    autoComplete: string,
  ) => (
    <div className="grid gap-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="password"
        autoComplete={autoComplete}
        value={value}
        onChange={(e) => {
          set(e.target.value)
        }}
      />
    </div>
  )
  return (
    <form className="grid max-w-md gap-3" onSubmit={submit}>
      <h3 className="text-sm font-medium">Change password</h3>
      {error && <ErrorNotice error={error} />}
      {field("current-password", "Current password", current, setCurrent, "current-password")}
      {field("changed-password", "New password", password, setPassword, "new-password")}
      {field(
        "changed-password-confirmation",
        "Repeat new password",
        confirmation,
        setConfirmation,
        "new-password",
      )}
      {password.length > 0 && password.length < 8 && (
        <p className="text-sm text-destructive">Use at least 8 characters.</p>
      )}
      {confirmation.length > 0 && confirmation !== password && (
        <p className="text-sm text-destructive">The passwords do not match.</p>
      )}
      <Button type="submit" className="justify-self-start" disabled={!ready || busy}>
        {busy ? "Re-encrypting…" : "Change password"}
      </Button>
    </form>
  )
}
