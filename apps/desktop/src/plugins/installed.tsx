import {
  ArchiveRestore,
  ChevronDown,
  Play,
  RefreshCw,
  RotateCcw,
  Settings2,
  Trash2,
  Wrench,
} from "lucide-react"
import { useState } from "react"

import { DestructiveDialog } from "@/components/dialogs"
import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { i18n, useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { translateErrorCode } from "@/lib/backend-error"
import { cn } from "@/lib/utils"

import { pluginErrorText, type PluginErrorText } from "./api"
import { errorSentence } from "./dialog-error"
import { Notice, PermissionList, PluginIcon, PublisherLine } from "./components"
import { ConfigureDialog } from "./configure"
import type { MarketplaceActions } from "./marketplace"
import {
  declaredConfiguration,
  formatTime,
  manifestCompatibility,
  platformLabel,
  platformsText,
  stateLabel,
  storageText,
  type UpdateCandidate,
} from "./model"
import { grantOfInfo } from "./permissions"
import type {
  ActionParamValue,
  AvailablePlugin,
  KnownPlugin,
  MarketplaceSnapshot,
  PluginActionInfo,
  PluginActionOutcome,
  PluginDetails,
  PluginRetention,
} from "./types"

export function InstalledView({
  snapshot,
  updates,
  actions,
  onBrowse,
}: {
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  actions: MarketplaceActions
  onBrowse: () => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  // DeckPilot's own plugins the user removed, which the agent can restore.
  const removed = actions.canRestore
    ? (snapshot.known ?? [])
        .filter((k) => k.restorable === true && !k.installed)
        .sort((a, b) => format.compare(a.name, b.name))
    : []
  if (snapshot.installed.length === 0) {
    return (
      <>
        <EmptyState variant="plain" title={t("marketplace:installed.empty")}>
          <Button variant="outline" size="sm" className="mt-2" onClick={onBrowse}>
            {t("marketplace:installed.browse")}
          </Button>
        </EmptyState>
        <RemovedBundledList removed={removed} actions={actions} />
      </>
    )
  }
  const byId = new Map(updates.map((u) => [u.plugin.summary.id, u]))
  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="grid gap-0.5">
          <h3 className="text-sm font-medium">{t("marketplace:classes.installed.title")}</h3>
          <p className="text-sm text-muted-foreground">{t("marketplace:installed.intro")}</p>
        </div>
        <Button
          variant="ghost"
          size="sm"
          disabled={actions.busy !== null}
          onClick={actions.refresh}
        >
          <RefreshCw /> {t("marketplace:refresh")}
        </Button>
      </div>
      <ul className="grid divide-y border-y" aria-label={t("marketplace:installed.list")}>
        {[...snapshot.installed]
          .sort((a, b) => format.compare(a.summary.name, b.summary.name))
          .map((item) => (
            <InstalledItem
              key={item.summary.id}
              item={item}
              listed={snapshot.available.find((p) => p.id === item.summary.id)}
              platform={snapshot.runtime.platform}
              runtimeReason={snapshot.runtime.reason}
              missingSettings={snapshot.missingSettings?.[item.summary.id] ?? []}
              update={byId.get(item.summary.id)}
              actions={actions}
            />
          ))}
      </ul>
      <RemovedBundledList removed={removed} actions={actions} />
    </>
  )
}

/**
 * Plugins that come with DeckPilot and that the user uninstalled: each can
 * be restored from DeckPilot's own copy (`restoreBundledPlugin`). Nothing
 * is shown when there are none.
 */
function RemovedBundledList({
  removed,
  actions,
}: {
  removed: KnownPlugin[]
  actions: MarketplaceActions
}) {
  const { t } = useTranslation()
  if (removed.length === 0) return null
  return (
    <section className="grid gap-2" aria-labelledby="removed-bundled-title">
      <div className="grid gap-0.5">
        <h3 id="removed-bundled-title" className="text-sm font-medium">
          {t("marketplace:installed.removed.title")}
        </h3>
        <p className="text-sm text-muted-foreground">{t("marketplace:installed.removed.intro")}</p>
      </div>
      <ul className="grid divide-y border-y" aria-label={t("marketplace:installed.removed.list")}>
        {removed.map((plugin) => (
          <li key={plugin.id} className="flex items-center gap-3 py-2">
            <div className="grid min-w-0 flex-1 gap-0.5">
              <span className="truncate text-sm font-medium">{plugin.name || plugin.id}</span>
              <span className="text-xs text-muted-foreground">
                {t("marketplace:installed.removed.lastVersion", { version: plugin.lastVersion })}
              </span>
            </div>
            <Button
              variant="outline"
              size="sm"
              disabled={actions.busy !== null}
              aria-label={t("marketplace:installed.removed.restoreNamed", {
                name: plugin.name || plugin.id,
              })}
              onClick={() => {
                actions.restore(plugin.id, plugin.name || plugin.id, false)
              }}
            >
              <ArchiveRestore /> {t("marketplace:installed.removed.restore")}
            </Button>
          </li>
        ))}
      </ul>
    </section>
  )
}

/** Failures of the plugin's host since the agent started. */
function HealthNotice({ item }: { item: PluginDetails }) {
  const { t } = useTranslation()
  const health = item.summary.health
  if (!health || (health.crashes === 0 && !health.lastError)) return null
  const title =
    health.crashes > 0
      ? t("marketplace:installed.health.crashes", { count: health.crashes })
      : t("marketplace:installed.health.lastFailed")
  return (
    <div
      className="mt-3"
      role="group"
      aria-label={t("marketplace:installed.health.label", { name: item.summary.name })}
    >
      <Notice tone={health.crashes > 0 ? "destructive" : "default"} title={title}>
        {health.lastError && (
          <p className="select-text">
            {health.lastErrorAt !== undefined
              ? t("marketplace:installed.health.lastErrorAt", {
                  time: formatTime(health.lastErrorAt),
                  error: health.lastError,
                })
              : t("marketplace:installed.health.lastError", { error: health.lastError })}
          </p>
        )}
      </Notice>
    </div>
  )
}

function InstalledItem({
  item,
  listed,
  platform,
  runtimeReason,
  missingSettings,
  update,
  actions,
}: {
  item: PluginDetails
  /** The registry's entry of the plugin, when the registry lists it. */
  listed: AvailablePlugin | undefined
  platform: string
  runtimeReason: string | undefined
  /** Labels of required settings without a value or default. */
  missingSettings: string[]
  update: UpdateCandidate | undefined
  actions: MarketplaceActions
}) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [configuring, setConfiguring] = useState(false)
  const declared = declaredConfiguration(item.manifest)
  const configurable = declared.settings.length > 0 || declared.credentials.length > 0
  const compatibility = manifestCompatibility(item.manifest)
  const { summary } = item
  const id = `installed-${summary.id}`
  const enabled = summary.state === "enabled"
  const busy = actions.busy !== null
  const notGranted = item.permissions.filter((p) => p.status !== "granted").length
  const blockedState = summary.state === "broken" || summary.state === "quarantined"
  // A broken copy of one of DeckPilot's own plugins: reinstalled from the
  // copy that comes with DeckPilot.
  const reinstallable =
    actions.canRestore && summary.origin.type === "bundled" && summary.state === "broken"
  const updateVersion = update
    ? update.kind === "staged"
      ? update.pending.version
      : update.version
    : undefined

  return (
    <li className="py-3" aria-labelledby={`${id}-name`}>
      <div className="flex items-start gap-3">
        <PluginIcon muted={!enabled} src={item.icon} />
        <div className="grid min-w-0 flex-1 gap-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span id={`${id}-name`} className="text-sm font-medium">
              {summary.name}
            </span>
            <span className="text-xs text-muted-foreground">v{summary.version}</span>
            {summary.origin.type === "dev" && (
              <Badge variant="outline">{t("marketplace:installed.origin.dev")}</Badge>
            )}
            {summary.origin.type === "sideload" && (
              <Badge variant="outline">{t("marketplace:installed.origin.sideload")}</Badge>
            )}
            {summary.origin.type === "bundled" && (
              <Badge variant="outline">{t("marketplace:installed.origin.bundled")}</Badge>
            )}
            <Badge variant={blockedState ? "destructive" : "outline"}>
              {stateLabel(summary.state)}
            </Badge>
            {updateVersion && (
              <Badge>{t("marketplace:installed.updateBadge", { version: updateVersion })}</Badge>
            )}
            {missingSettings.length > 0 && (
              <Badge variant="destructive">{t("marketplace:installed.settingsMissing")}</Badge>
            )}
          </div>
          <PublisherLine publisher={summary.publisher} info={listed?.publisherInfo} />
          <span className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
            {compatibility.deckpilot && <span>DeckPilot {compatibility.deckpilot}</span>}
            <span>{platformsText(compatibility.platforms)}</span>
            {listed?.latest && listed.latest !== summary.version && (
              <span>{t("marketplace:installed.latest", { version: listed.latest })}</span>
            )}
          </span>
          {item.storage && (
            <span className="text-xs text-muted-foreground">
              {t("marketplace:installed.storage", { usage: storageText(item.storage) })}
            </span>
          )}
          {notGranted > 0 && (
            <p className="text-xs text-muted-foreground">
              {t("marketplace:installed.notGranted", { count: notGranted })}
            </p>
          )}
        </div>
        <Switch
          checked={enabled}
          disabled={busy || blockedState || (!enabled && !actions.canRun)}
          aria-label={
            enabled
              ? t("marketplace:installed.disableNamed", { name: summary.name })
              : t("marketplace:installed.enableNamed", { name: summary.name })
          }
          onCheckedChange={(value) => {
            actions.setEnabled(item, value)
          }}
        />
      </div>

      {(blockedState || summary.stateReason) && (
        <div className="mt-3">
          <Notice
            tone={blockedState ? "destructive" : "default"}
            title={
              summary.state === "broken"
                ? t("marketplace:installed.broken", { name: summary.name })
                : summary.state === "quarantined"
                  ? t("marketplace:installed.quarantined")
                  : t("marketplace:installed.stateOf", {
                      name: summary.name,
                      state: stateLabel(summary.state).toLowerCase(),
                    })
            }
          >
            {summary.stateReason && <p>{summary.stateReason}</p>}
            {blockedState && (
              <p>
                {reinstallable
                  ? t("marketplace:installed.brokenBundledHint")
                  : t("marketplace:installed.blockedHint")}
              </p>
            )}
          </Notice>
        </div>
      )}

      {!actions.canRun && (
        <div className="mt-3">
          <Notice
            title={t("marketplace:installed.cannotRun", { platform: platformLabel(platform) })}
          >
            {runtimeReason && <p>{runtimeReason}</p>}
            <p>{t("marketplace:installed.cannotRunHint")}</p>
          </Notice>
        </div>
      )}

      <HealthNotice item={item} />

      {missingSettings.length > 0 && (
        <div className="mt-3">
          <Notice title={t("plugins:settings.missing.title")}>
            <p>
              {t("marketplace:installed.missingSettings", {
                count: missingSettings.length,
                settings: missingSettings.join(", "),
              })}
            </p>
          </Notice>
        </div>
      )}

      {update?.kind === "staged" && (
        <div className="mt-3">
          <Notice
            title={t("marketplace:installed.staged.title", { version: update.pending.version })}
          >
            <p>
              {t("marketplace:installed.staged.text", {
                count:
                  update.pending.changes.added.length + update.pending.changes.broadened.length,
              })}
            </p>
          </Notice>
        </div>
      )}

      <div className="mt-3 flex flex-wrap gap-2 pl-13">
        {reinstallable && (
          <Button
            size="sm"
            disabled={busy}
            aria-label={t("marketplace:installed.reinstallNamed", { name: summary.name })}
            onClick={() => {
              actions.restore(summary.id, summary.name, true)
            }}
          >
            <Wrench /> {t("marketplace:installed.reinstall")}
          </Button>
        )}
        {update && (
          <Button
            size="sm"
            disabled={busy || actions.blocked !== null}
            onClick={() => {
              actions.update(update)
            }}
          >
            <RefreshCw />
            {update.kind === "staged"
              ? t("marketplace:installed.reviewUpdate", { version: updateVersion ?? "" })
              : t("marketplace:details.updateTo", { version: updateVersion ?? "" })}
          </Button>
        )}
        {summary.previousVersion && (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => {
              actions.rollback(item)
            }}
          >
            <RotateCcw />{" "}
            {t("marketplace:installed.rollBack", { version: summary.previousVersion })}
          </Button>
        )}
        {listed && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              actions.openDetails(summary.id)
            }}
          >
            {t("marketplace:installed.details")}
          </Button>
        )}
        {configurable && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setConfiguring(true)
            }}
          >
            <Settings2 /> {t("marketplace:installed.configure")}
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          aria-expanded={expanded}
          aria-controls={`${id}-more`}
          onClick={() => {
            setExpanded(!expanded)
          }}
        >
          <ChevronDown className={cn("transition-transform", expanded && "rotate-180")} />
          {t("marketplace:installed.more")}
        </Button>
        <UninstallButton
          item={item}
          disabled={busy}
          actions={actions}
          declares={{
            settings: declared.settings.length > 0,
            credentials: declared.credentials.length > 0,
          }}
        />
      </div>

      {expanded && (
        <div id={`${id}-more`} className="mt-3 grid gap-4 border-t pt-3">
          <PermissionList
            label={t("marketplace:installed.permissionsOf", { name: summary.name })}
            grants={item.permissions.map(grantOfInfo)}
            renderControl={(grant) => {
              const info = item.permissions.find((p) => p.key === grant.key)
              if (info?.status === "granted") {
                return (
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    aria-label={t("marketplace:installed.revokeNamed", { title: grant.title })}
                    onClick={() => {
                      actions.setGrant(item, grant.key, false)
                    }}
                  >
                    {t("marketplace:installed.revoke")}
                  </Button>
                )
              }
              return (
                <div className="flex shrink-0 items-center gap-2">
                  <Badge variant="outline">
                    {info?.status === "revoked"
                      ? t("marketplace:installed.revoked")
                      : t("marketplace:installed.notGrantedBadge")}
                  </Badge>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    aria-label={t("marketplace:installed.grantNamed", { title: grant.title })}
                    onClick={() => {
                      actions.setGrant(item, grant.key, true)
                    }}
                  >
                    {t("marketplace:installed.grant")}
                  </Button>
                </div>
              )
            }}
          />
          <ActionList item={item} actions={actions} />
        </div>
      )}
      {configuring && (
        <ConfigureDialog
          item={item}
          client={actions.client}
          onChanged={actions.reload}
          onClose={() => {
            setConfiguring(false)
          }}
        />
      )}
    </li>
  )
}

