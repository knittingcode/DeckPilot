import { KeyRound, Trash2 } from "lucide-react"
import { useEffect, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { api, errorMessage } from "@/lib/api"
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
      toast.error(errorMessage(e))
      setCredentials([])
    })
  }, [])
  return (
    <Card>
      <CardHeader>
        <CardTitle>Credentials</CardTitle>
        <CardDescription>
          Tokens and passwords that requests use by their name. They are stored encrypted on this
          computer only: never in a workflow, never in an export, and never shown again after
          saving.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid max-w-xl gap-4">
        {credentials === null ? null : credentials.length === 0 ? (
          <p className="text-sm text-muted-foreground">No credentials are stored.</p>
        ) : (
          <ul className="grid gap-2" aria-label="Stored credentials">
            {credentials.map((credential) => (
              <CredentialRow
                key={credential.id}
                credential={credential}
                onChange={setCredentials}
              />
            ))}
          </ul>
        )}
        <NewCredential existing={credentials ?? []} onChange={setCredentials} />
      </CardContent>
    </Card>
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
    <li className="grid gap-2 rounded-md border px-3 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <KeyRound className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        <code className="truncate font-mono text-sm">{credential.id}</code>
        <span className="min-w-0 flex-1 truncate text-sm text-muted-foreground">
          {credential.description}
        </span>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            setReplacing(!replacing)
          }}
        >
          Replace secret
        </Button>
        <ConfirmDialog
          trigger={
            <Button variant="ghost" size="icon" aria-label={`Delete ${credential.id}`}>
              <Trash2 className="size-4" />
            </Button>
          }
          title={`Delete ${credential.id}?`}
          description="Requests that use it will fail until a credential with this name is stored again."
          onConfirm={() => {
            api.deleteCredential(credential.id).then(onChange, (e: unknown) => {
              toast.error(errorMessage(e))
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
              (e: unknown) => toast.error(errorMessage(e)),
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
      className="grid gap-3 border-t pt-4"
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
          (e: unknown) => toast.error(errorMessage(e)),
        )
      }}
    >
      <h3 className="text-sm font-medium">Add a credential</h3>
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
