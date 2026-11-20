import { ArrowLeft, CircleCheck, Download, RefreshCw } from "lucide-react"
import { useEffect, useRef } from "react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"

import { Notice, PermissionList, PluginIcon, PublisherLine } from "./components"
import { DependencySection, DependentsSection } from "./dependencies"
import type { MarketplaceActions } from "./marketplace"
import {
  categoryLabel,
  compareSemver,
  formatDate,
  knownUninstalled,
  offeredVersion,
  platformLabel,
  platformsText,
  stateLabel,
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
  const { t } = useTranslation()
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
      ? t("marketplace:details.noVersion")
      : actions.blocked

  const facts: [string, string][] = [
    ...(plugin.license
      ? [[t("marketplace:details.license"), plugin.license] as [string, string]]
      : []),
    ...(plugin.homepage
      ? [[t("marketplace:details.homepage"), plugin.homepage] as [string, string]]
      : []),
    ...(plugin.sourceRepository
      ? [
          [t("marketplace:details.source"), `github.com/${plugin.sourceRepository}`] as [
            string,
            string,
          ],
        ]
      : []),
    ...((plugin.categories ?? []).length > 0
      ? [
          [
            t("marketplace:details.categories"),
            (plugin.categories ?? []).map(categoryLabel).join(", "),
          ] as [string, string],
        ]
      : []),
  ]

  return (
    <>
      <div>
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft /> {t("marketplace:details.back")}
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
            aria-label={t("marketplace:details.about")}
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
          <Notice title={t("marketplace:details.deprecated")}>
            <p>{plugin.deprecated}</p>
          </Notice>
        )}

        <section
          className="flex flex-wrap items-center gap-3 border-y py-3"
          aria-label={t("marketplace:details.installStatus")}
        >
          {installed ? (
            <>
              <Badge variant="secondary">
                <CircleCheck aria-hidden />{" "}
                {t("marketplace:details.installed", {
                  version: installed.summary.version,
                  state: stateLabel(installed.summary.state).toLowerCase(),
                })}
              </Badge>
              {update ? (
                <Button
                  size="sm"
                  disabled={busy || actions.blocked !== null}
                  onClick={() => {
                    actions.update(update)
                  }}
                >
                  <RefreshCw />{" "}
                  {t("marketplace:details.updateTo", {
                    version: update.kind === "staged" ? update.pending.version : update.version,
                  })}
                </Button>
              ) : (
                <span className="text-sm text-muted-foreground">
                  {t("marketplace:details.noUpdate")}
                </span>
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
              <Download />{" "}
              {offered
                ? t("marketplace:details.installVersion", { version: offered.version })
                : t("marketplace:details.install")}
            </Button>
          )}
          {installReason && (
            <span id="plugin-install-reason" className="text-sm text-muted-foreground">
              {installReason}
            </span>
          )}
          {installed?.summary.installedFor && (
            <span className="text-sm text-muted-foreground">
              {t("marketplace:details.installedFor", { plugin: installed.summary.installedFor })}
            </span>
          )}
          {known && (
            <span className="text-sm text-muted-foreground">
              {t("marketplace:details.knownBefore", {
                name: known.name,
                version: known.lastVersion,
                date: formatDate(known.lastSeenAt),
              })}
            </span>
          )}
        </section>

        {installed?.storage && (
          <section aria-labelledby="plugin-detail-storage" className="grid gap-1">
            <h4 id="plugin-detail-storage" className="text-sm font-medium">
              {t("marketplace:details.storage")}
            </h4>
            <p className="text-sm">{storageText(installed.storage)}</p>
            <p className="text-xs text-muted-foreground">{t("marketplace:details.storageHint")}</p>
          </section>
        )}

        {shown && (
          <section aria-labelledby="plugin-detail-compatibility" className="grid gap-2">
            <h4 id="plugin-detail-compatibility" className="text-sm font-medium">
              {t("marketplace:details.compatibility", { version: shown.version })}
            </h4>
            <dl className="grid grid-cols-[max-content_1fr] gap-x-6 gap-y-1.5 text-sm">
              <dt className="text-muted-foreground">DeckPilot</dt>
              <dd>{shown.deckpilot ?? t("marketplace:details.notStated")}</dd>
              <dt className="text-muted-foreground">{t("marketplace:details.platforms")}</dt>
              <dd>{platformsText(shown.platforms)}</dd>
              <dt className="text-muted-foreground">{t("marketplace:details.thisComputer")}</dt>
              <dd>
                {(shown.incompatible ?? []).length === 0 && !shown.withdrawn
                  ? t("marketplace:details.compatibleOn", {
                      platform: platformLabel(snapshot.runtime.platform),
                    })
                  : platformLabel(snapshot.runtime.platform)}
              </dd>
            </dl>
          </section>
        )}

        {shown && (shown.incompatible ?? []).length > 0 && (
          <Notice title={t("marketplace:details.incompatible")}>
            <ul className="grid gap-0.5" aria-label={t("marketplace:details.incompatibleReasons")}>
              {(shown.incompatible ?? []).map((reason) => (
                <li key={reason}>{reason}</li>
              ))}
            </ul>
          </Notice>
        )}
        {shown?.withdrawn && (
          <Notice
            tone="destructive"
            title={t("marketplace:details.withdrawn", { version: shown.version })}
          >
            <p>{shown.withdrawn}</p>
          </Notice>
        )}

        {installed && installed.summary.version === shown?.version ? (
          <DependencySection
            title={t("plugins:dependencies.title")}
            dependencies={installed.dependencies ?? shown.dependencies ?? []}
          />
        ) : (
          shown && (
            <DependencySection
              title={t("plugins:dependencies.titleOfVersion", { version: shown.version })}
              dependencies={shown.dependencies ?? []}
            />
          )
        )}
        {installed && <DependentsSection dependents={installed.dependents ?? []} />}

        {shown && (
          <section aria-labelledby="plugin-detail-permissions" className="grid gap-2">
            <h4 id="plugin-detail-permissions" className="text-sm font-medium">
              {t("marketplace:details.permissions", { version: shown.version })}
            </h4>
            <p className="text-xs text-muted-foreground">
              {t("marketplace:details.permissionsHint")}
            </p>
            <PermissionList
              label={t("marketplace:details.permissionsLabel")}
              grants={grantsOf(shown.permissions)}
            />
          </section>
        )}

        <ActionsSection installed={installed} />

        <section aria-labelledby="plugin-detail-versions" className="grid gap-2">
          <h4 id="plugin-detail-versions" className="text-sm font-medium">
            {t("marketplace:details.versions")}
          </h4>
          <ul
            className="grid divide-y border-y text-sm"
            aria-label={t("marketplace:details.versions")}
          >
            {versions.map((v) => (
              <li key={v.version} className="flex flex-wrap items-center gap-2 py-1.5">
                <span className="font-mono text-xs">{v.version}</span>
                {v.publishedAt !== undefined && (
                  <span className="text-xs text-muted-foreground">
                    {t("marketplace:details.published", { date: formatDate(v.publishedAt) })}
                  </span>
                )}
                {v.deckpilot && (
                  <span className="text-xs text-muted-foreground">DeckPilot {v.deckpilot}</span>
                )}
                {v.version === plugin.latest && (
                  <Badge variant="outline">{t("marketplace:details.latest")}</Badge>
                )}
                {v.withdrawn && (
                  <Badge variant="destructive">
                    {t("marketplace:details.withdrawnBecause", { reason: v.withdrawn })}
                  </Badge>
                )}
                {(v.incompatible ?? []).length > 0 && (
                  <Badge variant="outline">{t("marketplace:badge.incompatible")}</Badge>
                )}
                {installed?.summary.version === v.version && (
                  <Badge variant="secondary">{t("marketplace:details.installedBadge")}</Badge>
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
  const { t } = useTranslation()
  return (
    <section aria-labelledby="plugin-detail-actions" className="grid gap-2">
      <h4 id="plugin-detail-actions" className="text-sm font-medium">
        {t("marketplace:details.actions")}
      </h4>
      {!installed ? (
        <p className="text-xs text-muted-foreground">{t("marketplace:details.actionsNotListed")}</p>
      ) : installed.summary.actions.length === 0 ? (
        <p className="text-xs text-muted-foreground">{t("marketplace:details.noActions")}</p>
      ) : (
        <ul className="grid divide-y border-y" aria-label={t("marketplace:details.actions")}>
          {installed.summary.actions.map((action) => (
            <li key={action.action} className="grid gap-0.5 py-2.5 text-sm">
              <span className="flex flex-wrap items-center gap-2">
                <span className="font-medium">{action.name}</span>
                <span className="font-mono text-xs text-muted-foreground">{action.actionType}</span>
                {!action.available && (
                  <Badge variant="outline">{t("marketplace:details.unavailable")}</Badge>
                )}
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