interface ParameterSpec {
  key: string
  label: string
  type: string
  options?: string[]
  defaultValue?: ActionParamValue
}

/** The manifest's parameters of `action` (`contributes.actions[].parameters`). */
function actionParameters(manifest: unknown, action: string): ParameterSpec[] {
  const record = (v: unknown) =>
    typeof v === "object" && v !== null ? (v as Record<string, unknown>) : undefined
  const actions = record(record(manifest)?.contributes)?.actions
  if (!Array.isArray(actions)) return []
  const entry = actions.map(record).find((a) => a?.id === action)
  const parameters = entry?.parameters
  if (!Array.isArray(parameters)) return []
  return parameters.flatMap((p): ParameterSpec[] => {
    const r = record(p)
    if (!r || typeof r.key !== "string") return []
    const kind = record(r.kind)
    const spec: ParameterSpec = {
      key: r.key,
      label: typeof r.label === "string" ? r.label : r.key,
      type: typeof kind?.type === "string" ? kind.type : "text",
    }
    if (Array.isArray(kind?.options)) {
      spec.options = kind.options.filter((o): o is string => typeof o === "string")
    }
    const value = record(r.default)?.value
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
      spec.defaultValue = value
    }
    return [spec]
  })
}

function ActionList({ item, actions }: { item: PluginDetails; actions: MarketplaceActions }) {
  const { t } = useTranslation()
  if (item.summary.actions.length === 0) return null
  return (
    <section
      className="grid gap-2"
      aria-label={t("marketplace:installed.actionsOf", { name: item.summary.name })}
    >
      <h4 className="text-xs font-medium text-muted-foreground uppercase">
        {t("marketplace:details.actions")}
      </h4>
      <ul className="grid divide-y border-y">
        {item.summary.actions.map((action) => (
          <ActionRow key={action.action} item={item} action={action} actions={actions} />
        ))}
      </ul>
    </section>
  )
}

