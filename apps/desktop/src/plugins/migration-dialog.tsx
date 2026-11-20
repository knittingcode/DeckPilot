// The one-time consent dialog of a module migration
// (docs/marketplace/module-migration.md §5.5): what the plugin that
// replaces a built-in module may use beyond what is derived, item by item.
// Credentials and the private network always come here; nothing is
// granted until the user allows it.
//
// It lists exactly what the agent says waits (`MigrationConsentItem`):
// the permission, for a credential the stored credential's id, the
// built-in's setting, the plugin's slot and where it is sent, and for a
// server the user entered in the built-in (a self-managed GitLab, a Jira
// site) exactly that server, with reaching it on a private network as an
// item of its own.
// It never shows, asks for or receives a credential's value: only those
// named fields are rendered.

import { KeyRound, Network, Server, ShieldAlert, ShieldCheck } from "lucide-react"
import { useState } from "react"

import { ModalShell } from "@/components/dialogs"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { i18n, useTranslation } from "@/i18n"

import { pluginErrorSentence } from "./dialog-error"
import { grantOfInfo, tierLabel } from "./permissions"
import type { CredentialImport, MigrationConsentItem, ModuleMigration } from "./types"

type Choice = "allow" | "decline"

/** The items still waiting for a decision. */
function waitingItems(migration: ModuleMigration): MigrationConsentItem[] {
  return (migration.consent ?? []).filter((item) => item.decision === "pending")
}

function ImportFacts({ item }: { item: MigrationConsentItem }) {
  const { t } = useTranslation()
  const facts = item.import
  if (!facts) return null
  // Names only, field by field: never anything else the answer may carry.
  return (
    <dl
      className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-0.5 text-xs"
      aria-label={t("plugins:migration.import.label")}
    >
      <dt className="text-muted-foreground">{t("plugins:migration.import.credential")}</dt>
      <dd className="font-mono break-all">{facts.credential}</dd>
      <dt className="text-muted-foreground">{t("plugins:migration.fromSetting")}</dt>
      <dd className="font-mono break-all">{facts.setting}</dd>
      <dt className="text-muted-foreground">{t("plugins:migration.import.slot")}</dt>
      <dd className="font-mono break-all">{facts.slot}</dd>
      <dt className="text-muted-foreground">{t("plugins:migration.import.sentTo")}</dt>
      <dd className="font-mono break-all">{sentTo(facts)}</dd>
    </dl>
  )
}

/** Where an imported credential goes: its hosts, and the server allowed in its origin setting. */
function sentTo(facts: CredentialImport): string {
  const server = facts.userOrigin
    ? i18n.t("plugins:migration.import.allowedServer", { setting: facts.userOrigin })
    : null
  if (facts.hosts.length === 0) return server ?? i18n.t("plugins:migration.import.declaredHosts")
  return server
    ? i18n.t("plugins:list.and", { items: facts.hosts.join(", "), last: server })
    : facts.hosts.join(", ")
}

/** `https://server[:port]` of an origin with a path prefix. */
function serverOf(origin: string): string {
  const slash = origin.indexOf("/", 8)
  return slash < 0 ? origin : origin.slice(0, slash)
}

/** What allowing a server means, fact by fact. */
function OriginFacts({ item }: { item: MigrationConsentItem }) {
  const { t } = useTranslation()
  const facts = item.origin
  if (!facts) return null
  return (
    <dl
      className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-0.5 text-xs"
      aria-label={t("plugins:migration.origin.label")}
    >
      <dt className="text-muted-foreground">{t("plugins:migration.origin.server")}</dt>
      <dd className="font-mono break-all">{facts.origin}</dd>
      <dt className="text-muted-foreground">{t("plugins:migration.fromSetting")}</dt>
      <dd className="font-mono break-all">{facts.setting}</dd>
      {!facts.privateNetwork && (
        <>
          <dt className="text-muted-foreground">{t("plugins:migration.origin.requests")}</dt>
          <dd>
            {t(
              facts.origin.includes("/", 8)
                ? "plugins:migration.origin.requestsPath"
                : "plugins:migration.origin.requestsServer",
              {
                methods: facts.methods?.length
                  ? facts.methods.join(", ")
                  : t("plugins:migration.origin.anyMethod"),
              },
            )}
          </dd>
          {facts.credentials?.length ? (
            <>
              <dt className="text-muted-foreground">{t("plugins:migration.origin.secrets")}</dt>
              <dd className="font-mono break-all">{facts.credentials.join(", ")}</dd>
            </>
          ) : null}
          {facts.opensPages && (
            <>
              <dt className="text-muted-foreground">{t("plugins:migration.origin.pages")}</dt>
              <dd>{t("plugins:migration.origin.pagesOf", { server: serverOf(facts.origin) })}</dd>
            </>
          )}
        </>
      )}
    </dl>
  )
}

/** The title of an item: what the user is asked to allow. */
function titleOf(item: MigrationConsentItem, fallback: string): string {
  if (item.origin?.privateNetwork) {
    return i18n.t("plugins:migration.item.privateNetwork", { server: item.origin.origin })
  }
  if (item.origin) return i18n.t("plugins:migration.item.connect", { server: item.origin.origin })
  if (item.import) {
    return i18n.t("plugins:migration.item.credential", { credential: item.import.credential })
  }
  return fallback
}

