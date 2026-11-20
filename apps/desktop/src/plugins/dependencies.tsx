// Plugin dependencies in the marketplace (plugin API 1.4;
// docs/marketplace/dependencies.md): what a plugin needs, the full install
// plan with every plugin's own permissions, and what disabling or
// uninstalling a plugin affects. Compact rows in the settings layout, no
// cards.

import { Check, Loader2, ShieldAlert, ShieldCheck, TriangleAlert } from "lucide-react"
import { useState } from "react"

import { ModalShell } from "@/components/dialogs"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Trans, type TranslationKey, useTranslation } from "@/i18n"

import { Notice, PermissionList, PublisherLine } from "./components"
import { pluginErrorSentence } from "./dialog-error"
import { grantOfInfo } from "./permissions"
import { dependencyStateText } from "./dependency-text"
import type {
  PluginDependencyInfo,
  PluginDependentInfo,
  PluginInstallPlan,
  PluginPlanStep,
  PluginRemovalPlan,
} from "./types"

/**
 * The dependencies of a plugin version: the required ones, then the
 * optional integrations with what they add. Empty: nothing is shown.
 */
export function DependencySection({
  dependencies,
  title,
}: {
  dependencies: PluginDependencyInfo[]
  title: string
}) {
  const { t } = useTranslation()
  if (dependencies.length === 0) return null
  const required = dependencies.filter((d) => !d.optional)
  const optional = dependencies.filter((d) => d.optional)
  return (
    <section aria-labelledby="plugin-detail-dependencies" className="grid gap-2">
      <h4 id="plugin-detail-dependencies" className="text-sm font-medium">
        {title}
      </h4>
      {required.length > 0 && (
        <DependencyRows label={t("plugins:dependencies.requires")} items={required} />
      )}
      {optional.length > 0 && (
        <>
          <p className="text-xs text-muted-foreground">{t("plugins:dependencies.optionalHint")}</p>
          <DependencyRows label={t("plugins:dependencies.optional")} items={optional} />
        </>
      )}
    </section>
  )
}

function DependencyRows({ label, items }: { label: string; items: PluginDependencyInfo[] }) {
  const { t } = useTranslation()
  return (
    <ul className="grid divide-y border-y text-sm" aria-label={label}>
      {items.map((d) => (
        <li key={d.id} className="grid gap-0.5 py-1.5">
          <span className="flex flex-wrap items-center gap-2">
            {d.name && <span className="font-medium">{d.name}</span>}
            <span
              className={d.name ? "font-mono text-xs text-muted-foreground" : "font-mono text-xs"}
            >
              {d.id}
            </span>
            <span className="text-xs text-muted-foreground">{d.version}</span>
            {d.builtin && <Badge variant="outline">{t("plugins:dependencies.builtIn")}</Badge>}
            {d.optional && (
              <Badge variant="outline">{t("plugins:dependencies.optionalBadge")}</Badge>
            )}
            <span
              className={
                d.state === "satisfied"
                  ? "ml-auto text-xs text-muted-foreground"
                  : "ml-auto text-xs font-medium"
              }
            >
              {dependencyStateText(d)}
            </span>
          </span>
          {d.justification && (
            <span className="text-xs text-muted-foreground">{d.justification}</span>
          )}
          {(d.actions ?? []).length > 0 && (
            <span className="text-xs text-muted-foreground">
              <Trans
                i18nKey="plugins:dependencies.usedByActions"
                values={{ actions: (d.actions ?? []).join(", ") }}
                components={{ code: <span className="font-mono" /> }}
              />
            </span>
          )}
        </li>
      ))}
    </ul>
  )
}

