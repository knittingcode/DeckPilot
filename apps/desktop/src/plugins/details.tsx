import { ArrowLeft, CircleCheck, Download, RefreshCw } from "lucide-react"
import { useEffect, useRef } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

import { Notice, PermissionList, PluginIcon, PublisherLine } from "./components"
import type { MarketplaceActions } from "./marketplace"
import { STATE_LABELS, compareSemver, offeredVersion, type UpdateCandidate } from "./model"
import { grantsOf } from "./permissions"
import type { AvailablePlugin, MarketplaceSnapshot } from "./types"

/** One registry plugin: its versions, whether they run here and what they may do. */
export function PluginDetailsView({
  plugin,
  snapshot,
  updates,
  actions,
  onBack,
}: {
  plugin: AvailablePlugin
  snapshot: MarketplaceSnapshot
  updates: UpdateCandidate[]
  actions: MarketplaceActions
  onBack: () => void
}) {
  const heading = useRef<HTMLHeadingElement>(null)

  // Move focus to the page that replaced the list.
  useEffect(() => {
    heading.current?.focus()
  }, [])

  const installed = snapshot.installed.find((p) => p.summary.id === plugin.id)
  const update = updates.find((u) => u.plugin.summary.id === plugin.id)
  const offered = offeredVersion(plugin)
  const versions = [...plugin.versions].sort((a, b) => compareSemver(b.version, a.version))
  const shown = offered ?? versions[0]
  const busy = actions.busy === plugin.id

  const installReason = installed
    ? null
    : !offered
      ? "No version can be installed: every version was withdrawn or does not run here."
      : actions.blocked

  return (
    <>
      <div>
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft /> Back to plugins
        </Button>
      </div>
      <article className="grid gap-6" aria-labelledby="plugin-detail-name">
        <header className="flex items-start gap-3">
          <PluginIcon muted={!offered} />
          <div className="grid min-w-0 flex-1 gap-1">
            <h3
              id="plugin-detail-name"
              ref={heading}
              tabIndex={-1}
              className="text-lg font-semibold outline-none"
            >
              {plugin.name}
            </h3>
            <span className="font-mono text-xs text-muted-foreground select-text">{plugin.id}</span>
            <PublisherLine publisher={plugin.publisher} />
            <p className="text-sm">{installed?.description ?? plugin.summary}</p>
          </div>
        </header>

        <section className="flex flex-wrap items-center gap-3" aria-label="Install status">
          {installed ? (
            <>
              <Badge variant="secondary">
                <CircleCheck aria-hidden /> Installed {installed.summary.version} (
                {STATE_LABELS[installed.summary.state].toLowerCase()})
              </Badge>
              {update ? (
                <Button
                  disabled={busy || actions.blocked !== null}
                  onClick={() => {
                    actions.update(update)
                  }}
                >
                  <RefreshCw /> Update to{" "}
                  {update.kind === "staged" ? update.pending.version : update.version}
                </Button>
              ) : (
                <span className="text-sm text-muted-foreground">No update available</span>
              )}
            </>
          ) : (
            <Button
              disabled={busy || installReason !== null}
              aria-describedby={installReason ? "plugin-install-reason" : undefined}
              onClick={() => {
                if (offered) actions.install(plugin, offered)
              }}
            >
              <Download /> Install{offered ? ` ${offered.version}` : ""}
            </Button>
          )}
          {installReason && (
            <span id="plugin-install-reason" className="text-sm text-muted-foreground">
              {installReason}
            </span>
          )}
        </section>

        {shown && (shown.incompatible ?? []).length > 0 && (
          <Notice title="Not compatible with this DeckPilot">
            <ul className="grid gap-0.5" aria-label="Incompatibility reasons">
              {(shown.incompatible ?? []).map((reason) => (
                <li key={reason}>{reason}</li>
              ))}
            </ul>
          </Notice>
        )}
        {shown?.withdrawn && (
          <Notice tone="destructive" title={`Version ${shown.version} was withdrawn`}>
            <p>{shown.withdrawn}</p>
          </Notice>
        )}

        {shown && (
          <section aria-labelledby="plugin-detail-permissions" className="grid gap-2">
            <h4 id="plugin-detail-permissions" className="text-sm font-medium">
              Permissions of version {shown.version}
            </h4>
            <p className="text-xs text-muted-foreground">
              DeckPilot asks for your consent before it installs anything.
            </p>
            <PermissionList label="Permissions" grants={grantsOf(shown.permissions)} />
          </section>
        )}

        <section aria-labelledby="plugin-detail-versions" className="grid gap-2">
          <h4 id="plugin-detail-versions" className="text-sm font-medium">
            Versions
          </h4>
          <ul className="grid gap-1 text-sm" aria-label="Versions">
            {versions.map((v) => (
              <li key={v.version} className="flex flex-wrap items-center gap-2">
                <span className="font-mono text-xs">{v.version}</span>
                {v.version === plugin.latest && <Badge variant="outline">Latest</Badge>}
                {v.withdrawn && <Badge variant="destructive">Withdrawn: {v.withdrawn}</Badge>}
                {(v.incompatible ?? []).length > 0 && (
                  <Badge variant="outline">Not compatible</Badge>
                )}
                {installed?.summary.version === v.version && (
                  <Badge variant="secondary">Installed</Badge>
                )}
              </li>
            ))}
          </ul>
        </section>
      </article>
    </>
  )
}
