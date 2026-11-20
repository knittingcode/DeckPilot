import { useCallback, useEffect, useRef, useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/dialogs"
import { Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { i18n, useTranslation } from "@/i18n"
import { useAppContext } from "@/lib/app-context"

import {
  pluginErrorText,
  usePluginClientOverride,
  type PluginClient,
  type PluginErrorText,
} from "./api"
import { createTauriPluginClient } from "./client"
import { errorSentence } from "./dialog-error"
import {
  ExperimentalNotice,
  Notice,
  ProgressList,
  RegistryNotice,
  RuntimeNotice,
  type Progress,
} from "./components"
import { PluginDetailsView } from "./details"
import { MigrationNotice } from "./migration"
import { MigrationConsentDialog } from "./migration-dialog"
import { DiscoverView } from "./discover"
import { InstalledView } from "./installed"
import {
  availabilityOf,
  availabilityOfError,
  installBlock,
  loadSnapshot,
  updateCandidates,
  type UpdateCandidate,
} from "./model"
import { grantOfInfo, grantsOf } from "./permissions"
import { PermissionReviewDialog, type ReviewRequest } from "./review-dialog"
import {
  InstallPlanDialog,
  RemovalDialog,
  type InstallPlanRequest,
  type RemovalRequest,
} from "./dependencies"
import { removalNeedsReview } from "./dependency-text"
import type {
  ActionParamValue,
  AvailablePlugin,
  AvailableVersion,
  MarketplaceSnapshot,
  ModuleMigration,
  PermissionChanges,
  PluginActionOutcome,
  PluginDetails,
  PluginRetention,
  RuntimeAvailability,
} from "./types"
import { DEPENDENCIES_FEATURE, RESTORE_BUNDLED_FEATURE } from "./types"
import { UpdatesView } from "./updates"

type Tab = "discover" | "installed" | "updates"

type Load =
  | { status: "loading" }
  | { status: "unavailable"; availability: RuntimeAvailability }
  | { status: "error"; error: unknown }
  | {
      status: "ready"
      availability: RuntimeAvailability
      snapshot: MarketplaceSnapshot
      /** Built-in modules moving to plugins; empty from agents without them. */
      migrations: ModuleMigration[]
    }

/** What the screens can ask the marketplace to do. */
export interface MarketplaceActions {
  /** Id of the plugin with a request in flight, or "registry". */
  busy: string | null
  /** Why installs and updates are not possible right now. */
  blocked: string | null
  /** Plugins can be enabled and run here (not on Windows). */
  canRun: boolean
  /** The client, for the requests a dialog makes itself (settings, credentials). */
  client: PluginClient
  openDetails: (id: string) => void
  install: (plugin: AvailablePlugin, version: AvailableVersion) => void
  installFile: () => void
  update: (candidate: UpdateCandidate) => void
  setEnabled: (item: PluginDetails, enabled: boolean) => void
  setGrant: (item: PluginDetails, key: string, granted: boolean) => void
  /** Uninstalls; `keep` keeps settings and/or credentials for a reinstall. */
  uninstall: (item: PluginDetails, keep?: PluginRetention) => void
  /** The agent can restore DeckPilot's own plugins (feature `restoreBundled`). */
  canRestore: boolean
  /**
   * Installs a removed bundled plugin again from DeckPilot's own copy, or
   * (`reinstall`) replaces a broken bundled copy with it.
   */
  restore: (id: string, name: string, reinstall: boolean) => void
  rollback: (item: PluginDetails) => void
  invoke: (
    item: PluginDetails,
    action: string,
    params: Record<string, ActionParamValue>,
  ) => Promise<PluginActionOutcome>
  refresh: () => void
  /** Reads everything again (without re-reading the registry). */
  reload: () => void
}

async function loadAll(client: PluginClient, refresh = false): Promise<Load> {
  let runtime
  try {
    runtime = await client.getRuntime()
  } catch (error) {
    const availability = availabilityOfError(error)
    return availability ? { status: "unavailable", availability } : { status: "error", error }
  }
  const availability = availabilityOf(runtime)
  if (!runtime.available) return { status: "unavailable", availability }
  try {
    // An agent before module migrations refuses the request: none then.
    const migrations = client.listModuleMigrations().catch(() => [])
    const snapshot = await loadSnapshot(client, runtime, refresh)
    return { status: "ready", availability, snapshot, migrations: await migrations }
  } catch (error) {
    return { status: "error", error }
  }
}

/** A refusal as a toast: what it means, the agent's words and what to do. */
function toastError({ title, message, hint }: PluginErrorText) {
  const description = [message === title ? null : message, hint].filter(Boolean).join(" ")
  toast.error(title, description ? { description } : undefined)
}

function updateItems(changes: PermissionChanges): ReviewRequest["items"] {
  return [
    ...changes.added.map((info) => ({ grant: grantOfInfo(info), state: "new" as const })),
    ...changes.broadened.map((info) => ({
      grant: grantOfInfo(info),
      state: "broadened" as const,
    })),
  ]
}

/**
 * The experimental plugin marketplace: the catalog (Discover, search,
 * details, install with permission review) and the installed plugins
 * (enable, permissions, configure, updates, uninstall). Shown as the main
 * window's Marketplace and, starting at the installed plugins, as Settings
 * › Extensions › Plugins. Only reachable in builds with
 * VITE_DECKPILOT_PLUGINS=1. Everything comes from the agent; the page says
 * why when the agent cannot answer plugin requests.
 */
export function PluginsSettings({ initialTab = "discover" }: { initialTab?: Tab } = {}) {
  const { t } = useTranslation()
  const override = usePluginClientOverride()
  const [client] = useState<PluginClient>(() => override ?? createTauriPluginClient())
  const { agent } = useAppContext()
  const [load, setLoad] = useState<Load>({ status: "loading" })
  const [tab, setTab] = useState<Tab>(initialTab)
  const [selected, setSelected] = useState<string | null>(null)
  const [review, setReview] = useState<ReviewRequest | null>(null)
  const [planReview, setPlanReview] = useState<InstallPlanRequest | null>(null)
  const [removal, setRemoval] = useState<RemovalRequest | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [progress, setProgress] = useState<Map<string, Progress>>(new Map())
  /** A package file waiting for the user to confirm its installation. */
  const [packageFile, setPackageFile] = useState<string | null>(null)
  /** A module migration whose consent dialog is open. */
  const [consentFor, setConsentFor] = useState<ModuleMigration | null>(null)
  const returnFocus = useRef<HTMLElement | null>(null)
  const generation = useRef(0)

  /** Reads everything again; the page keeps showing the old data meanwhile. */
  const reload = useCallback(
    (refresh = false) => {
      const current = ++generation.current
      return loadAll(client, refresh).then((next) => {
        if (current === generation.current) setLoad(next)
      })
    },
    [client],
  )

  // The agent started, stopped or was replaced: ask again.
  const agentKey = agent
    ? `${String(agent.running)}:${agent.status?.version ?? ""}:${String(agent.protocol)}`
    : "unknown"
  useEffect(() => {
    void reload()
  }, [reload, agentKey])

  useEffect(
    () =>
      client.subscribe((event) => {
        if (event.type === "progress") {
          setProgress((previous) => {
            const next = new Map(previous)
            if (event.stage === "finished" || event.stage === "failed") next.delete(event.plugin)
            else next.set(event.plugin, event)
            return next
          })
        } else {
          void reload()
        }
      }),
    [client, reload],
  )

  const retry = () => {
    setLoad({ status: "loading" })
    void reload()
  }

  const header = (
    <>
      <ExperimentalNotice />
      {progress.size > 0 && <ProgressList progress={[...progress.values()]} />}
    </>
  )

  if (load.status === "loading") {
    return (
      <>
        {header}
        <Loading />
      </>
    )
  }
  if (load.status === "unavailable") {
    return (
      <>
        {header}
        <RuntimeNotice availability={load.availability} busy={false} onRetry={retry} />
      </>
    )
  }
  if (load.status === "error") {
    // Read at render time, so the words follow the language on screen.
    const failure = pluginErrorText(load.error)
    return (
      <>
        {header}
        <Notice tone="destructive" title={t("marketplace:loadFailed")}>
          <p>{errorSentence(failure, false)}</p>
          {failure.hint && <p>{failure.hint}</p>}
          <div>
            <Button variant="outline" size="sm" onClick={retry}>
              {t("common:actions.tryAgain")}
            </Button>
          </div>
        </Notice>
      </>
    )
  }

  const { snapshot, availability, migrations } = load
  const blocked = installBlock(snapshot)
  // Dependency plans (plugin API 1.4): agents without them install one
  // plugin at a time, as before.
  const plans = (snapshot.runtime.features ?? []).includes(DEPENDENCIES_FEATURE)
  const canRun = snapshot.runtime.executionSupported
  const canRestore = (snapshot.runtime.features ?? []).includes(RESTORE_BUNDLED_FEATURE)
  const updates = updateCandidates(snapshot)

  /** Runs one request, then reads everything again. Rejections propagate. */
  const run = async (id: string, request: () => Promise<unknown>, success: string) => {
    setBusy(id)
    try {
      await request()
      toast.success(success)
    } finally {
      await reload()
      setBusy(null)
    }
  }
  /** Like `run`, but reports failures as a toast instead of to a dialog. */
  const runReported = (id: string, request: () => Promise<unknown>, success: string) => {
    run(id, request, success).catch((error: unknown) => {
      toastError(pluginErrorText(error))
    })
  }

  const openReview = (request: ReviewRequest) => {
    returnFocus.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    setReview(request)
  }

  /** The consent dialog for enabling `item` with every declared permission. */
  const reviewEnable = (item: PluginDetails) => {
    const id = item.summary.id
    openReview({
      mode: "enable",
      pluginId: id,
      name: item.summary.name,
      publisher: item.summary.publisher,
      version: item.summary.version,
      items: item.permissions.map((info) => ({
        grant: grantOfInfo(info),
        state: info.status === "granted" ? "granted" : "new",
      })),
      confirm: (consent) =>
        run(
          id,
          () => client.enablePlugin(id, consent),
          i18n.t("marketplace:toast.enabled", { name: item.summary.name }),
        ),
    })
  }

  /** Installs the confirmed package file, then shows its permissions. */
  const installPackage = (path: string) => {
    setBusy("package")
    client
      .installPlugin({ type: "file", path }, [])
      .then(async (details) => {
        toast.success(
          i18n.t("marketplace:toast.installedFromFile", {
            name: details.summary.name,
            version: details.summary.version,
          }),
        )
        await reload()
        // Its permissions are known now that the agent verified it:
        // nothing is granted until the user consents.
        if (canRun) reviewEnable(details)
      })
      .catch((error: unknown) => {
        toastError(pluginErrorText(error))
      })
      .finally(() => {
        setBusy(null)
      })
  }

  const reviewUpdate = (item: PluginDetails, version: string, changes: PermissionChanges) => {
    const id = item.summary.id
    openReview({
      mode: "update",
      pluginId: id,
      name: item.summary.name,
      publisher: item.summary.publisher,
      version,
      items: updateItems(changes),
      removed: changes.removed,
      confirm: (consent) =>
        run(
          id,
          () => client.approveUpdate(id, consent),
          i18n.t("marketplace:toast.updated", { name: item.summary.name, version }),
        ),
      decline: () =>
        run(
          id,
          () => client.declineUpdate(id),
          i18n.t("marketplace:toast.updateDeclined", { name: item.summary.name }),
        ),
    })
  }

  /**
   * The full install plan (dependencies, built-in modules, every plugin's
   * permissions), resolved and verified by the agent before the dialog
   * opens; nothing is installed until the user confirms it.
   */
  const reviewPlan = (plugin: AvailablePlugin, version: AvailableVersion) => {
    const replan = (optional: string[]) => client.planInstall(plugin.id, version.version, optional)
    setBusy(plugin.id)
    replan([])
      .then((plan) => {
        returnFocus.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null
        setPlanReview({
          name: plugin.name,
          plan,
          optional: [],
          activate: canRun,
          replan,
          confirm: (confirmed, optional, consent) =>
            run(
              plugin.id,
              () => client.installPlan(confirmed, optional, consent, canRun),
              confirmed.steps.length > 1
                ? i18n.t("marketplace:toast.installedWithMore", {
                    name: plugin.name,
                    count: confirmed.steps.length - 1,
                  })
                : i18n.t("marketplace:toast.installed", { name: plugin.name }),
            ),
        })
      })
      .catch((error: unknown) => {
        toastError(pluginErrorText(error))
      })
      .finally(() => {
        setBusy(null)
      })
  }

  /**
   * Disabling or uninstalling with what it affects: plugins that need it
   * are turned off with it (after the user saw them), orphaned
   * dependencies are removed only if checked. Nothing to show: done at once.
   */
  const reviewRemoval = (item: PluginDetails, uninstall: boolean, keep?: PluginRetention) => {
    const id = item.summary.id
    const name = item.summary.name
    const remove = (withDependents: boolean, orphans: string[]) =>
      run(
        id,
        () => client.removePlugin(id, uninstall, keep, withDependents, orphans),
        uninstall
          ? i18n.t("marketplace:toast.uninstalled", { name })
          : i18n.t("marketplace:toast.disabled", { name }),
      )
    client
      .planRemoval(id, uninstall)
      .then((plan) => {
        if (!removalNeedsReview(plan)) return remove(false, [])
        returnFocus.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null
        setRemoval({ name, plan, confirm: remove })
        return undefined
      })
      .catch((error: unknown) => {
        toastError(pluginErrorText(error))
      })
  }

  const actions: MarketplaceActions = {
    busy,
    blocked,
    canRun,
    client,
    openDetails: (id) => {
      setSelected(id)
    },
    install: (plugin, version) => {
      if (plans) {
        reviewPlan(plugin, version)
        return
      }
      openReview({
        mode: "install",
        pluginId: plugin.id,
        name: plugin.name,
        publisher: plugin.publisher,
        publisherInfo: plugin.publisherInfo,
        version: version.version,
        items: grantsOf(version.permissions).map((grant) => ({ grant, state: "new" })),
        confirm: (consent) =>
          run(
            plugin.id,
            () =>
              client.installPlugin(
                { type: "registry", plugin: plugin.id, version: version.version },
                consent,
              ),
            i18n.t("marketplace:toast.installed", { name: plugin.name }),
          ),
      })
    },
    installFile: () => {
      client.pickPackage().then(
        (path) => {
          if (path) setPackageFile(path)
        },
        (error: unknown) => {
          toastError(pluginErrorText(error))
        },
      )
    },
    update: (candidate) => {
      const item = candidate.plugin
      const id = item.summary.id
      if (candidate.kind === "staged") {
        reviewUpdate(item, candidate.pending.version, candidate.pending.changes)
        return
      }
      setBusy(id)
      client
        .updatePlugin(id, candidate.version, [])
        .then(async (result) => {
          if (result.outcome.type === "applied") {
            toast.success(
              i18n.t("marketplace:toast.updated", {
                name: item.summary.name,
                version: result.outcome.version,
              }),
            )
          } else {
            // Staged until the user decides (approve or decline).
            reviewUpdate(item, result.outcome.version, result.outcome.changes)
          }
          await reload()
        })
        .catch((error: unknown) => {
          toastError(pluginErrorText(error))
        })
        .finally(() => {
          setBusy(null)
        })
    },
    setEnabled: (item, enabled) => {
      const id = item.summary.id
      if (!enabled) {
        if (plans) {
          reviewRemoval(item, false, undefined)
          return
        }
        runReported(
          id,
          () => client.disablePlugin(id),
          i18n.t("marketplace:toast.disabled", { name: item.summary.name }),
        )
        return
      }
      reviewEnable(item)
    },
    setGrant: (item, key, granted) => {
      const id = item.summary.id
      runReported(
        id,
        () => (granted ? client.grantPermissions(id, [key]) : client.revokePermission(id, key)),
        granted
          ? i18n.t("marketplace:toast.permissionGranted")
          : i18n.t("marketplace:toast.permissionRevoked"),
      )
    },
    uninstall: (item, keep) => {
      const id = item.summary.id
      if (plans) {
        reviewRemoval(item, true, keep)
        return
      }
      const name = item.summary.name
      const kept =
        keep?.settings && keep.credentials
          ? i18n.t("marketplace:toast.uninstalledKeptBoth", { name })
          : keep?.settings
            ? i18n.t("marketplace:toast.uninstalledKeptSettings", { name })
            : keep?.credentials
              ? i18n.t("marketplace:toast.uninstalledKeptCredentials", { name })
              : null
      runReported(
        id,
        () =>
          keep?.settings || keep?.credentials
            ? client.uninstallPlugin(id, keep)
            : client.uninstallPlugin(id),
        kept ?? i18n.t("marketplace:toast.uninstalled", { name }),
      )
    },
    canRestore,
    restore: (id, name, reinstall) => {
      setBusy(id)
      client
        .restoreBundledPlugin(id)
        .then(async (details) => {
          toast.success(
            reinstall
              ? i18n.t("marketplace:toast.reinstalled", { name: details.summary.name || name })
              : i18n.t("marketplace:toast.restored", { name: details.summary.name || name }),
          )
          await reload()
          // Permissions beyond what DeckPilot grants by itself wait for the
          // user, as after any install.
          if (
            canRun &&
            details.summary.state === "disabled" &&
            details.permissions.some((p) => p.status !== "granted")
          ) {
            reviewEnable(details)
          }
        })
        .catch((error: unknown) => {
          toastError(pluginErrorText(error))
        })
        .finally(() => {
          setBusy(null)
        })
    },
    rollback: (item) => {
      const id = item.summary.id
      runReported(
        id,
        () => client.rollbackPlugin(id),
        item.summary.previousVersion
          ? i18n.t("marketplace:toast.rolledBack", {
              name: item.summary.name,
              version: item.summary.previousVersion,
            })
          : i18n.t("marketplace:toast.rolledBackPrevious", { name: item.summary.name }),
      )
    },
    // Read again afterwards: a failure changes the plugin's health.
    invoke: (item, action, params) =>
      client.invokeAction(item.summary.id, action, params).finally(() => {
        void reload()
      }),
    refresh: () => {
      setBusy("registry")
      void reload(true).finally(() => {
        setBusy(null)
      })
    },
    reload: () => {
      void reload()
    },
  }

  const closeReview = () => {
    setReview(null)
    returnFocus.current?.focus()
  }

  const selectedPlugin = selected ? snapshot.available.find((p) => p.id === selected) : undefined

  return (
    <>
      {header}
      <RuntimeNotice availability={availability} busy={busy !== null} onRetry={retry} />
      <RegistryNotice
        registry={snapshot.registry}
        error={snapshot.registryError}
        busy={busy !== null}
        onRetry={actions.refresh}
      />
      <MigrationNotice
        migrations={migrations}
        busy={busy}
        onReview={(migration) => {
          returnFocus.current =
            document.activeElement instanceof HTMLElement ? document.activeElement : null
          setConsentFor(migration)
        }}
        onRetry={(migration) => {
          runReported(
            migration.module,
            () => client.retryModuleMigration(migration.module),
            i18n.t("marketplace:toast.migrationRetry", { module: migration.moduleName }),
          )
        }}
      />
      {selectedPlugin ? (
        <PluginDetailsView
          key={selectedPlugin.id}
          plugin={selectedPlugin}
          snapshot={snapshot}
          updates={updates}
          actions={actions}
          onBack={() => {
            setSelected(null)
          }}
        />
      ) : (
        <Tabs
          value={tab}
          onValueChange={(value) => {
            setTab(value as Tab)
          }}
        >
          <TabsList aria-label={t("marketplace:tabs.label")}>
            <TabsTrigger value="discover">{t("marketplace:tabs.discover")}</TabsTrigger>
            <TabsTrigger value="installed">
              {t("marketplace:tabs.installed", { number: snapshot.installed.length })}
            </TabsTrigger>
            <TabsTrigger value="updates">
              {t("marketplace:tabs.updates", { number: updates.length })}
            </TabsTrigger>
          </TabsList>
          <TabsContent value="discover" className="grid gap-4 pt-2">
            <DiscoverView snapshot={snapshot} updates={updates} actions={actions} />
          </TabsContent>
          <TabsContent value="installed" className="grid gap-4 pt-2">
            <InstalledView
              snapshot={snapshot}
              updates={updates}
              actions={actions}
              onBrowse={() => {
                setTab("discover")
              }}
            />
          </TabsContent>
          <TabsContent value="updates" className="grid gap-4 pt-2">
            <UpdatesView updates={updates} actions={actions} />
          </TabsContent>
        </Tabs>
      )}
      {review && <PermissionReviewDialog request={review} onClose={closeReview} />}
      {planReview && (
        <InstallPlanDialog
          request={planReview}
          onClose={() => {
            setPlanReview(null)
            returnFocus.current?.focus()
          }}
        />
      )}
      {removal && (
        <RemovalDialog
          request={removal}
          onClose={() => {
            setRemoval(null)
            returnFocus.current?.focus()
          }}
        />
      )}
      {consentFor && (
        <MigrationConsentDialog
          migration={consentFor}
          onDecide={(approve, decline) =>
            run(
              consentFor.module,
              () => client.decideModuleMigration(consentFor.module, approve, decline),
              decline.length > 0
                ? i18n.t("marketplace:toast.migrationDeclined", { module: consentFor.moduleName })
                : i18n.t("marketplace:toast.migrationConsent", { module: consentFor.moduleName }),
            )
          }
          onClose={() => {
            setConsentFor(null)
            returnFocus.current?.focus()
          }}
        />
      )}
      <ConfirmDialog
        open={packageFile !== null}
        onOpenChange={(open) => {
          if (!open) setPackageFile(null)
        }}
        destructive={false}
        title={t("marketplace:package.title")}
        description={t("marketplace:package.description")}
        confirmLabel={t("marketplace:package.confirm")}
        onConfirm={() => {
          if (packageFile) installPackage(packageFile)
          setPackageFile(null)
        }}
      >
        <p className="font-mono text-xs break-all select-text">{packageFile}</p>
      </ConfirmDialog>
    </>
  )
}

/** Settings › Extensions › Plugins: the marketplace, at the installed plugins. */
export function InstalledPluginsSettings() {
  return <PluginsSettings initialTab="installed" />
}