/** The installed plugins that declare a dependency on this one. */
export function DependentsSection({ dependents }: { dependents: PluginDependentInfo[] }) {
  const { t } = useTranslation()
  if (dependents.length === 0) return null
  return (
    <section aria-labelledby="plugin-detail-dependents" className="grid gap-2">
      <h4 id="plugin-detail-dependents" className="text-sm font-medium">
        {t("plugins:dependencies.usedBy")}
      </h4>
      <ul className="grid divide-y border-y text-sm" aria-label={t("plugins:dependencies.usedBy")}>
        {dependents.map((d) => (
          <li key={d.id} className="flex flex-wrap items-center gap-2 py-1.5">
            <span className="font-medium">{d.name}</span>
            <span className="font-mono text-xs text-muted-foreground">{d.id}</span>
            <span className="text-xs text-muted-foreground">
              {d.optional
                ? t("plugins:dependencies.dependentOptional", { version: d.version })
                : t("plugins:dependencies.dependentRequires", { version: d.version })}
            </span>
            {!d.enabled && <Badge variant="outline">{t("plugins:dependencies.off")}</Badge>}
          </li>
        ))}
      </ul>
    </section>
  )
}

const ACTION_LABELS = {
  install: "plugins:plan.action.install",
  update: "plugins:plan.action.update",
  enable: "plugins:plan.action.enable",
  keep: "plugins:plan.action.keep",
  builtin: "plugins:plan.action.builtin",
  enableBuiltin: "plugins:plan.action.enableBuiltin",
} as const satisfies Record<PluginPlanStep["action"], TranslationKey>

/** The key of one permission of one plugin in the dialog's checks. */
const checkKey = (plugin: string, key: string) => `${plugin}\u0000${key}`

/** The permissions of a step that need consent now. */
function pending(step: PluginPlanStep) {
  return (step.permissions ?? []).filter((p) => p.status !== "granted")
}

export interface InstallPlanRequest {
  /** The plugin's display name. */
  name: string
  plan: PluginInstallPlan
  /** Optional integrations currently selected. */
  optional: string[]
  /** Whether plugins can run here: the plan then enables everything. */
  activate: boolean
  /** Plans again with another selection of optional integrations. */
  replan: (optional: string[]) => Promise<PluginInstallPlan>
  /** Installs the confirmed plan with each plugin's own consent. */
  confirm: (
    plan: PluginInstallPlan,
    optional: string[],
    consent: Record<string, string[]>,
  ) => Promise<void>
}

/**
 * The whole install plan before anything is installed: every plugin and
 * built-in module it needs, in install order, each with its own
 * permissions to consent to. A dependency never gets the permissions
 * granted to the plugin that needs it. Nothing happens until Install.
 */
