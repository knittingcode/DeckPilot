import { BadgeCheck, CircleAlert, FlaskConical, Puzzle, RefreshCw, WifiOff } from "lucide-react"
import type { ReactNode } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { cn } from "@/lib/utils"

import { FIXTURE_LABEL } from "./fixtures"
import { grantsByTier, type Grant } from "./permissions"
import { publisherName } from "./model"
import type { RegistryPublisher, RegistryState, RegistryStatus } from "./types"

export function PluginIcon({ muted = false }: { muted?: boolean }) {
  // Manifest icons are PNGs inside the package; until packages are fetched
  // every plugin shows the generic icon.
  return (
    <div
      className={cn(
        "flex size-10 shrink-0 items-center justify-center rounded-lg border",
        muted ? "bg-muted text-muted-foreground" : "bg-primary/10 text-primary",
      )}
      aria-hidden
    >
      <Puzzle className="size-5" />
    </div>
  )
}

export function PublisherLine({
  publisher,
  namespace,
}: {
  publisher: RegistryPublisher | null
  namespace: string
}) {
  return (
    <span className="inline-flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
      <span>
        by {publisherName(publisher, namespace)}
        {publisher && <span className="font-mono"> @{publisher.login}</span>}
      </span>
      {publisher?.verified ? (
        <Badge variant="secondary">
          <BadgeCheck aria-hidden /> Verified publisher
        </Badge>
      ) : (
        <Badge variant="outline">Unverified publisher</Badge>
      )}
    </span>
  )
}

const SIMULATED: { value: RegistryState; label: string }[] = [
  { value: "fresh", label: "Up to date" },
  { value: "offline", label: "Offline" },
  { value: "stale", label: "Stale" },
  { value: "verification-failed", label: "Verification failed" },
  { value: "never-reached", label: "Never reached" },
]

/** Shown on every screen that uses the fixture client. */
export function FixtureNotice({
  registry,
  onSimulate,
}: {
  registry?: RegistryState
  onSimulate?: (state: RegistryState) => void
}) {
  return (
    <Alert role="note" aria-label={FIXTURE_LABEL}>
      <FlaskConical />
      <AlertTitle className="line-clamp-none">{FIXTURE_LABEL}</AlertTitle>
      <AlertDescription className="grid gap-2">
        <p>
          The plugin marketplace is an experimental preview. Plugins, publishers and versions here
          are made up; nothing is downloaded or installed and no plugin code runs. Permissions
          describe what DeckPilot is designed to enforce; the plugin runtime and its sandbox are not
          implemented and not ready for production use.
        </p>
        {onSimulate && registry && (
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-xs" id="plugins-simulate">
              Simulated registry state
            </span>
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              value={registry}
              aria-labelledby="plugins-simulate"
              className="flex-wrap"
              onValueChange={(value) => {
                if (value) onSimulate(value as RegistryState)
              }}
            >
              {SIMULATED.map((item) => (
                <ToggleGroupItem key={item.value} value={item.value} className="px-2 text-xs">
                  {item.label}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </div>
        )}
      </AlertDescription>
    </Alert>
  )
}

function formatTime(iso: string | undefined): string {
  return iso ? new Date(iso).toLocaleString() : "never"
}

const REGISTRY_TITLES: Record<Exclude<RegistryState, "fresh">, string> = {
  offline: "Offline: showing the last verified registry",
  stale: "Registry stale",
  "verification-failed": "Registry verification failed",
  "never-reached": "Registry not reached yet",
}

export function RegistryNotice({
  status,
  busy,
  onRetry,
}: {
  status: RegistryStatus
  busy: boolean
  onRetry: () => void
}) {
  if (status.state === "fresh") return null
  const failed = status.state === "verification-failed"
  return (
    <Alert variant={failed ? "destructive" : "default"}>
      {status.state === "offline" ? <WifiOff /> : <CircleAlert />}
      <AlertTitle>{REGISTRY_TITLES[status.state]}</AlertTitle>
      <AlertDescription className="grid gap-2">
        {status.message && <p>{status.message}</p>}
        <p>
          Installed plugins keep running.{" "}
          {status.state === "never-reached"
            ? "Nothing can be browsed until the registry is reached."
            : `Last verified: ${formatTime(status.verifiedAt)}. Installing and updating are paused.`}
        </p>
        <div>
          <Button variant="outline" size="sm" disabled={busy} onClick={onRetry}>
            <RefreshCw /> Try again
          </Button>
        </div>
      </AlertDescription>
    </Alert>
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
        <section key={group.tier} className="grid gap-2" aria-labelledby={`tier-${group.tier}`}>
          <div>
            <h4
              id={`tier-${group.tier}`}
              className={cn(
                "text-xs font-medium uppercase",
                group.tier === "high" ? "text-destructive" : "text-muted-foreground",
              )}
            >
              {group.label}
            </h4>
            <p className="text-xs text-muted-foreground">{group.description}</p>
          </div>
          <ul className="grid gap-2">
            {group.grants.map((grant) => (
              <li key={grant.key} className="flex items-start gap-3 rounded-lg border p-3">
                <div className="grid min-w-0 flex-1 gap-0.5 text-sm">
                  <span className="font-medium break-words">{grant.title}</span>
                  <span className="text-xs text-muted-foreground">{grant.detail}</span>
                  <span className="text-xs">
                    <span className="text-muted-foreground">Publisher's reason: </span>
                    {grant.justification}
                  </span>
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
