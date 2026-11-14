import { ArrowLeft, CircleCheck, Download, RefreshCw } from "lucide-react"
import { useEffect, useRef, useState } from "react"

import { Loading } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

import { pluginErrorText, type PluginClient } from "./api"
import {
  compatibility,
  offeredVersion,
  platformList,
  revocationOf,
  versionPlatforms,
  compareSemver,
} from "./compat"
import { Notice, PermissionList, PluginIcon, PublisherLine } from "./components"
import type { MarketplaceActions } from "./marketplace"
import { grantsOf } from "./permissions"
import type { MarketplaceSnapshot, PluginDetail } from "./types"

type DetailLoad =
  | { status: "loading" }
  | { status: "error"; message: string; hint?: string }
  | { status: "ready"; detail: PluginDetail }

function formatDate(iso: string) {
  return new Date(iso).toLocaleDateString()
}

function formatSize(bytes: number) {
  return bytes < 1024 * 1024
    ? `${(bytes / 1024).toFixed(0)} KB`
    : `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

/** One plugin: what it is, who publishes it, whether it runs here and what it may do. */
export function PluginDetails({
  client,
  id,
  snapshot,
  actions,
  onBack,
}: {
  client: PluginClient
  id: string
  snapshot: MarketplaceSnapshot
  actions: MarketplaceActions
  onBack: () => void
}) {
  const [load, setLoad] = useState<DetailLoad>({ status: "loading" })
  const heading = useRef<HTMLHeadingElement>(null)

  useEffect(() => {
    let cancelled = false
    client.getPluginDetail(id).then(
      (detail) => {
        if (!cancelled) setLoad({ status: "ready", detail })
      },
      (error: unknown) => {
        if (!cancelled) setLoad({ status: "error", ...pluginErrorText(error) })
      },
    )
    return () => {
      cancelled = true
    }
  }, [client, id])

  // Move focus to the page that replaced the list.
  useEffect(() => {
    if (load.status !== "loading") heading.current?.focus()
  }, [load.status])

  const back = (
    <div>
      <Button variant="ghost" size="sm" onClick={onBack}>
        <ArrowLeft /> Back to plugins
      </Button>
    </div>
  )

  if (load.status === "loading") {
    return (
      <>
        {back}
        <Loading />
      </>
    )
  }
  if (load.status === "error") {
    return (
      <>
        {back}
        <h3 ref={heading} tabIndex={-1} className="sr-only">
          Plugin details
        </h3>
        <Notice tone="destructive" title="Plugin details could not be loaded">
          <p>{load.message}</p>
          {load.hint && <p>{load.hint}</p>}
        </Notice>
      </>
    )
  }

  const { detail } = load
  const { plugin, entry, publisher, revocations } = detail
  const installed = snapshot.installed.find((p) => p.id === plugin.id)
  const update = snapshot.updates.find((u) => u.id === plugin.id)
  const offered = offeredVersion(plugin.versions, revocations, snapshot.environment)
  const latest = [...plugin.versions].sort((a, b) => compareSemver(b.version, a.version))[0]
  const shown = offered?.version ?? latest
  const compat = shown ? compatibility(shown, snapshot.environment) : null
  const disabledRevocation = revocations.find((r) => r.action === "disable")
  const busy = actions.busy === plugin.id

  const installReason = installed
    ? null
    : !offered
      ? "No version can be installed: every version was withdrawn or revoked."
      : !offered.compatibility.compatible
        ? "Not compatible with this DeckPilot."
        : actions.blocked

  return (
    <>
      {back}
      <article className="grid gap-6" aria-labelledby="plugin-detail-name">
        <header className="flex items-start gap-3">
          <PluginIcon muted={!compat?.compatible} />
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
            <PublisherLine publisher={publisher} namespace={plugin.publisher} />
            <p className="text-sm">{plugin.description}</p>
          </div>
        </header>

        <section className="flex flex-wrap items-center gap-3" aria-label="Install status">
          {installed ? (
            <>
              <Badge variant="secondary">
                <CircleCheck aria-hidden /> Installed {installed.version}
                {installed.enabled ? "" : " (disabled)"}
              </Badge>
              {update ? (
                <Button
                  disabled={busy || actions.blocked !== null}
                  onClick={() => {
                    actions.update(installed, update)
                  }}
                >
                  <RefreshCw /> Update to {update.to}
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
                if (offered) actions.install(detail, offered)
              }}
            >
              <Download /> Install{offered ? ` ${offered.version.version}` : ""}
            </Button>
          )}
          {installReason && (
            <span id="plugin-install-reason" className="text-sm text-muted-foreground">
              {installReason}
            </span>
          )}
        </section>

        {disabledRevocation && (
          <Notice tone="destructive" title="Revoked by the registry">
            <p>{disabledRevocation.reason}</p>
            <p>
              Revoked versions do not run and cannot be installed.
              {disabledRevocation.advisory && (
                <>
                  {" "}
                  Advisory: <span className="select-text">{disabledRevocation.advisory}</span>
                </>
              )}
            </p>
          </Notice>
        )}
        {entry.deprecated && (
          <Notice title="Deprecated">
            <p>
              {entry.deprecated.reason}
              {entry.deprecated.replacement && ` Replacement: ${entry.deprecated.replacement}.`}
            </p>
          </Notice>
        )}
        {compat && !compat.compatible && (
          <Notice title="Not compatible with this DeckPilot">
            <ul className="grid gap-0.5" aria-label="Incompatibility reasons">
              {compat.reasons.map((reason) => (
                <li key={reason}>{reason}</li>
              ))}
            </ul>
          </Notice>
        )}

        {shown && (
          <section aria-labelledby="plugin-detail-version" className="grid gap-2">
            <h4 id="plugin-detail-version" className="text-sm font-medium">
              Version {shown.version}
            </h4>
            <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-[11rem_1fr]">
              <dt className="text-muted-foreground">Compatibility</dt>
              <dd>
                {compat?.compatible
                  ? `Works with DeckPilot ${snapshot.environment.appVersion} on ${platformList([snapshot.environment.platform])}`
                  : "Not compatible (see above)"}
              </dd>
              <dt className="text-muted-foreground">Requires</dt>
              <dd>
                DeckPilot {shown.engines.deckpilot}, plugin API {shown.engines.pluginApi}
              </dd>
              <dt className="text-muted-foreground">Supported platforms</dt>
              <dd>{platformList(versionPlatforms(shown))}</dd>
              <dt className="text-muted-foreground">Published</dt>
              <dd>{formatDate(shown.published_at)}</dd>
              <dt className="text-muted-foreground">License</dt>
              <dd>{plugin.license}</dd>
              <dt className="text-muted-foreground">Source</dt>
              <dd className="font-mono text-xs break-all select-text">
                {shown.source.repository} @ {shown.source.commit.slice(0, 12)}
              </dd>
              <dt className="text-muted-foreground">Package</dt>
              <dd>{formatSize(shown.package.size)}, SHA-256 pinned by the signed registry</dd>
            </dl>
          </section>
        )}

        {shown && (
          <section aria-labelledby="plugin-detail-permissions" className="grid gap-2">
            <h4 id="plugin-detail-permissions" className="text-sm font-medium">
              Permissions
            </h4>
            <p className="text-xs text-muted-foreground">
              What version {shown.version} asks for. DeckPilot asks for your consent before it
              installs anything. The plugin runtime that is designed to enforce these permissions is
              experimental.
            </p>
            <PermissionList label="Permissions" grants={grantsOf(shown.permissions)} />
          </section>
        )}

        <section aria-labelledby="plugin-detail-versions" className="grid gap-2">
          <h4 id="plugin-detail-versions" className="text-sm font-medium">
            Versions
          </h4>
          <ul className="grid gap-1 text-sm" aria-label="Versions">
            {[...plugin.versions]
              .sort((a, b) => compareSemver(b.version, a.version))
              .map((v) => {
                const revocation = revocationOf(revocations, v.version)
                return (
                  <li key={v.version} className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-xs">{v.version}</span>
                    <span className="text-xs text-muted-foreground">
                      {formatDate(v.published_at)}
                    </span>
                    {v.yanked && <Badge variant="outline">Withdrawn: {v.yanked.reason}</Badge>}
                    {revocation && (
                      <Badge variant="destructive">
                        {revocation.action === "disable" ? "Revoked" : "Withdrawn"}
                      </Badge>
                    )}
                    {installed?.version === v.version && (
                      <Badge variant="secondary">Installed</Badge>
                    )}
                  </li>
                )
              })}
          </ul>
        </section>
      </article>
    </>
  )
}