/**
 * Asks, item by item, whether the plugin replacing `migration.moduleName`
 * may have what the migration waits for. Each item is allowed or declined
 * on its own; there is no "allow all". Confirming sends exactly those
 * decisions; "Not now" decides nothing and the built-in keeps running.
 */
export function MigrationConsentDialog({
  migration,
  onDecide,
  onClose,
}: {
  migration: ModuleMigration
  /** Sends the decisions; resolves when the agent applied them. */
  onDecide: (approve: string[], decline: string[]) => Promise<void>
  onClose: () => void
}) {
  const { t } = useTranslation()
  const items = waitingItems(migration)
  const [choices, setChoices] = useState<Map<string, Choice>>(new Map())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const undecided = items.filter((item) => !choices.has(item.key)).length
  const plugin = migration.pluginName
    ? `${migration.pluginName} (${migration.plugin})`
    : migration.plugin

  const choose = (key: string, choice: Choice) => {
    setChoices((previous) => new Map(previous).set(key, choice))
  }

  const submit = () => {
    const approve = items.filter((i) => choices.get(i.key) === "allow").map((i) => i.key)
    const decline = items.filter((i) => choices.get(i.key) === "decline").map((i) => i.key)
    setBusy(true)
    setError(null)
    onDecide(approve, decline).then(onClose, (e: unknown) => {
      setError(pluginErrorSentence(e))
      setBusy(false)
    })
  }

  return (
    <ModalShell
      presentation="dialog"
      size="medium"
      icon={ShieldCheck}
      title={t("plugins:migration.dialog.title", { module: migration.moduleName })}
      description={t("plugins:migration.dialog.description", {
        module: migration.moduleName,
        plugin,
      })}
      error={error}
      status={
        <span aria-live="polite">
          {undecided === 0
            ? [...choices.values()].includes("decline")
              ? t("plugins:migration.dialog.declined", { module: migration.moduleName })
              : t("plugins:migration.dialog.allAllowed")
            : t("plugins:migration.dialog.undecided", { undecided, total: items.length })}
        </span>
      }
      onClose={() => {
        if (!busy) onClose()
      }}
      onSubmit={submit}
      secondary={{ label: t("plugins:migration.dialog.notNow"), onClick: onClose, disabled: busy }}
      primary={{
        label: t("plugins:migration.dialog.confirm"),
        loadingLabel: t("common:status.working"),
        loading: busy,
        disabled: items.length === 0 || undecided > 0,
      }}
    >
      <p className="flex items-start gap-2 text-xs text-muted-foreground">
        <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
        <span>{t("plugins:migration.dialog.never")}</span>
      </p>
      {items.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("plugins:migration.dialog.nothing")}</p>
      ) : (
        <ul className="grid gap-2" aria-label={t("plugins:migration.dialog.list")}>
          {items.map((item) => {
            const grant = grantOfInfo(item.permission)
            const tier = tierLabel(grant.tier)
            const choice = choices.get(item.key)
            const name = `consent-${item.key}`
            return (
              <li
                key={item.key}
                className="grid gap-2 rounded-lg border p-3"
                aria-labelledby={`${name}-title`}
              >
                <div className="flex flex-wrap items-center gap-2">
                  {item.origin?.privateNetwork ? (
                    <Network className="size-4" aria-hidden />
                  ) : item.origin ? (
                    <Server className="size-4" aria-hidden />
                  ) : (
                    item.import && <KeyRound className="size-4" aria-hidden />
                  )}
                  <span id={`${name}-title`} className="font-medium break-words">
                    {titleOf(item, grant.title)}
                  </span>
                  {tier && (
                    <Badge variant={grant.tier === "high" ? "destructive" : "outline"}>
                      {tier}
                    </Badge>
                  )}
                </div>
                <span className="font-mono text-xs text-muted-foreground">{item.key}</span>
                <p className="text-xs">{item.why}</p>
                {grant.detail && !item.import && !item.origin && (
                  <p className="text-xs text-muted-foreground">{grant.detail}</p>
                )}
                <ImportFacts item={item} />
                <OriginFacts item={item} />
                {grant.justification && (
                  <p className="text-xs">
                    <span className="text-muted-foreground">
                      {t("permissions:list.publisherReason")}{" "}
                    </span>
                    {grant.justification}
                  </p>
                )}
                <div
                  role="radiogroup"
                  aria-label={t("plugins:migration.dialog.decision", { key: item.key })}
                  className="flex flex-wrap gap-2"
                >
                  {(["allow", "decline"] as const).map((value) => (
                    <Button
                      key={value}
                      type="button"
                      size="sm"
                      role="radio"
                      aria-checked={choice === value}
                      variant={
                        choice !== value
                          ? "outline"
                          : value === "decline"
                            ? "destructive"
                            : "default"
                      }
                      disabled={busy}
                      onClick={() => {
                        choose(item.key, value)
                      }}
                    >
                      {value === "allow"
                        ? t("permissions:review.allow")
                        : t("plugins:migration.dialog.dontAllow")}
                    </Button>
                  ))}
                </div>
              </li>
            )
          })}
        </ul>
      )}
    </ModalShell>
  )
}