export function InstallPlanDialog({
  request,
  onClose,
}: {
  request: InstallPlanRequest
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [plan, setPlan] = useState(request.plan)
  const [optional, setOptional] = useState<string[]>(request.optional)
  const [checked, setChecked] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [planning, setPlanning] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const required = plan.steps.flatMap((s) => pending(s).map((p) => checkKey(s.id, p.key)))
  const missing = required.filter((k) => !checked.has(k)).length
  const blockers = plan.steps.filter((s) => s.blocker)
  const extra = plan.steps.filter(
    (s) => !s.requested && s.action !== "keep" && s.action !== "builtin",
  )
  const root = plan.steps.find((s) => s.requested)

  const toggle = (key: string, value: boolean) => {
    setChecked((previous) => {
      const next = new Set(previous)
      if (value) next.add(key)
      else next.delete(key)
      return next
    })
  }

  const choose = (id: string, value: boolean) => {
    const next = value ? [...optional, id] : optional.filter((o) => o !== id)
    setPlanning(true)
    setError(null)
    request.replan(next).then(
      (fresh) => {
        setPlan(fresh)
        setOptional(next)
        setPlanning(false)
      },
      (e: unknown) => {
        setError(pluginErrorSentence(e))
        setPlanning(false)
      },
    )
  }

  const submit = () => {
    const consent: Record<string, string[]> = {}
    for (const step of plan.steps) {
      const keys = pending(step)
        .map((p) => p.key)
        .filter((k) => checked.has(checkKey(step.id, k)))
      if (keys.length > 0) consent[step.id] = keys
    }
    setBusy(true)
    setError(null)
    request.confirm(plan, optional, consent).then(onClose, (e: unknown) => {
      setError(pluginErrorSentence(e))
      setBusy(false)
    })
  }

  return (
    <ModalShell
      presentation="dialog"
      size="large"
      icon={ShieldCheck}
      title={t("permissions:review.title.install", { name: request.name, version: plan.version })}
      description={
        extra.length === 0
          ? t("plugins:plan.intro")
          : t("plugins:plan.introMore", { count: extra.length })
      }
      error={error}
      status={
        <span aria-live="polite">
          {planning ? (
            <span className="flex items-center gap-1">
              <Loader2 className="size-3 animate-spin" aria-hidden /> {t("plugins:plan.planning")}
            </span>
          ) : required.length === 0 ? (
            t("plugins:plan.noConsent")
          ) : missing === 0 ? (
            t("permissions:review.status.consented")
          ) : (
            t("permissions:review.status.missing", { missing, total: required.length })
          )}
        </span>
      }
      onClose={() => {
        if (!busy) onClose()
      }}
      onSubmit={submit}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose, disabled: busy }}
      primary={{
        label:
          extra.length > 0
            ? t("plugins:plan.installCount", { number: extra.length + 1 })
            : t("permissions:review.confirm.install"),
        loadingLabel: t("plugins:plan.installing"),
        loading: busy,
        disabled: planning || missing > 0 || blockers.length > 0,
      }}
    >
      <p className="flex items-start gap-2 text-xs text-muted-foreground">
        <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
        <span>{t("plugins:plan.checked")}</span>
      </p>

      {required.length > 1 && (
        <div className="flex justify-end">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={busy || planning || missing === 0}
            onClick={() => {
              setChecked(new Set([...checked, ...required]))
            }}
          >
            {t("permissions:review.allowAll", { number: required.length })}
          </Button>
        </div>
      )}
      <ol className="grid divide-y border-y" aria-label={t("plugins:plan.label")}>
        {plan.steps.map((step) => (
          <li key={step.id} className="grid gap-1.5 py-2.5">
            <span className="flex flex-wrap items-center gap-2 text-sm">
              <span className="font-medium">{step.name}</span>
              <span className="font-mono text-xs text-muted-foreground">{step.id}</span>
              <span className="text-xs text-muted-foreground">
                {step.fromVersion ? `${step.fromVersion} → ${step.version}` : step.version}
              </span>
              {step.builtin && <Badge variant="outline">{t("plugins:dependencies.builtIn")}</Badge>}
              {step.optional && (
                <Badge variant="outline">{t("plugins:dependencies.optionalBadge")}</Badge>
              )}
              <Badge variant={step.requested ? "secondary" : "outline"} className="ml-auto">
                {step.requested ? t("plugins:plan.requested") : t(ACTION_LABELS[step.action])}
              </Badge>
            </span>
            {step.publisher && (
              <PublisherLine publisher={step.publisher.namespace} info={step.publisher} />
            )}
            {(step.requiredBy ?? []).length > 0 && (
              <span className="text-xs text-muted-foreground">
                {t("plugins:plan.neededBecause", { reasons: (step.requiredBy ?? []).join("; ") })}
              </span>
            )}
            {step.blocker && <span className="text-xs font-medium">{step.blocker}</span>}
            {(step.permissions ?? []).length > 0 && (
              <PermissionList
                label={t("marketplace:installed.permissionsOf", { name: step.name })}
                grants={(step.permissions ?? []).map(grantOfInfo)}
                renderControl={(grant) => {
                  const info = (step.permissions ?? []).find((p) => p.key === grant.key)
                  if (info?.status === "granted") {
                    return (
                      <Badge variant="secondary">
                        <Check aria-hidden /> {t("permissions:review.alreadyGranted")}
                      </Badge>
                    )
                  }
                  const key = checkKey(step.id, grant.key)
                  const id = `plan-consent-${step.id}-${grant.key}`
                  return (
                    <div className="flex shrink-0 items-center gap-2">
                      <Checkbox
                        id={id}
                        checked={checked.has(key)}
                        disabled={busy || planning}
                        aria-label={t("plugins:plan.allowNamed", {
                          name: step.name,
                          title: grant.title,
                        })}
                        onCheckedChange={(value) => {
                          toggle(key, value === true)
                        }}
                      />
                      <label htmlFor={id} className="text-xs">
                        {t("permissions:review.allow")}
                      </label>
                    </div>
                  )
                }}
              />
            )}
          </li>
        ))}
      </ol>

      {(plan.optional ?? []).length > 0 || optional.length > 0 ? (
        <section className="grid gap-1.5" aria-labelledby="plan-optional">
          <h4 id="plan-optional" className="text-xs font-medium text-muted-foreground uppercase">
            {t("plugins:dependencies.optional")}
          </h4>
          <ul className="grid gap-1.5" aria-label={t("plugins:dependencies.optional")}>
            {[
              ...optional.map((id) => ({ id, selected: true, info: undefined })),
              ...(plan.optional ?? []).map((o) => ({
                id: o.dependency.id,
                selected: false,
                info: o.dependency,
              })),
            ].map(({ id, selected, info }) => (
              <li key={id} className="flex items-start gap-2 text-sm">
                <Checkbox
                  id={`plan-optional-${id}`}
                  checked={selected}
                  disabled={busy || planning}
                  onCheckedChange={(value) => {
                    choose(id, value === true)
                  }}
                />
                <label htmlFor={`plan-optional-${id}`} className="grid gap-0.5">
                  <span>
                    <Trans
                      i18nKey="plugins:plan.alsoInstall"
                      values={{
                        name: info?.name ?? plan.steps.find((s) => s.id === id)?.name ?? id,
                        id,
                      }}
                      components={{
                        name: <span className="font-medium" />,
                        id: <span className="font-mono text-xs text-muted-foreground" />,
                      }}
                    />
                  </span>
                  {info?.justification && (
                    <span className="text-xs text-muted-foreground">{info.justification}</span>
                  )}
                  {info && (info.actions ?? []).length > 0 && (
                    <span className="text-xs text-muted-foreground">
                      <Trans
                        i18nKey="plugins:plan.withoutIt"
                        values={{ actions: (info.actions ?? []).join(", ") }}
                        components={{ code: <span className="font-mono" /> }}
                      />
                    </span>
                  )}
                </label>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      {blockers.length > 0 && (
        <Notice title={t("plugins:plan.blocked")}>
          <ul className="grid gap-0.5">
            {blockers.map((s) => (
              <li key={s.id}>{s.blocker}</li>
            ))}
          </ul>
        </Notice>
      )}
      {root && !request.activate && (
        <p className="text-xs text-muted-foreground">{t("plugins:plan.installedOff")}</p>
      )}
    </ModalShell>
  )
}

export interface RemovalRequest {
  name: string
  plan: PluginRemovalPlan
  confirm: (withDependents: boolean, orphans: string[]) => Promise<void>
}

/**
 * What disabling or uninstalling a plugin affects, before it happens: the
 * plugins that need it (turned off with it, never uninstalled), the
 * optional actions that stop, and the plugins installed only for it,
 * which are removed only if checked.
 */
export function RemovalDialog({
  request,
  onClose,
}: {
  request: RemovalRequest
  onClose: () => void
}) {
  const { plan } = request
  const [orphans, setOrphans] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { t } = useTranslation()
  const disables = plan.disables ?? []
  const names = new Map(
    [...(plan.requiredBy ?? []), ...(plan.usedBy ?? [])].map((d) => [d.id, d.name]),
  )

  const submit = () => {
    setBusy(true)
    setError(null)
    request.confirm(disables.length > 0, [...orphans]).then(onClose, (e: unknown) => {
      setError(pluginErrorSentence(e))
      setBusy(false)
    })
  }

  return (
    <ModalShell
      presentation="dialog"
      size="medium"
      // Uninstalling deletes data; turning off does not.
      icon={plan.uninstall ? TriangleAlert : undefined}
      tone={plan.uninstall ? "destructive" : "default"}
      title={
        plan.uninstall
          ? t("marketplace:uninstall.title", { name: request.name })
          : t("plugins:removal.turnOffTitle", { name: request.name })
      }
      description={
        disables.length > 0 ? t("plugins:removal.dependents") : t("plugins:removal.review")
      }
      error={error}
      onClose={() => {
        if (!busy) onClose()
      }}
      onSubmit={submit}
      secondary={{ label: t("common:actions.cancel"), onClick: onClose, disabled: busy }}
      primary={{
        label: plan.uninstall
          ? disables.length > 0
            ? t("plugins:removal.uninstallCount", { number: disables.length + 1 })
            : t("marketplace:uninstall.button")
          : disables.length > 0
            ? t("plugins:removal.turnOffCount", { number: disables.length + 1 })
            : t("plugins:removal.turnOff"),
        loadingLabel: t("common:status.working"),
        loading: busy,
        variant: plan.uninstall ? "destructive" : "default",
      }}
    >
      {disables.length > 0 && (
        <section className="grid gap-1" aria-labelledby="removal-disables">
          <h4 id="removal-disables" className="text-xs font-medium text-muted-foreground uppercase">
            {t("plugins:removal.alsoOff")}
          </h4>
          <ul className="grid divide-y border-y text-sm" aria-label={t("plugins:removal.alsoOff")}>
            {disables.map((id) => (
              <li key={id} className="flex flex-wrap items-center gap-2 py-1.5">
                <span className="font-medium">{names.get(id) ?? id}</span>
                <span className="font-mono text-xs text-muted-foreground">{id}</span>
              </li>
            ))}
          </ul>
        </section>
      )}
      {(plan.usedBy ?? []).length > 0 && (
        <section className="grid gap-1" aria-labelledby="removal-used-by">
          <h4 id="removal-used-by" className="text-xs font-medium text-muted-foreground uppercase">
            {t("plugins:removal.unavailable")}
          </h4>
          <ul
            className="grid divide-y border-y text-sm"
            aria-label={t("plugins:removal.unavailable")}
          >
            {(plan.usedBy ?? []).map((d) => (
              <li key={d.id} className="grid gap-0.5 py-1.5">
                <span className="font-medium">{d.name}</span>
                <span className="font-mono text-xs text-muted-foreground">
                  {(d.actions ?? []).join(", ") || t("plugins:removal.optionalIntegration")}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
      {(plan.orphans ?? []).length > 0 && (
        <section className="grid gap-1" aria-labelledby="removal-orphans">
          <h4 id="removal-orphans" className="text-xs font-medium text-muted-foreground uppercase">
            {t("plugins:removal.orphans")}
          </h4>
          <ul className="grid gap-1.5" aria-label={t("plugins:removal.orphans")}>
            {(plan.orphans ?? []).map((o) => (
              <li key={o.id} className="flex items-center gap-2 text-sm">
                <Checkbox
                  id={`removal-orphan-${o.id}`}
                  checked={orphans.has(o.id)}
                  disabled={busy}
                  onCheckedChange={(value) => {
                    setOrphans((previous) => {
                      const next = new Set(previous)
                      if (value === true) next.add(o.id)
                      else next.delete(o.id)
                      return next
                    })
                  }}
                />
                <label htmlFor={`removal-orphan-${o.id}`}>
                  <Trans
                    i18nKey="plugins:removal.alsoUninstall"
                    values={{ name: o.name, id: o.id }}
                    components={{
                      name: <span className="font-medium" />,
                      id: <span className="font-mono text-xs text-muted-foreground" />,
                    }}
                  />
                </label>
              </li>
            ))}
          </ul>
        </section>
      )}
    </ModalShell>
  )
}