function ActionRow({
  item,
  action,
  actions,
}: {
  item: PluginDetails
  action: PluginActionInfo
  actions: MarketplaceActions
}) {
  const { t } = useTranslation()
  const parameters = actionParameters(item.manifest, action.action)
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(
      parameters.map((p) => [p.key, p.defaultValue === undefined ? "" : String(p.defaultValue)]),
    ),
  )
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<
    { outcome: PluginActionOutcome } | { error: PluginErrorText } | null
  >(null)

  const run = () => {
    const params: Record<string, ActionParamValue> = {}
    for (const p of parameters) {
      const raw = values[p.key] ?? ""
      if (raw === "") continue
      params[p.key] =
        p.type === "number" ? Number(raw) : p.type === "boolean" ? raw === "true" : raw
    }
    setRunning(true)
    setResult(null)
    actions
      .invoke(item, action.action, params)
      .then(
        (outcome) => {
          setResult({ outcome })
        },
        (error: unknown) => {
          setResult({ error: pluginErrorText(error) })
        },
      )
      .finally(() => {
        setRunning(false)
      })
  }

  const id = `action-${item.summary.id}-${action.action}`
  return (
    <li className="grid gap-2 py-2.5" aria-labelledby={`${id}-name`}>
      <div className="flex flex-wrap items-center gap-2">
        <span id={`${id}-name`} className="text-sm font-medium">
          {action.name}
        </span>
        <span className="font-mono text-xs text-muted-foreground">{action.actionType}</span>
        <Button
          size="sm"
          variant="outline"
          className="ml-auto"
          disabled={running || !action.available}
          aria-label={t("marketplace:installed.run.named", { name: action.name })}
          onClick={run}
        >
          <Play /> {t("marketplace:installed.run.button")}
        </Button>
      </div>
      {action.description && <p className="text-xs text-muted-foreground">{action.description}</p>}
      {!action.available && action.unavailableReason && (
        <p className="text-xs text-muted-foreground">{action.unavailableReason}</p>
      )}
      {action.available && parameters.length > 0 && (
        <div className="grid gap-2 sm:grid-cols-2">
          {parameters.map((p) => (
            <label key={p.key} className="grid gap-1 text-xs">
              <span>{p.label}</span>
              <Input
                value={values[p.key] ?? ""}
                type={p.type === "number" ? "number" : "text"}
                placeholder={p.options ? p.options.join(" / ") : undefined}
                onChange={(e) => {
                  setValues({ ...values, [p.key]: e.target.value })
                }}
              />
            </label>
          ))}
        </div>
      )}
      {result && (
        <div
          role="status"
          aria-label={t("marketplace:installed.run.result", { name: action.name })}
          className="text-xs"
        >
          {"error" in result ? (
            <p className="text-destructive">{errorSentence(result.error)}</p>
          ) : result.outcome.status === "ok" ? (
            <div className="grid gap-1">
              <p>
                {result.outcome.message
                  ? t("marketplace:installed.run.succeededWith", {
                      message: result.outcome.message,
                    })
                  : t("marketplace:installed.run.succeeded")}
              </p>
              {result.outcome.data && Object.keys(result.outcome.data).length > 0 && (
                <pre className="relative overflow-x-auto rounded bg-muted p-2 select-text">
                  {JSON.stringify(result.outcome.data, null, 2)}
                </pre>
              )}
            </div>
          ) : result.outcome.status === "skipped" ? (
            <p>
              {result.outcome.message
                ? t("marketplace:installed.run.skippedWith", { message: result.outcome.message })
                : t("marketplace:installed.run.skipped")}
            </p>
          ) : (
            <p className="text-destructive">
              {failedText(result.outcome.code, result.outcome.message)}
            </p>
          )}
        </div>
      )}
    </li>
  )
}

