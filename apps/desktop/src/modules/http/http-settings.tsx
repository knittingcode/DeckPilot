import { KeyRound, Trash2 } from "lucide-react"
import { useEffect, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useTranslation } from "@/i18n"
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
  const { t } = useTranslation()
  const [credentials, setCredentials] = useState<CredentialInfo[] | null>(null)
  useEffect(() => {
    api.listCredentials().then(setCredentials, (e: unknown) => {
      toastFailure(
        t("workspace:http.credentials.loadFailed"),
        e,
        t("workspace:http.credentials.loadNext"),
      )
      setCredentials([])
    })
  }, [t])
  return (
    <SettingsSection
      title={t("workspace:http.credentials.title")}
      description={t("workspace:http.credentials.description")}
    >
      {credentials === null ? null : credentials.length === 0 ? (
        <p className="border-y py-3 text-sm text-muted-foreground">
          {t("workspace:http.credentials.empty")}
        </p>
      ) : (
        <ul className="grid divide-y border-y" aria-label={t("workspace:http.credentials.stored")}>
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
  const { t } = useTranslation()
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
          {t("workspace:http.credentials.replace")}
        </Button>
        <ConfirmDialog
          trigger={
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-7"
              aria-label={t("workspace:http.credentials.delete", { name: credential.id })}
            >
              <Trash2 className="size-4" />
            </Button>
          }
          title={t("workspace:http.credentials.deleteTitle", { name: credential.id })}
          description={t("workspace:http.credentials.deleteDescription")}
          onConfirm={() => {
            api.deleteCredential(credential.id).then(onChange, (e: unknown) => {
              toastFailure(t("workspace:http.credentials.deleteFailed", { name: credential.id }), e)
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
                toast.success(t("workspace:http.credentials.replaced", { name: credential.id }))
              },
              (e: unknown) => {
                toastFailure(
                  t("workspace:http.credentials.replaceFailed", { name: credential.id }),
                  e,
                )
              },
            )
          }}
        >
          <Input
            type="password"
            autoComplete="off"
            aria-label={t("workspace:http.credentials.newSecret", { name: credential.id })}
            value={secret}
            onChange={(e) => {
              setSecret(e.target.value)
            }}
          />
          <Button type="submit" disabled={secret === ""}>
            {t("workspace:http.credentials.save")}
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
  const { t } = useTranslation()
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
            toast.success(t("workspace:http.credentials.storedToast", { name: id }))
          },
          (e: unknown) => {
            toastFailure(t("workspace:http.credentials.storeFailed", { name: id }), e)
          },
        )
      }}
    >
      <h4 className="text-sm font-medium">{t("workspace:http.credentials.add")}</h4>
      <div className="grid gap-2">
        <Label htmlFor="credential-id">{t("workspace:http.credentials.name")}</Label>
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
            ? t("workspace:http.credentials.nameInvalid")
            : taken
              ? t("workspace:http.credentials.nameTaken")
              : t("workspace:http.credentials.nameHint")}
        </p>
      </div>
      <div className="grid gap-2">
        <Label htmlFor="credential-description">
          {t("workspace:http.credentials.descriptionLabel")}
        </Label>
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
        <Label htmlFor="credential-secret">{t("workspace:http.credentials.secret")}</Label>
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
          {t("workspace:http.credentials.secretHint")}
        </p>
      </div>
      <div>
        <Button type="submit" disabled={!validId || secret === ""}>
          {t("workspace:http.credentials.submit")}
        </Button>
      </div>
    </form>
  )
}
