import { BadgeCheck, CircleAlert, FlaskConical, Loader2, Puzzle, RefreshCw } from "lucide-react"
import type { ReactNode } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Trans, type TranslationKey, useTranslation } from "@/i18n"
import { cn } from "@/lib/utils"

import { platformLabel, platformsText } from "./model"
import { grantsByTier, type Grant } from "./permissions"
import type {
  AvailableVersion,
  PluginOperation,
  PluginStage,
  PublisherInfo,
  RegistryStatus,
  RuntimeAvailability,
} from "./types"

/**
 * The plugin's icon: the installed version's PNG as the agent read and
 * checked it (`PluginDetails.icon`). The registry has no icons, so plugins
 * that are not installed show the generic one.
 */
export function PluginIcon({ muted = false, src }: { muted?: boolean; src?: string | undefined }) {
  // Only data URLs of PNGs (what the agent sends), never a remote image.
  if (src?.startsWith("data:image/png;base64,")) {
    return (
      <img
        src={src}
        alt=""
        aria-hidden
        className={cn("size-8 shrink-0 rounded-md object-cover", muted && "opacity-60")}
      />
    )
  }
  return (
    <div
      className={cn(
        "flex size-8 shrink-0 items-center justify-center rounded-md bg-muted",
        muted ? "text-muted-foreground" : "text-foreground",
      )}
      aria-hidden
    >
      <Puzzle className="size-4" />
    </div>
  )
}

/**
 * Who publishes the plugin: the registry's name for the publisher and
 * whether the registry verified it, when the registry lists it.
 */
export function PublisherLine({
  publisher,
  info,
}: {
  publisher: string
  info?: PublisherInfo | undefined
}) {
  const { t } = useTranslation()
  const name = info?.displayName ?? publisher
  return (
    <span className="flex flex-wrap items-center gap-1 text-xs text-muted-foreground">
      <span>{t("marketplace:publisher.by", { name })}</span>
      {info && info.displayName && info.displayName !== info.namespace && (
        <span className="font-mono">({info.namespace})</span>
      )}
      {info?.verified ? (
        <Badge variant="secondary" className="h-5 gap-1 px-1.5 text-[11px]">
          <BadgeCheck aria-hidden className="size-3" /> {t("marketplace:publisher.verified")}
        </Badge>
      ) : info ? (
        <span>{t("marketplace:publisher.notVerified")}</span>
      ) : null}
    </span>
  )
}

/** What a registry version needs: the DeckPilot range and the platforms. */
export function Compatibility({ version }: { version: AvailableVersion }) {
  return (
    <span className="flex flex-wrap gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
      {version.deckpilot && <span>DeckPilot {version.deckpilot}</span>}
      <span>{platformsText(version.platforms)}</span>
    </span>
  )
}

/** Shown above the marketplace in every state: what it is and what it is not. */
export function ExperimentalNotice() {
  const { t } = useTranslation()
  return (
    <Alert role="note" aria-label={t("marketplace:experimental.title")}>
      <FlaskConical />
      <AlertTitle className="line-clamp-none">{t("marketplace:experimental.title")}</AlertTitle>
      <AlertDescription>{t("marketplace:experimental.text")}</AlertDescription>
    </Alert>
  )
}

type Unavailable = Exclude<RuntimeAvailability["kind"], "ready">

const RUNTIME_TITLES = {
  platformUnsupported: "marketplace:runtime.title.platformUnsupported",
  notBuilt: "marketplace:runtime.title.notBuilt",
  notEnabled: "marketplace:runtime.title.notEnabled",
  unavailable: "marketplace:runtime.title.unavailable",
  agentNotRunning: "marketplace:runtime.title.agentNotRunning",
  agentTooOld: "marketplace:runtime.title.agentTooOld",
} as const satisfies Record<Unavailable, TranslationKey>

const RUNTIME_HINTS = {
  platformUnsupported: "marketplace:runtime.hint.platformUnsupported",
  notBuilt: "marketplace:runtime.hint.notBuilt",
  notEnabled: "marketplace:runtime.hint.notEnabled",
  unavailable: "marketplace:runtime.hint.unavailable",
  agentNotRunning: "marketplace:runtime.hint.agentNotRunning",
  agentTooOld: "marketplace:runtime.hint.agentTooOld",
} as const satisfies Record<Unavailable, TranslationKey>

/** Why plugins cannot (fully) be used; nothing when they can. */
export function RuntimeNotice({
  availability,
  busy,
  onRetry,
}: {
  availability: RuntimeAvailability
  busy: boolean
  onRetry: () => void
}) {
  const { t } = useTranslation()
  if (availability.kind === "ready") return null
  const message = "status" in availability ? availability.status.reason : availability.message
  const title =
    availability.kind === "platformUnsupported"
      ? t("marketplace:runtime.cannotRunOn", {
          platform: platformLabel(availability.status.platform),
        })
      : t(RUNTIME_TITLES[availability.kind])
  return (
    <Alert
      variant={availability.kind === "platformUnsupported" ? "default" : "destructive"}
      aria-label={t("marketplace:runtime.label")}
    >
      <CircleAlert />
      <AlertTitle className="line-clamp-none">{title}</AlertTitle>
      <AlertDescription className="grid gap-2">
        {message && <p>{message}</p>}
        <p>{t(RUNTIME_HINTS[availability.kind])}</p>
        {availability.kind !== "platformUnsupported" && (
          <div>
            <Button variant="outline" size="sm" disabled={busy} onClick={onRetry}>
              <RefreshCw /> {t("marketplace:runtime.checkAgain")}
            </Button>
          </div>
        )}
      </AlertDescription>
    </Alert>
  )
}

