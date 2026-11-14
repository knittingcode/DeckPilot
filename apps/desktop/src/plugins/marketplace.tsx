import { useEffect, useRef, useState } from "react"
import { toast } from "sonner"

import { Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"

import { pluginErrorText, usePluginClientOverride, type PluginClient } from "./api"
import { FixtureNotice, Notice, RegistryNotice } from "./components"
import { PluginDetails } from "./details"
import { DiscoverView } from "./discover"
import { createFixtureClient, type FixtureClient } from "./fixtures"
import { InstalledView } from "./installed"
import { installBlock } from "./model"
import { compareGrants, grantsOf } from "./permissions"
import { PermissionReviewDialog, type ReviewRequest } from "./review-dialog"
import type { MarketplaceSnapshot } from "./types"
import { UpdatesView } from "./updates"
import type { OfferedVersion } from "./compat"
import type { InstalledPlugin, PluginDetail, PluginUpdate } from "./types"

type Tab = "discover" | "installed" | "updates"

type Load =
  | { status: "loading" }
  | { status: "error"; message: string; hint?: string }
  | { status: "ready"; snapshot: MarketplaceSnapshot }

/** What the screens can ask the marketplace to do. */
export interface MarketplaceActions {
  /** Id of the plugin with a request in flight, or "registry". */
  busy: string | null
  /** Why installs and updates are not possible right now. */
  blocked: string | null
  openDetails: (id: string) => void
  install: (detail: PluginDetail, offered: OfferedVersion) => void
  update: (item: InstalledPlugin, update: PluginUpdate) => void
  setEnabled: (item: InstalledPlugin, enabled: boolean) => void
  setGrant: (item: InstalledPlugin, key: string, enabled: boolean) => void
  uninstall: (item: InstalledPlugin, deleteCredentials: boolean) => void
  rollback: (item: InstalledPlugin) => void
  refresh: () => void
}

function isFixtureClient(client: PluginClient): client is FixtureClient {
  return client.source === "fixtures" && "simulate" in client
}

/**
 * Settings › Extensions › Plugins: the experimental marketplace. Only
 * reachable in builds with VITE_DECKPILOT_PLUGINS=1 (see the settings
 * registry); it runs on development fixtures until the agent implements
 * the plugin requests.
 */
export function PluginsSettings() {
  const override = usePluginClientOverride()
  const [client] = useState<PluginClient>(() => override ?? createFixtureClient())
  const [load, setLoad] = useState<Load>({ status: "loading" })
  const [generation, setGeneration] = useState(0)
  const [tab, setTab] = useState<Tab>("discover")
  const [selected, setSelected] = useState<string | null>(null)
  const [review, setReview] = useState<ReviewRequest | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const returnFocus = useRef<HTMLElement | null>(null)

  useEffect(() => {
    let cancelled = false
    client.listPlugins().then(
      (snapshot) => {
        if (!cancelled) setLoad({ status: "ready", snapshot })
      },
      (error: unknown) => {
        if (!cancelled) setLoad({ status: "error", ...pluginErrorText(error) })
      },
    )
    return () => {
      cancelled = true
    }
  }, [client, generation])

  const fixtures = client.source === "fixtures"
  const simulate = isFixtureClient(client)
    ? (state: Parameters<FixtureClient["simulate"]>[0]) => {
        client.simulate(state)
        setGeneration((g) => g + 1)
      }
    : undefined
  const notice = (
    <FixtureNotice
      registry={load.status === "ready" ? load.snapshot.registry.state : undefined}
      onSimulate={simulate}
    />
  )

  if (load.status === "loading") {
    return (
      <>
        {fixtures && notice}
        <Loading />
      </>
    )
  }
  if (load.status === "error") {
    return (
      <>
        {fixtures && notice}
        <Notice tone="destructive" title="Plugins could not be loaded">
          <p>{load.message}</p>
          {load.hint && <p>{load.hint}</p>}
          <div>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setLoad({ status: "loading" })
                setGeneration((g) => g + 1)
              }}
            >
              Try again
            </Button>
          </div>
        </Notice>
      </>
    )
  }

  const snapshot = load.snapshot
  const blocked = installBlock(snapshot)

  /** Runs one request; the new snapshot replaces the old one. */
  const run = (id: string, request: () => Promise<MarketplaceSnapshot>, success: string) => {
    setBusy(id)
    return request()
      .then((next) => {
        setLoad({ status: "ready", snapshot: next })
        toast.success(success)
      })
      .finally(() => {
        setBusy(null)
      })
  }
  /** Like `run`, but reports failures as a toast instead of to a dialog. */
  const runReported = (
    id: string,
    request: () => Promise<MarketplaceSnapshot>,
    success: string,
  ) => {
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

  const actions: MarketplaceActions = {
    busy,
    blocked,
    openDetails: (id) => {
      setSelected(id)
    },
    install: (detail, offered) => {
      openReview({
        mode: "install",
        pluginId: detail.plugin.id,
        name: detail.plugin.name,
        namespace: detail.plugin.publisher,
        publisher: detail.publisher,
        version: offered.version.version,
        items: grantsOf(offered.version.permissions).map((grant) => ({ grant, state: "new" })),
        confirm: (consent) =>
          run(
            detail.plugin.id,
            () =>
              client.installPlugin({
                id: detail.plugin.id,
                version: offered.version.version,
                consent,
              }),
            `${detail.plugin.name} installed`,
          ),
      })
    },
    update: (item, candidate) => {
      const change = compareGrants(item.grants, candidate.permissions)
      const apply = (consent: string[]) =>
        run(
          item.id,
          () => client.approvePluginUpdate({ id: item.id, version: candidate.to, consent }),
          `${item.name} updated to ${candidate.to}`,
        )
      if (change.added.length === 0) {
        // Narrowing or unchanged permissions are applied without asking (§13.2).
        runReported(
          item.id,
          () => client.approvePluginUpdate({ id: item.id, version: candidate.to, consent: [] }),
          `${item.name} updated to ${candidate.to}`,
        )
        return
      }
      const entry = snapshot.catalog.find((c) => c.entry.id === item.id)
      openReview({
        mode: "update",
        pluginId: item.id,
        name: item.name,
        namespace: item.publisher,
        publisher: entry?.publisher ?? null,
        version: candidate.to,
        items: [
          ...change.added.map((grant) => ({ grant, state: "new" as const })),
          ...change.kept.map(({ grant, stored }) => ({
            grant,
            state: stored.enabled ? ("granted" as const) : ("revoked" as const),
          })),
        ],
        removed: change.removed.map(
          (g) => grantsOf([g.permission]).find((x) => x.key === g.key)?.title ?? g.key,
        ),
        confirm: apply,
      })
    },
    setEnabled: (item, enabled) => {
      if (!enabled) {
        runReported(item.id, () => client.setPluginEnabled(item.id, false), `${item.name} disabled`)
        return
      }
      const stored = new Map(item.grants.map((g) => [g.key, g]))
      const entry = snapshot.catalog.find((c) => c.entry.id === item.id)
      openReview({
        mode: "enable",
        pluginId: item.id,
        name: item.name,
        namespace: item.publisher,
        publisher: entry?.publisher ?? null,
        version: item.version,
        items: grantsOf(item.permissions).map((grant) => {
          const g = stored.get(grant.key)
          return { grant, state: !g ? "new" : g.enabled ? "granted" : "revoked" }
        }),
        confirm: (consent) =>
          run(
            item.id,
            () => client.setPluginEnabled(item.id, true, consent),
            `${item.name} enabled`,
          ),
      })
    },
    setGrant: (item, key, enabled) => {
      runReported(
        item.id,
        () => client.setPluginGrant(item.id, key, enabled),
        enabled ? "Permission allowed again" : "Permission revoked",
      )
    },
    uninstall: (item, deleteCredentials) => {
      runReported(
        item.id,
        () => client.uninstallPlugin(item.id, deleteCredentials),
        `${item.name} uninstalled`,
      )
    },
    rollback: (item) => {
      runReported(
        item.id,
        () => client.rollbackPlugin(item.id),
        `${item.name} rolled back to ${item.previousVersion ?? "the previous version"}`,
      )
    },
    refresh: () => {
      runReported("registry", () => client.refreshRegistry(), "Checked the registry")
    },
  }

  const closeReview = () => {
    setReview(null)
    returnFocus.current?.focus()
  }

  return (
    <>
      {fixtures && notice}
      <RegistryNotice status={snapshot.registry} busy={busy !== null} onRetry={actions.refresh} />
      {selected ? (
        <PluginDetails
          key={selected}
          client={client}
          id={selected}
          snapshot={snapshot}
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
            <TabsTrigger value="updates">Updates {snapshot.updates.length}</TabsTrigger>
          </TabsList>
          <TabsContent value="discover" className="grid gap-4 pt-2">
            <DiscoverView snapshot={snapshot} actions={actions} />
          </TabsContent>
          <TabsContent value="installed" className="grid gap-4 pt-2">
            <InstalledView
              snapshot={snapshot}
              actions={actions}
              onBrowse={() => {
                setTab("discover")
              }}
            />
          </TabsContent>
          <TabsContent value="updates" className="grid gap-4 pt-2">
            <UpdatesView snapshot={snapshot} actions={actions} />
          </TabsContent>
        </Tabs>
      )}
      {review && (
        <PermissionReviewDialog request={review} fixtures={fixtures} onClose={closeReview} />
      )}
    </>
  )
}
