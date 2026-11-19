import { BadgeCheck, CircleAlert, FlaskConical, Loader2, Puzzle, RefreshCw } from "lucide-react"
import type { ReactNode } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
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
  const name = info?.displayName ?? publisher
  return (
    <span className="flex flex-wrap items-center gap-1 text-xs text-muted-foreground">
      <span>by {name}</span>
      {info && info.displayName && info.displayName !== info.namespace && (
        <span className="font-mono">({info.namespace})</span>
      )}
      {info?.verified ? (
        <Badge variant="secondary" className="h-5 gap-1 px-1.5 text-[11px]">
          <BadgeCheck aria-hidden className="size-3" /> Verified publisher
        </Badge>
      ) : info ? (
        <span>· not verified by the registry</span>
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
  return (
    <Alert role="note" aria-label="Experimental">
      <FlaskConical />
      <AlertTitle className="line-clamp-none">Experimental</AlertTitle>
      <AlertDescription>
        Plugins run out of process in a sandboxed host. The plugin runtime is experimental and not
        ready for production use; only install plugins you trust.
      </AlertDescription>
    </Alert>
  )
}

const RUNTIME_TITLES: Record<Exclude<RuntimeAvailability["kind"], "ready">, string> = {
  platformUnsupported: "Plugins cannot run on this platform",
  notBuilt: "Plugins are not available: the agent was built without plugin support",
  notEnabled: "Plugins are not available: they are turned off in the agent",
  unavailable: "Plugins are not available right now",
  agentNotRunning: "Plugins are not available: the agent is not running",
  agentTooOld: "Plugins are not available: the running agent is too old",
}

const RUNTIME_HINTS: Record<Exclude<RuntimeAvailability["kind"], "ready">, string> = {
  platformUnsupported:
    "Plugins can be installed and inspected here, but not enabled or run: the plugin sandbox is only verified on macOS, Linux and Windows.",
  notBuilt: "Plugins need an agent built with the `plugins` cargo feature.",
  notEnabled: "Start the agent without --disable-plugins (and without DECKPILOT_ENABLE_PLUGINS=0).",
  unavailable: "Unlock DeckPilot, or check the agent's log.",
  agentNotRunning: "Start it in Settings › Background Agent.",
  agentTooOld: "Restart it in Settings › Background Agent so that the agent of this release runs.",
}

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
  if (availability.kind === "ready") return null
  const message = "status" in availability ? availability.status.reason : availability.message
  const title =
    availability.kind === "platformUnsupported"
      ? `Plugins cannot run on ${platformLabel(availability.status.platform)}`
      : RUNTIME_TITLES[availability.kind]
  return (
    <Alert
      variant={availability.kind === "platformUnsupported" ? "default" : "destructive"}
      aria-label="Plugin runtime"
    >
      <CircleAlert />
      <AlertTitle className="line-clamp-none">{title}</AlertTitle>
      <AlertDescription className="grid gap-2">
        {message && <p>{message}</p>}
        <p>{RUNTIME_HINTS[availability.kind]}</p>
        {availability.kind !== "platformUnsupported" && (
          <div>
            <Button variant="outline" size="sm" disabled={busy} onClick={onRetry}>
              <RefreshCw /> Check again
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
  let title: string
  let text: string
  if (!registry) {
    title = "No plugin registry configured"
    text = "Start the agent with --plugin-registry and --plugin-registry-roots."
  } else if (error) {
    title = "The registry could not be read"
    text = error.message
  } else if (registry.freshness === "neverVerified") {
    title = "Registry not verified yet"
    text = "Nothing can be installed until the registry has been verified."
  } else if (registry.freshness === "stale") {
    title = "Registry stale"
    text = "Installed plugins keep running. Installing and updating are paused."
  } else {
    return null
  }
  return (
    <Alert variant={error ? "destructive" : "default"} aria-label="Registry">
      <CircleAlert />
      <AlertTitle className="line-clamp-none">{title}</AlertTitle>
      <AlertDescription className="grid gap-2">
        <p>{text}</p>
        {registry && <p className="font-mono text-xs break-all select-text">{registry.source}</p>}
        <div>
          <Button variant="outline" size="sm" disabled={busy} onClick={onRetry}>
            <RefreshCw /> Check the registry
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  )
}

const OPERATION_LABELS: Record<PluginOperation, string> = {
  install: "Installing",
  enable: "Enabling",
  disable: "Disabling",
  grant: "Granting",
  revoke: "Revoking",
  update: "Updating",
  rollback: "Rolling back",
  uninstall: "Uninstalling",
  recover: "Recovering",
}

const STAGE_LABELS: Record<PluginStage, string> = {
  started: "started",
  fetching: "reading the package",
  verifying: "verifying signatures and pins",
  installing: "unpacking and committing",
  finished: "done",
  failed: "failed",
}

export interface Progress {
  plugin: string
  operation: PluginOperation
  stage: PluginStage
  message?: string | undefined
}

/** Operations in flight, from the agent's progress events. */
export function ProgressList({ progress }: { progress: Progress[] }) {
  return (
    <div role="status" aria-live="polite" aria-label="Plugin operations" className="grid gap-1">
      {progress.map((p) => (
        <p key={p.plugin} className="flex items-center gap-2 text-sm text-muted-foreground">
          {p.stage !== "finished" && p.stage !== "failed" && (
            <Loader2 className="size-4 animate-spin" aria-hidden />
          )}
          <span>
            {OPERATION_LABELS[p.operation]} <span className="font-mono text-xs">{p.plugin}</span>:{" "}
            {STAGE_LABELS[p.stage]}
            {p.message ? ` (${p.message})` : ""}
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
  if (grants.length === 0) {
    return <p className="text-sm text-muted-foreground">This plugin asks for no permissions.</p>
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
                      <span className="text-muted-foreground">Publisher's reason: </span>
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