export function RegistryNotice({
  registry,
  error,
  busy,
  onRetry,
}: {
  registry: RegistryStatus | null
  error?: { reason: string; message: string } | undefined
  busy: boolean
  onRetry: () => void
}) {
  const { t } = useTranslation()
  let title: string
  let text: string
  if (!registry) {
    title = t("marketplace:registry.none.title")
    text = t("marketplace:registry.none.text")
  } else if (error) {
    title = t("marketplace:registry.error.title")
    text = error.message
  } else if (registry.freshness === "neverVerified") {
    title = t("marketplace:registry.neverVerified.title")
    text = t("marketplace:registry.neverVerified.text")
  } else if (registry.freshness === "stale") {
    title = t("marketplace:registry.stale.title")
    text = t("marketplace:registry.stale.text")
  } else {
    return null
  }
  return (
    <Alert variant={error ? "destructive" : "default"} aria-label={t("marketplace:registry.label")}>
      <CircleAlert />
      <AlertTitle className="line-clamp-none">{title}</AlertTitle>
      <AlertDescription className="grid gap-2">
        <p>{text}</p>
        {registry && <p className="font-mono text-xs break-all select-text">{registry.source}</p>}
        <div>
          <Button variant="outline" size="sm" disabled={busy} onClick={onRetry}>
            <RefreshCw /> {t("marketplace:registry.check")}
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  )
}

const OPERATION_LABELS = {
  install: "marketplace:progress.operation.install",
  enable: "marketplace:progress.operation.enable",
  disable: "marketplace:progress.operation.disable",
  grant: "marketplace:progress.operation.grant",
  revoke: "marketplace:progress.operation.revoke",
  update: "marketplace:progress.operation.update",
  rollback: "marketplace:progress.operation.rollback",
  uninstall: "marketplace:progress.operation.uninstall",
  recover: "marketplace:progress.operation.recover",
} as const satisfies Record<PluginOperation, TranslationKey>

const STAGE_LABELS = {
  started: "marketplace:progress.stage.started",
  fetching: "marketplace:progress.stage.fetching",
  verifying: "marketplace:progress.stage.verifying",
  installing: "marketplace:progress.stage.installing",
  finished: "marketplace:progress.stage.finished",
  failed: "marketplace:progress.stage.failed",
} as const satisfies Record<PluginStage, TranslationKey>

export interface Progress {
  plugin: string
  operation: PluginOperation
  stage: PluginStage
  message?: string | undefined
}

/** Operations in flight, from the agent's progress events. */
export function ProgressList({ progress }: { progress: Progress[] }) {
  const { t } = useTranslation()
  return (
    <div
      role="status"
      aria-live="polite"
      aria-label={t("marketplace:progress.label")}
      className="grid gap-1"
    >
      {progress.map((p) => (
        <p key={p.plugin} className="flex items-center gap-2 text-sm text-muted-foreground">
          {p.stage !== "finished" && p.stage !== "failed" && (
            <Loader2 className="size-4 animate-spin" aria-hidden />
          )}
          <span>
            <Trans
              i18nKey={
                p.message ? "marketplace:progress.lineWithMessage" : "marketplace:progress.line"
              }
              values={{
                operation: t(OPERATION_LABELS[p.operation]),
                plugin: p.plugin,
                stage: t(STAGE_LABELS[p.stage]),
                message: p.message ?? "",
              }}
              components={{ id: <span className="font-mono text-xs" /> }}
            />
          </span>
        </p>
      ))}
    </div>
  )
}

/** Grants grouped by risk, with each justification from the manifest. */
export function PermissionList({
  grants,
  label,
  renderControl,
}: {
  grants: Grant[]
  label: string
  /** A control per grant, e.g. a checkbox or a revoke button. */
  renderControl?: (grant: Grant) => ReactNode
}) {
  const { t } = useTranslation()
  if (grants.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("permissions:list.none")}</p>
  }
  return (
    <div className="grid gap-4" aria-label={label} role="group">
      {grantsByTier(grants).map((group) => (
        <section key={group.tier} className="grid gap-2" aria-label={group.label}>
          <div>
            <h4
              className={cn(
                "text-xs font-medium uppercase",
                group.tier === "high" ? "text-destructive" : "text-muted-foreground",
              )}
            >
              {group.label}
            </h4>
            <p className="text-xs text-muted-foreground">{group.description}</p>
          </div>
          <ul className="grid divide-y border-y">
            {group.grants.map((grant) => (
              <li key={grant.key} className="flex items-start gap-3 py-2.5">
                <div className="grid min-w-0 flex-1 gap-0.5 text-sm">
                  <span className="font-medium break-words">{grant.title}</span>
                  <span className="font-mono text-xs text-muted-foreground">{grant.key}</span>
                  {grant.detail && (
                    <span className="text-xs text-muted-foreground">{grant.detail}</span>
                  )}
                  {grant.justification && (
                    <span className="text-xs">
                      <span className="text-muted-foreground">
                        {t("permissions:list.publisherReason")}{" "}
                      </span>
                      {grant.justification}
                    </span>
                  )}
                </div>
                {renderControl?.(grant)}
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  )
}

export function Notice({
  tone = "default",
  title,
  children,
}: {
  tone?: "default" | "destructive"
  title: string
  children?: ReactNode
}) {
  return (
    <Alert variant={tone}>
      <CircleAlert />
      <AlertTitle className="line-clamp-none">{title}</AlertTitle>
      {children && <AlertDescription className="grid gap-2">{children}</AlertDescription>}
    </Alert>
  )
}
