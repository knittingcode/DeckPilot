// The one-time consent dialog of a module migration
// (docs/marketplace/module-migration.md §5.5): what the plugin that
// replaces a built-in module may use beyond what is derived, item by item.
// Credentials and the private network always come here; nothing is
// granted until the user allows it.
//
// It lists exactly what the agent says waits (`MigrationConsentItem`):
// the permission, and for a credential the stored credential's id, the
// built-in's setting, the plugin's slot and the only hosts it is sent to.
// It never shows, asks for or receives a credential's value: only those
// named fields are rendered.

import { KeyRound, ShieldAlert } from "lucide-react"
import { useState } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { cn } from "@/lib/utils"

import { pluginErrorText, type PluginErrorText } from "./api"
import { grantOfInfo, TIERS } from "./permissions"
import type { MigrationConsentItem, ModuleMigration } from "./types"

type Choice = "allow" | "decline"

/** The items still waiting for a decision. */
function waitingItems(migration: ModuleMigration): MigrationConsentItem[] {
  return (migration.consent ?? []).filter((item) => item.decision === "pending")
}

function ImportFacts({ item }: { item: MigrationConsentItem }) {
  const facts = item.import
  if (!facts) return null
  // Names only, field by field: never anything else the answer may carry.
  return (
    <dl
      className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-0.5 text-xs"
      aria-label="What is imported"
    >
      <dt className="text-muted-foreground">Stored credential</dt>
      <dd className="font-mono break-all">{facts.credential}</dd>
      <dt className="text-muted-foreground">From the setting</dt>
      <dd className="font-mono break-all">{facts.setting}</dd>
      <dt className="text-muted-foreground">Into the plugin's slot</dt>
      <dd className="font-mono break-all">{facts.slot}</dd>
      <dt className="text-muted-foreground">Sent only to</dt>
      <dd className="font-mono break-all">
        {facts.hosts.length > 0 ? facts.hosts.join(", ") : "the hosts the plugin declares"}
      </dd>
    </dl>
  )
}

/**
 * Asks, item by item, whether the plugin replacing `migration.moduleName`
 * may have what the migration waits for. Each item is allowed or declined
 * on its own; there is no "allow all". Confirming sends exactly those
 * decisions; "Not now" decides nothing and the built-in keeps running.
 */
export function MigrationConsentDialog({
  migration,
  onDecide,
  onClose,
}: {
  migration: ModuleMigration
  /** Sends the decisions; resolves when the agent applied them. */
  onDecide: (approve: string[], decline: string[]) => Promise<void>
  onClose: () => void
}) {
  const items = waitingItems(migration)
  const [choices, setChoices] = useState<Map<string, Choice>>(new Map())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<PluginErrorText | null>(null)
  const undecided = items.filter((item) => !choices.has(item.key)).length
  const plugin = migration.pluginName
    ? `${migration.pluginName} (${migration.plugin})`
    : migration.plugin

  const choose = (key: string, choice: Choice) => {
    setChoices((previous) => new Map(previous).set(key, choice))
  }

  const submit = () => {
    const approve = items.filter((i) => choices.get(i.key) === "allow").map((i) => i.key)
    const decline = items.filter((i) => choices.get(i.key) === "decline").map((i) => i.key)
    setBusy(true)
    setError(null)
    onDecide(approve, decline).then(onClose, (e: unknown) => {
      setError(pluginErrorText(e))
      setBusy(false)
    })
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose()
      }}
    >
      <DialogContent className="max-h-[90svh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{migration.moduleName} actions now run as a plugin</DialogTitle>
          <DialogDescription>
            The plugin {plugin} replaces the built-in {migration.moduleName} module. It gets nothing
            the built-in did not use, and these only with your consent. Until you allow every item,
            the built-in keeps running your actions.
          </DialogDescription>
        </DialogHeader>
        <p className="flex items-start gap-2 text-xs text-muted-foreground">
          <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden />
          <span>
            Credentials and access to your local network are never granted automatically. Stored
            credentials stay where they are; the plugin gets a copy in its own slot.
          </span>
        </p>
        {items.length === 0 ? (
          <p className="text-sm text-muted-foreground">Nothing waits for your consent.</p>
        ) : (
          <ul className="grid gap-2" aria-label="What the plugin asks for">
            {items.map((item) => {
              const grant = grantOfInfo(item.permission)
              const tier = TIERS.find((t) => t.tier === grant.tier)
              const choice = choices.get(item.key)
              const name = `consent-${item.key}`
              return (
                <li
                  key={item.key}
                  className="grid gap-2 rounded-lg border p-3"
                  aria-labelledby={`${name}-title`}
                >
                  <div className="flex flex-wrap items-center gap-2">
                    {item.import && <KeyRound className="size-4" aria-hidden />}
                    <span id={`${name}-title`} className="font-medium break-words">
                      {item.import
                        ? `Use your stored credential ${item.import.credential}`
                        : grant.title}
                    </span>
                    {tier && (
                      <Badge variant={grant.tier === "high" ? "destructive" : "outline"}>
                        {tier.label}
                      </Badge>
                    )}
                  </div>
                  <span className="font-mono text-xs text-muted-foreground">{item.key}</span>
                  <p className="text-xs">{item.why}</p>
                  {grant.detail && !item.import && (
                    <p className="text-xs text-muted-foreground">{grant.detail}</p>
                  )}
                  <ImportFacts item={item} />
                  {grant.justification && (
                    <p className="text-xs">
                      <span className="text-muted-foreground">Publisher's reason: </span>
                      {grant.justification}
                    </p>
                  )}
                  <div
                    role="radiogroup"
                    aria-label={`Decision: ${item.key}`}
                    className="flex flex-wrap gap-2"
                  >
                    {(["allow", "decline"] as const).map((value) => (
                      <Button
                        key={value}
                        type="button"
                        size="sm"
                        role="radio"
                        aria-checked={choice === value}
                        variant={choice === value ? "default" : "outline"}
                        className={cn(
                          choice === value && value === "decline" && "bg-destructive text-white",
                        )}
                        disabled={busy}
                        onClick={() => {
                          choose(item.key, value)
                        }}
                      >
                        {value === "allow" ? "Allow" : "Don't allow"}
                      </Button>
                    ))}
                  </div>
                </li>
              )
            })}
          </ul>
        )}
        {error && (
          <Alert variant="destructive">
            <AlertTitle className="line-clamp-none">{error.title}</AlertTitle>
            <AlertDescription className="grid gap-1">
              <p>{error.message}</p>
              {error.hint && <p>{error.hint}</p>}
            </AlertDescription>
          </Alert>
        )}
        <DialogFooter className="items-center gap-2 sm:justify-between">
          <p className="text-xs text-muted-foreground" aria-live="polite">
            {undecided === 0
              ? [...choices.values()].includes("decline")
                ? `${migration.moduleName} stays built-in; you can ask again later.`
                : "You allowed every item."
              : `${String(undecided)} of ${String(items.length)} items still need your decision.`}
          </p>
          <div className="flex flex-wrap gap-2">
            <Button variant="ghost" disabled={busy} onClick={onClose}>
              Not now
            </Button>
            <Button disabled={busy || items.length === 0 || undecided > 0} onClick={submit}>
              {busy ? "Working…" : "Confirm"}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
