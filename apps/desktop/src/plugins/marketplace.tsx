import { useCallback, useEffect, useRef, useState } from "react"
import { toast } from "sonner"

import { Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useAppContext } from "@/lib/app-context"

import { pluginErrorText, usePluginClientOverride, type PluginClient } from "./api"
import { createTauriPluginClient } from "./client"
import {
  ExperimentalNotice,
  Notice,
  ProgressList,
  RegistryNotice,
  RuntimeNotice,
  type Progress,
} from "./components"
import { PluginDetailsView } from "./details"
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
import type {
  ActionParamValue,
  AvailablePlugin,
  AvailableVersion,
  MarketplaceSnapshot,
  PermissionChanges,
  PluginActionOutcome,
  PluginDetails,
  RuntimeAvailability,
} from "./types"
import { UpdatesView } from "./updates"

type Tab = "discover" | "installed" | "updates"

type Load =
  | { status: "loading" }
  | { status: "unavailable"; availability: RuntimeAvailability }
  | { status: "error"; message: string; hint?: string }
  | { status: "ready"; availability: RuntimeAvailability; snapshot: MarketplaceSnapshot }

/** What the screens can ask the marketplace to do. */
export interface MarketplaceActions {
  /** Id of the plugin with a request in flight, or "registry". */
  busy: string | null
  /** Why installs and updates are not possible right now. */
  blocked: string | null
  /** Plugins can be enabled and run here (not on Windows). */
  canRun: boolean
  openDetails: (id: string) => void
  install: (plugin: AvailablePlugin, version: AvailableVersion) => void
  installFile: () => void
  update: (candidate: UpdateCandidate) => void
  setEnabled: (item: PluginDetails, enabled: boolean) => void
  setGrant: (item: PluginDetails, key: string, granted: boolean) => void
  uninstall: (item: PluginDetails) => void
  rollback: (item: PluginDetails) => void
  invoke: (
    item: PluginDetails,
    action: string,
    params: Record<string, ActionParamValue>,
  ) => Promise<PluginActionOutcome>
  refresh: () => void
}

async function loadAll(client: PluginClient, refresh = false): Promise<Load> {
  let runtime
  try {
    runtime = await client.getRuntime()
  } catch (error) {
    const availability = availabilityOfError(error)
    return availability
      ? { status: "unavailable", availability }
      : { status: "error", ...pluginErrorText(error) }
  }
  const availability = availabilityOf(runtime)
  if (!runtime.available) return { status: "unavailable", availability }
  try {
    return { status: "ready", availability, snapshot: await loadSnapshot(client, runtime, refresh) }
  } catch (error) {
    return { status: "error", ...pluginErrorText(error) }
  }
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
 * Settings › Extensions › Plugins: the experimental marketplace. Only
 * reachable in builds with VITE_DECKPILOT_PLUGINS=1 (see the settings
 * registry). Everything comes from the agent; the page says why when the
 * agent cannot answer plugin requests.
 */
export function PluginsSettings() {
  const override = usePluginClientOverride()
  const [client] = useState<PluginClient>(() => override ?? createTauriPluginClient())
  const { agent } = useAppContext()
  const [load, setLoad] = useState<Load>({ status: "loading" })
  const [tab, setTab] = useState<Tab>("discover")
  const [selected, setSelected] = useState<string | null>(null)
  const [review, setReview] = useState<ReviewRequest | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [progress, setProgress] = useState<Map<string, Progress>>(new Map())
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
    return (
      <>
        {header}
        <Notice tone="destructive" title="Plugins could not be loaded">
          <p>{load.message}</p>
          {load.hint && <p>{load.hint}</p>}
          <div>
            <Button variant="outline" size="sm" onClick={retry}>
              Try again
            </Button>
          </div>
        </Notice>
      </>
    )
  }

  const { snapshot, availability } = load
  const blocked = installBlock(snapshot)
  const canRun = snapshot.runtime.executionSupported
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
      const { message, hint } = pluginErrorText(error)
      toast.error(message, hint ? { description: hint } : undefined)
    })
  }

  const openReview = (request: ReviewRequest) => {
    returnFocus.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    setReview(request)
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
          `${item.summary.name} updated to ${version}`,
        ),
      decline: () =>
        run(id, () => client.declineUpdate(id), `Update of ${item.summary.name} declined`),
    })
  }

  const actions: MarketplaceActions = {
    busy,
    blocked,
    canRun,
    openDetails: (id) => {
      setSelected(id)
    },
    install: (plugin, version) => {
      openReview({
        mode: "install",
        pluginId: plugin.id,
        name: plugin.name,
        publisher: plugin.publisher,
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
            `${plugin.name} installed`,
          ),
      })
    },
    installFile: () => {
      client.pickPackage().then(
        (path) => {
          if (!path) return
          // The package's permissions are known once the agent verified
          // it: nothing is granted now; enabling asks for consent.
          runReported(
            "package",
            () => client.installPlugin({ type: "file", path }, []),
            "Plugin installed from the package file; enable it to review its permissions",
          )
        },
        (error: unknown) => {
          const { message, hint } = pluginErrorText(error)
          toast.error(message, hint ? { description: hint } : undefined)
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
            toast.success(`${item.summary.name} updated to ${result.outcome.version}`)
          } else {
            // Staged until the user decides (approve or decline).
            reviewUpdate(item, result.outcome.version, result.outcome.changes)
          }
          await reload()
        })
        .catch((error: unknown) => {
          const { message, hint } = pluginErrorText(error)
          toast.error(message, hint ? { description: hint } : undefined)
        })
        .finally(() => {
          setBusy(null)
        })
    },
    setEnabled: (item, enabled) => {
      const id = item.summary.id
      if (!enabled) {
        runReported(id, () => client.disablePlugin(id), `${item.summary.name} disabled`)
        return
      }
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
          run(id, () => client.enablePlugin(id, consent), `${item.summary.name} enabled`),
      })
    },
    setGrant: (item, key, granted) => {
      const id = item.summary.id
      runReported(
        id,
        () => (granted ? client.grantPermissions(id, [key]) : client.revokePermission(id, key)),
        granted ? "Permission granted" : "Permission revoked",
      )
    },
    uninstall: (item) => {
      const id = item.summary.id
      runReported(id, () => client.uninstallPlugin(id), `${item.summary.name} uninstalled`)
    },
    rollback: (item) => {
      const id = item.summary.id
      runReported(
        id,
        () => client.rollbackPlugin(id),
        `${item.summary.name} rolled back to ${item.summary.previousVersion ?? "the previous version"}`,
      )
    },
    invoke: (item, action, params) => client.invokeAction(item.summary.id, action, params),
    refresh: () => {
      setBusy("registry")
      void reload(true).finally(() => {
        setBusy(null)
      })
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
          <TabsList aria-label="Plugin marketplace">
            <TabsTrigger value="discover">Discover</TabsTrigger>
            <TabsTrigger value="installed">Installed {snapshot.installed.length}</TabsTrigger>
            <TabsTrigger value="updates">Updates {updates.length}</TabsTrigger>
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
    </>
  )
}
