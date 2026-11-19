// Plugin dependencies in the marketplace (plugin API 1.4;
// docs/marketplace/dependencies.md): what a plugin needs, the full install
// plan with every plugin's own permissions, and what disabling or
// uninstalling a plugin affects. Compact rows in the settings layout, no
// cards.

import { Check, Loader2, ShieldAlert } from "lucide-react"
import { useState } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

import { pluginErrorText, type PluginErrorText } from "./api"
import { Notice, PermissionList, PublisherLine } from "./components"
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
  if (dependencies.length === 0) return null
  const required = dependencies.filter((d) => !d.optional)
  const optional = dependencies.filter((d) => d.optional)
  return (
    <section aria-labelledby="plugin-detail-dependencies" className="grid gap-2">
      <h4 id="plugin-detail-dependencies" className="text-sm font-medium">
        {title}
      </h4>
      {required.length > 0 && <DependencyRows label="Requires" items={required} />}
      {optional.length > 0 && (
        <>
          <p className="text-xs text-muted-foreground">
            Optional integrations are never installed by themselves; without one, only the actions
            that use it are unavailable.
          </p>
          <DependencyRows label="Optional integrations" items={optional} />
        </>
      )}
    </section>
  )
}

function DependencyRows({ label, items }: { label: string; items: PluginDependencyInfo[] }) {
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
            {d.builtin && <Badge variant="outline">Built in</Badge>}
            {d.optional && <Badge variant="outline">Optional</Badge>}
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
              Used by: <span className="font-mono">{(d.actions ?? []).join(", ")}</span>
            </span>
          )}
        </li>
      ))}
    </ul>
  )
}

/** The installed plugins that declare a dependency on this one. */
export function DependentsSection({ dependents }: { dependents: PluginDependentInfo[] }) {
  if (dependents.length === 0) return null
  return (
    <section aria-labelledby="plugin-detail-dependents" className="grid gap-2">
      <h4 id="plugin-detail-dependents" className="text-sm font-medium">
        Used by
      </h4>
      <ul className="grid divide-y border-y text-sm" aria-label="Used by">
        {dependents.map((d) => (
          <li key={d.id} className="flex flex-wrap items-center gap-2 py-1.5">
            <span className="font-medium">{d.name}</span>
            <span className="font-mono text-xs text-muted-foreground">{d.id}</span>
            <span className="text-xs text-muted-foreground">
              {d.optional ? "optional" : "requires"} {d.version}
            </span>
            {!d.enabled && <Badge variant="outline">Off</Badge>}
          </li>
        ))}
      </ul>
    </section>
  )
}

