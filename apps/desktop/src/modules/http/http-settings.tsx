import { KeyRound, Trash2 } from "lucide-react"
import { useEffect, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import type { CredentialInfo } from "@/lib/types"
import { ModuleSettingsForm } from "@/settings/pages/module-settings"

/** Credential names: lower case letters, digits, `.`, `_` and `-`. */
export const CREDENTIAL_ID = /^[a-z][a-z0-9._-]{0,63}$/

/** Settings › HTTP: the module's settings and the credentials stored on this computer. */
export function HttpSettings() {
  const { modules } = useAppContext()
  const module = modules.find((m) => m.id === "http")
  return (
    <>
      {module && <ModuleSettingsForm module={module} />}
      <Credentials />
    </>
  )
}

function Credentials() {
  const [credentials, setCredentials] = useState<CredentialInfo[] | null>(null)
  useEffect(() => {
    api.listCredentials().then(setCredentials, (e: unknown) => {
      toastFailure(
        "The credentials could not be loaded",
        e,
        "Unlock DeckPilot and reopen this page",
      )
      setCredentials([])
    })
  }, [])
  return (
    <SettingsSection
      title="Credentials"
      description="Tokens and passwords that requests use by their name. They are stored encrypted on this computer only: never in a workflow, never in an export, and never shown again after saving."
    >
      {credentials === null ? null : credentials.length === 0 ? (
        <p className="border-y py-3 text-sm text-muted-foreground">No credentials are stored.</p>
      ) : (
        <ul className="grid divide-y border-y" aria-label="Stored credentials">
          {credentials.map((credential) => (
            <CredentialRow key={credential.id} credential={credential} onChange={setCredentials} />
          ))}
        </ul>
      )}
      <NewCredential existing={credentials ?? []} onChange={setCredentials} />
    </SettingsSection>
  )
}

function CredentialRow({
  credential,
  onChange,
}: {
  credential: CredentialInfo
  onChange: (credentials: CredentialInfo[]) => void
}) {
  const [secret, setSecret] = useState("")
  const [replacing, setReplacing] = useState(false)
  return (
    <li className="grid gap-2 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <KeyRound className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        <code className="truncate font-mono text-sm">{credential.id}</code>
        <span className="min-w-0 flex-1 truncate text-sm text-muted-foreground">
          {credential.description}
        </span>
        <Button
          variant="ghost"
          size="xs"
          onClick={() => {
            setReplacing(!replacing)
          }}
        >
          Replace secret
        </Button>
        <ConfirmDialog
          trigger={
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-7"
              aria-label={`Delete ${credential.id}`}
            >
              <Trash2 className="size-4" />
            </Button>
          }
          title={`Delete ${credential.id}?`}
          description="Requests that use it will fail until a credential with this name is stored again."
          onConfirm={() => {
            api.deleteCredential(credential.id).then(onChange, (e: unknown) => {
              toastFailure(`${credential.id} was not deleted`, e)
            })
          }}
        />
      </div>
      {replacing && (
        <form
          className="flex gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            api.setCredential(credential.id, credential.description ?? "", secret).then(
              (next) => {
                setSecret("")
                setReplacing(false)
                onChange(next)
                toast.success(`${credential.id} replaced`)
              },
              (e: unknown) => {
                toastFailure(`${credential.id} was not replaced`, e)
              },
            )
          }}
        >
          <Input
            type="password"
            autoComplete="off"
            aria-label={`New secret for ${credential.id}`}
            value={secret}
            onChange={(e) => {
              setSecret(e.target.value)
            }}
          />
          <Button type="submit" disabled={secret === ""}>
            Save
          </Button>
        </form>
      )}
    </li>
  )
}

function NewCredential({
  existing,
  onChange,
}: {
  existing: CredentialInfo[]
  onChange: (credentials: CredentialInfo[]) => void
}) {
  const [id, setId] = useState("")
  const [description, setDescription] = useState("")
  const [secret, setSecret] = useState("")
  const validId = CREDENTIAL_ID.test(id)
  const taken = existing.some((c) => c.id === id)
  return (
    <form
      className="grid max-w-xl gap-3 pt-2"
      onSubmit={(event) => {
        event.preventDefault()
        api.setCredential(id, description, secret).then(
          (next) => {
            onChange(next)
            setId("")
            setDescription("")
            setSecret("")
            toast.success(`${id} stored`)
          },
          (e: unknown) => {
            toastFailure(`${id} was not stored`, e)
          },
        )
      }}
    >
      <h4 className="text-sm font-medium">Add a credential</h4>
      <div className="grid gap-2">
        <Label htmlFor="credential-id">Name</Label>
        <Input
          id="credential-id"
          placeholder="github.token"
          value={id}
          onChange={(e) => {
            setId(e.target.value.trim())
          }}
        />
        <p className="text-xs text-muted-foreground">
          {id !== "" && !validId
            ? "Lower case letters, digits, “.”, “_” and “-”, starting with a letter."
            : taken
              ? "A credential with this name exists; saving replaces it."
              : "What actions refer to, e.g. in an HTTP request's Credential field."}
        </p>
      </div>
      <div className="grid gap-2">
        <Label htmlFor="credential-description">Description</Label>
        <Input
          id="credential-description"
          value={description}
          maxLength={200}
          onChange={(e) => {
            setDescription(e.target.value)
          }}
        />
      </div>
      <div className="grid gap-2">
        <Label htmlFor="credential-secret">Secret</Label>
        <Input
          id="credential-secret"
          type="password"
          autoComplete="off"
          value={secret}
          onChange={(e) => {
            setSecret(e.target.value)
          }}
        />
        <p className="text-xs text-muted-foreground">
          A token, or user:password for basic authentication.
        </p>
      </div>
      <div>
        <Button type="submit" disabled={!validId || secret === ""}>
          Store credential
        </Button>
      </div>
    </form>
  )
}