/**
 * A failed run: the code, and the plugin's message or else what the shared
 * code means (`errors:pluginAction.*`).
 */
function failedText(code: string, message: string | undefined): string {
  const detail = message ?? translateErrorCode(`pluginAction.${code}`)
  return detail
    ? i18n.t("marketplace:installed.run.failedWith", { code, message: detail })
    : i18n.t("marketplace:installed.run.failed", { code })
}

function UninstallButton({
  item,
  disabled,
  actions,
  declares,
}: {
  item: PluginDetails
  disabled: boolean
  actions: MarketplaceActions
  /** What the plugin declares, so what there is to keep. */
  declares: { settings: boolean; credentials: boolean }
}) {
  // The documented default (design §5.6): everything goes.
  const { t } = useTranslation()
  const [keep, setKeep] = useState<PluginRetention>({})
  const name = item.summary.name
  return (
    <DestructiveDialog
      onOpenChange={(open) => {
        if (open) setKeep({})
      }}
      trigger={
        <Button variant="ghost" size="sm" disabled={disabled}>
          <Trash2 /> {t("marketplace:uninstall.button")}
        </Button>
      }
      title={t("marketplace:uninstall.title", { name })}
      description={t("marketplace:uninstall.description")}
      confirmLabel={t("marketplace:uninstall.button")}
      onConfirm={() => {
        actions.uninstall(item, keep)
      }}
    >
      {(declares.settings || declares.credentials) && (
        <fieldset className="grid gap-2 text-sm">
          <legend className="mb-1 font-medium">{t("marketplace:uninstall.keep")}</legend>
          {declares.settings && (
            <label className="flex items-start gap-2">
              <Checkbox
                checked={keep.settings === true}
                aria-label={t("marketplace:uninstall.keepSettings")}
                onCheckedChange={(checked) => {
                  setKeep({ ...keep, settings: checked === true })
                }}
              />
              <span>{t("marketplace:uninstall.keepSettings")}</span>
            </label>
          )}
          {declares.credentials && (
            <label className="flex items-start gap-2">
              <Checkbox
                checked={keep.credentials === true}
                aria-label={t("marketplace:uninstall.keepCredentials")}
                onCheckedChange={(checked) => {
                  setKeep({ ...keep, credentials: checked === true })
                }}
              />
              <span>
                {t("marketplace:uninstall.keepCredentialsLong", {
                  keep: t("marketplace:uninstall.keepCredentials"),
                })}
              </span>
            </label>
          )}
        </fieldset>
      )}
    </DestructiveDialog>
  )
}