const ACTION_LABELS: Record<PluginPlanStep["action"], string> = {
  install: "Install",
  update: "Update",
  enable: "Turn on",
  keep: "Already installed",
  builtin: "Built in",
  enableBuiltin: "Built in, turned off",
}

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
  const [plan, setPlan] = useState(request.plan)
  const [optional, setOptional] = useState<string[]>(request.optional)
  const [checked, setChecked] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [planning, setPlanning] = useState(false)
  const [error, setError] = useState<PluginErrorText | null>(null)

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
        setError(pluginErrorText(e))
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
      setError(pluginErrorText(e))
      setBusy(false)
    })
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose()
      }}
    >
      <DialogContent className="max-h-[90svh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            Install {request.name} {plan.version}?
          </DialogTitle>
          <DialogDescription>
            {extra.length === 0
              ? "Review what this plugin may do. Check each permission to consent to it."
              : `It needs ${String(extra.length)} more ${extra.length === 1 ? "plugin" : "plugins"}, installed first. Each plugin gets only the permissions you grant it here.`}
          </DialogDescription>
        </DialogHeader>
        <p className="flex items-start gap-2 text-xs text-muted-foreground">
          <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
          <span>
            Every package was downloaded and checked against the signed registry; nothing is
            installed before you confirm. If one step fails, what was done is undone.
          </span>
        </p>

        <ol className="grid divide-y border-y" aria-label="Install plan">
          {plan.steps.map((step) => (
            <li key={step.id} className="grid gap-1.5 py-2.5">
              <span className="flex flex-wrap items-center gap-2 text-sm">
                <span className="font-medium">{step.name}</span>
                <span className="font-mono text-xs text-muted-foreground">{step.id}</span>
                <span className="text-xs text-muted-foreground">
                  {step.fromVersion ? `${step.fromVersion} → ${step.version}` : step.version}
                </span>
                {step.builtin && <Badge variant="outline">Built in</Badge>}
                {step.optional && <Badge variant="outline">Optional</Badge>}
                <Badge variant={step.requested ? "secondary" : "outline"} className="ml-auto">
                  {step.requested ? "Requested" : ACTION_LABELS[step.action]}
                </Badge>
              </span>
              {step.publisher && (
                <PublisherLine publisher={step.publisher.namespace} info={step.publisher} />
              )}
              {(step.requiredBy ?? []).length > 0 && (
                <span className="text-xs text-muted-foreground">
                  Needed because {(step.requiredBy ?? []).join("; ")}
                </span>
              )}
              {step.blocker && <span className="text-xs font-medium">{step.blocker}</span>}
              {(step.permissions ?? []).length > 0 && (
                <PermissionList
                  label={`Permissions of ${step.name}`}
                  grants={(step.permissions ?? []).map(grantOfInfo)}
                  renderControl={(grant) => {
                    const info = (step.permissions ?? []).find((p) => p.key === grant.key)
                    if (info?.status === "granted") {
                      return (
                        <Badge variant="secondary">
                          <Check aria-hidden /> Already granted
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
                          aria-label={`Allow ${step.name}: ${grant.title}`}
                          onCheckedChange={(value) => {
                            toggle(key, value === true)
                          }}
                        />
                        <label htmlFor={id} className="text-xs">
                          Allow
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
              Optional integrations
            </h4>
            <ul className="grid gap-1.5" aria-label="Optional integrations">
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
                      Also install{" "}
                      <span className="font-medium">
                        {info?.name ?? plan.steps.find((s) => s.id === id)?.name ?? id}
                      </span>{" "}
                      <span className="font-mono text-xs text-muted-foreground">{id}</span>
                    </span>
                    {info?.justification && (
                      <span className="text-xs text-muted-foreground">{info.justification}</span>
                    )}
                    {info && (info.actions ?? []).length > 0 && (
                      <span className="text-xs text-muted-foreground">
                        Without it these actions are unavailable:{" "}
                        <span className="font-mono">{(info.actions ?? []).join(", ")}</span>
                      </span>
                    )}
                  </label>
                </li>
              ))}
            </ul>
          </section>
        ) : null}

        {blockers.length > 0 && (
          <Notice title="This plan cannot be installed yet">
            <ul className="grid gap-0.5">
              {blockers.map((s) => (
                <li key={s.id}>{s.blocker}</li>
              ))}
            </ul>
          </Notice>
        )}
        {error && (
          <Alert variant="destructive">
            <AlertTitle className="line-clamp-none">{error.title}</AlertTitle>
            <AlertDescription className="grid gap-1">
              <p>{error.message}</p>
              {error.hint && <p>{error.hint}</p>}
            </AlertDescription>
          </Alert>
        )}
        <DialogFooter className="items-center gap-2 sm:justify-between">
          <p className="text-xs text-muted-foreground" aria-live="polite">
            {planning ? (
              <span className="flex items-center gap-1">
                <Loader2 className="size-3 animate-spin" aria-hidden /> Planning again…
              </span>
            ) : required.length === 0 ? (
              "No permission needs your consent."
            ) : missing === 0 ? (
              "You consented to every permission."
            ) : (
              `${String(missing)} of ${String(required.length)} permissions still need your consent.`
            )}
          </p>
          <div className="flex flex-wrap gap-2">
            {required.length > 1 && (
              <Button
                variant="outline"
                disabled={busy || planning || missing === 0}
                onClick={() => {
                  setChecked(new Set([...checked, ...required]))
                }}
              >
                Allow all {required.length}
              </Button>
            )}
            <Button variant="ghost" disabled={busy} onClick={onClose}>
              Cancel
            </Button>
            <Button
              disabled={busy || planning || missing > 0 || blockers.length > 0}
              onClick={submit}
            >
              {busy
                ? "Installing…"
                : extra.length > 0
                  ? `Install ${String(extra.length + 1)}`
                  : "Install"}
            </Button>
          </div>
        </DialogFooter>
        {root && !request.activate && (
          <p className="text-xs text-muted-foreground">
            Plugins cannot run on this computer: they are installed turned off.
          </p>
        )}
      </DialogContent>
    </Dialog>
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
  const [error, setError] = useState<PluginErrorText | null>(null)
  const disables = plan.disables ?? []
  const verb = plan.uninstall ? "Uninstall" : "Turn off"
  const names = new Map(
    [...(plan.requiredBy ?? []), ...(plan.usedBy ?? [])].map((d) => [d.id, d.name]),
  )

  const submit = () => {
    setBusy(true)
    setError(null)
    request.confirm(disables.length > 0, [...orphans]).then(onClose, (e: unknown) => {
      setError(pluginErrorText(e))
      setBusy(false)
    })
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose()
      }}
    >
      <DialogContent className="max-h-[90svh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>
            {verb} {request.name}?
          </DialogTitle>
          <DialogDescription>
            {disables.length > 0
              ? "Other plugins need it. They are turned off with it; nothing else is uninstalled unless you check it below."
              : "Review what this changes."}
          </DialogDescription>
        </DialogHeader>
        {disables.length > 0 && (
          <section className="grid gap-1" aria-labelledby="removal-disables">
            <h4
              id="removal-disables"
              className="text-xs font-medium text-muted-foreground uppercase"
            >
              Also turned off
            </h4>
            <ul className="grid divide-y border-y text-sm" aria-label="Also turned off">
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
            <h4
              id="removal-used-by"
              className="text-xs font-medium text-muted-foreground uppercase"
            >
              Actions that become unavailable
            </h4>
            <ul
              className="grid divide-y border-y text-sm"
              aria-label="Actions that become unavailable"
            >
              {(plan.usedBy ?? []).map((d) => (
                <li key={d.id} className="grid gap-0.5 py-1.5">
                  <span className="font-medium">{d.name}</span>
                  <span className="font-mono text-xs text-muted-foreground">
                    {(d.actions ?? []).join(", ") || "its optional integration"}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        )}
        {(plan.orphans ?? []).length > 0 && (
          <section className="grid gap-1" aria-labelledby="removal-orphans">
            <h4
              id="removal-orphans"
              className="text-xs font-medium text-muted-foreground uppercase"
            >
              Installed only for it
            </h4>
            <ul className="grid gap-1.5" aria-label="Installed only for it">
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
                    Also uninstall <span className="font-medium">{o.name}</span>{" "}
                    <span className="font-mono text-xs text-muted-foreground">{o.id}</span>
                  </label>
                </li>
              ))}
            </ul>
          </section>
        )}
        {error && (
          <Alert variant="destructive">
            <AlertTitle className="line-clamp-none">{error.title}</AlertTitle>
            <AlertDescription className="grid gap-1">
              <p>{error.message}</p>
              {error.hint && <p>{error.hint}</p>}
            </AlertDescription>
          </Alert>
        )}
        <DialogFooter className="gap-2">
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant={plan.uninstall ? "destructive" : "default"}
            disabled={busy}
            onClick={submit}
          >
            {busy
              ? "Working…"
              : disables.length > 0
                ? `${verb} ${String(disables.length + 1)}`
                : verb}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
