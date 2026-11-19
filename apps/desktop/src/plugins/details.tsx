import { ArrowLeft, CircleCheck, Download, RefreshCw } from "lucide-react"
import { useEffect, useRef } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

import { Notice, PermissionList, PluginIcon, PublisherLine } from "./components"
import { DependencySection, DependentsSection } from "./dependencies"
import type { MarketplaceActions } from "./marketplace"
import {
  STATE_LABELS,
  categoryLabel,
  compareSemver,
  formatDate,
  knownUninstalled,
  offeredVersion,
  platformLabel,
  platformsText,
  storageText,
  type UpdateCandidate,
} from "./model"
import { grantsOf } from "./permissions"
import type { AvailablePlugin, MarketplaceSnapshot, PluginDetails } from "./types"

/**
 * One registry plugin: what it is, who publishes it, which versions there
 * are, whether they run here, what they may do and (once installed) which
 * actions it adds. The registry has no changelog, so none is shown.
 */
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
  const known = installed ? undefined : knownUninstalled(snapshot, plugin.id)
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

  const facts: [string, string][] = [
    ...(plugin.license ? [["License", plugin.license] as [string, string]] : []),
    ...(plugin.homepage ? [["Homepage", plugin.homepage] as [string, string]] : []),
    ...(plugin.sourceRepository
      ? [["Source", `github.com/${plugin.sourceRepository}`] as [string, string]]
      : []),
    ...((plugin.categories ?? []).length > 0
      ? [
          ["Categories", (plugin.categories ?? []).map(categoryLabel).join(", ")] as [
            string,
            string,
          ],
        ]
      : []),
  ]

  return (
    <>
      <div>
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft /> Back to plugins
        </Button>
      </div>
      <article className="grid gap-5" aria-labelledby="plugin-detail-name">
        <header className="flex items-start gap-3">
          <PluginIcon muted={!offered && !installed} src={installed?.icon} />
          <div className="grid min-w-0 flex-1 gap-1">
            <h3
              id="plugin-detail-name"
              ref={heading}
              tabIndex={-1}
              className="text-lg leading-7 font-semibold outline-none"
            >
              {plugin.name}
            </h3>
            <span className="font-mono text-xs text-muted-foreground select-text">{plugin.id}</span>
            <PublisherLine publisher={plugin.publisher} info={plugin.publisherInfo} />
            <p className="mt-1 text-sm whitespace-pre-line select-text">
              {plugin.description ?? installed?.description ?? plugin.summary}
            </p>
          </div>
        </header>

        {facts.length > 0 && (
          <dl
            className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1.5 border-t pt-4 text-sm"
            aria-label="About"
          >
            {facts.map(([term, value]) => (
              <div key={term} className="contents">
                <dt className="text-muted-foreground">{term}</dt>
                <dd className="break-all select-text">{value}</dd>
              </div>
            ))}
          </dl>
        )}

        {plugin.deprecated && (
          <Notice title="Deprecated by the registry">
            <p>{plugin.deprecated}</p>
          </Notice>
        )}

        <section
          className="flex flex-wrap items-center gap-3 border-y py-3"
          aria-label="Install status"
        >
          {installed ? (
            <>
              <Badge variant="secondary">
                <CircleCheck aria-hidden /> Installed {installed.summary.version} (
                {STATE_LABELS[installed.summary.state].toLowerCase()})
              </Badge>
              {update ? (
                <Button
                  size="sm"
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
              size="sm"
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
          {installed?.summary.installedFor && (
            <span className="text-sm text-muted-foreground">
              Installed for {installed.summary.installedFor}; it is offered for removal once nothing
              needs it.
            </span>
          )}
          {known && (
            <span className="text-sm text-muted-foreground">
              Installed here before as {known.name}, last version {known.lastVersion} (
              {formatDate(known.lastSeenAt)}). Actions that still use it say it is not installed.
            </span>
          )}
        </section>

        {installed?.storage && (
          <section aria-labelledby="plugin-detail-storage" className="grid gap-1">
            <h4 id="plugin-detail-storage" className="text-sm font-medium">
              Storage
            </h4>
            <p className="text-sm">{storageText(installed.storage)}</p>
            <p className="text-xs text-muted-foreground">
              What the plugin keeps for itself in DeckPilot. It is deleted when the plugin is
              uninstalled.
            </p>
          </section>
        )}

        {shown && (
          <section aria-labelledby="plugin-detail-compatibility" className="grid gap-2">
            <h4 id="plugin-detail-compatibility" className="text-sm font-medium">
              Compatibility of version {shown.version}
            </h4>
            <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1.5 text-sm">
              <dt className="text-muted-foreground">DeckPilot</dt>
              <dd>{shown.deckpilot ?? "Not stated by the registry"}</dd>
              <dt className="text-muted-foreground">Platforms</dt>
              <dd>{platformsText(shown.platforms)}</dd>
              <dt className="text-muted-foreground">This computer</dt>
              <dd>
                {platformLabel(snapshot.runtime.platform)}
                {(shown.incompatible ?? []).length === 0 && !shown.withdrawn ? ": compatible" : ""}
              </dd>
            </dl>
          </section>
        )}

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

        {installed && installed.summary.version === shown?.version ? (
          <DependencySection
            title="Dependencies"
            dependencies={installed.dependencies ?? shown.dependencies ?? []}
          />
        ) : (
          shown && (
            <DependencySection
              title={`Dependencies of version ${shown.version}`}
              dependencies={shown.dependencies ?? []}
            />
          )
        )}
        {installed && <DependentsSection dependents={installed.dependents ?? []} />}

        {shown && (
          <section aria-labelledby="plugin-detail-permissions" className="grid gap-2">
            <h4 id="plugin-detail-permissions" className="text-sm font-medium">
              Permissions of version {shown.version}
            </h4>
            <p className="text-xs text-muted-foreground">
              Each with the publisher's reason. DeckPilot asks for your consent before it installs
              anything.
            </p>
            <PermissionList label="Permissions" grants={grantsOf(shown.permissions)} />
          </section>
        )}

        <ActionsSection installed={installed} />

        <section aria-labelledby="plugin-detail-versions" className="grid gap-2">
          <h4 id="plugin-detail-versions" className="text-sm font-medium">
            Versions
          </h4>
          <ul className="grid divide-y border-y text-sm" aria-label="Versions">
            {versions.map((v) => (
              <li key={v.version} className="flex flex-wrap items-center gap-2 py-1.5">
                <span className="font-mono text-xs">{v.version}</span>
                {v.publishedAt !== undefined && (
                  <span className="text-xs text-muted-foreground">
                    published {formatDate(v.publishedAt)}
                  </span>
                )}
                {v.deckpilot && (
                  <span className="text-xs text-muted-foreground">DeckPilot {v.deckpilot}</span>
                )}
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

/** The actions the installed version adds; the registry does not list them. */
function ActionsSection({ installed }: { installed: PluginDetails | undefined }) {
  return (
    <section aria-labelledby="plugin-detail-actions" className="grid gap-2">
      <h4 id="plugin-detail-actions" className="text-sm font-medium">
        Actions
      </h4>
      {!installed ? (
        <p className="text-xs text-muted-foreground">
          The registry does not list a plugin's actions; they are shown here once it is installed.
        </p>
      ) : installed.summary.actions.length === 0 ? (
        <p className="text-xs text-muted-foreground">This plugin adds no actions.</p>
      ) : (
        <ul className="grid divide-y border-y" aria-label="Actions">
          {installed.summary.actions.map((action) => (
            <li key={action.action} className="grid gap-0.5 py-2.5 text-sm">
              <span className="flex flex-wrap items-center gap-2">
                <span className="font-medium">{action.name}</span>
                <span className="font-mono text-xs text-muted-foreground">{action.actionType}</span>
                {!action.available && <Badge variant="outline">Unavailable</Badge>}
              </span>
              {action.description && (
                <span className="text-xs text-muted-foreground">{action.description}</span>
              )}
              {!action.available && action.unavailableReason && (
                <span className="text-xs text-muted-foreground">{action.unavailableReason}</span>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
