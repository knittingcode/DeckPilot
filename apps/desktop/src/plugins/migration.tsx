// The module migration notice (experimental; builds with
// VITE_DECKPILOT_PLUGINS=1, docs/marketplace/module-migration.md §5.5-5.7).
//
// Built-in modules move to their first-party plugins one by one. The agent
// decides everything (`listModuleMigrations`); this notice says, per
// module, whether it is moving to a plugin, waiting for the user's consent
// or staying built-in, and why, with Retry for a failed migration and a
// review of what waits for consent. It never shows a value: settings by
// key, credentials by id.

import { ArrowRightLeft, RotateCcw, ShieldQuestion } from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"

import { isDeclined, migrationStatus, type MigrationTone } from "./migration-status"
import type { ModuleMigration } from "./types"

const BADGE: Record<MigrationTone, "default" | "secondary" | "outline" | "destructive"> = {
  moving: "secondary",
  waiting: "default",
  builtin: "outline",
  moved: "secondary",
  unavailable: "destructive",
}

/**
 * Settings › Extensions › Plugins: the built-in modules that move to
 * plugins and where each stands. Nothing renders while no module moves.
 */
export function MigrationNotice({
  migrations,
  busy,
  onReview,
  onRetry,
}: {
  migrations: ModuleMigration[]
  /** The module with a request in flight. */
  busy: string | null
  /** Opens the consent dialog of a migration that waits for it. */
  onReview: (migration: ModuleMigration) => void
  onRetry: (migration: ModuleMigration) => void
}) {
  const { t } = useTranslation()
  const format = useFormat()
  if (migrations.length === 0) return null
  const sorted = [...migrations].sort((a, b) => format.compare(a.moduleName, b.moduleName))
  return (
    <section aria-labelledby="module-migrations-title" className="grid gap-3">
      <div className="flex items-start gap-2">
        <ArrowRightLeft className="mt-0.5 size-4 shrink-0" aria-hidden />
        <div className="grid gap-0.5">
          <h3 id="module-migrations-title" className="text-sm font-medium">
            {t("plugins:migration.notice.title")}
          </h3>
          <p className="text-xs text-muted-foreground">{t("plugins:migration.notice.text")}</p>
        </div>
      </div>
      <ul className="grid divide-y border-y" aria-label={t("plugins:migration.notice.list")}>
        {sorted.map((m) => {
          const status = migrationStatus(m)
          const id = `migration-${m.module}`
          const declined = isDeclined(m)
          return (
            <li key={m.module} aria-labelledby={`${id}-name`} className="grid gap-1.5 py-2.5">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <span id={`${id}-name`} className="font-medium">
                  {m.moduleName}
                </span>
                <span className="font-mono text-xs text-muted-foreground">{m.plugin}</span>
                <Badge variant={BADGE[status.tone]}>{status.label}</Badge>
                {m.attempts > 0 && (
                  <span className="text-xs text-muted-foreground">
                    {t("plugins:migration.notice.attempts", {
                      attempts: m.attempts,
                      count: m.maxAttempts,
                    })}
                  </span>
                )}
              </div>
              <p className="text-sm">{status.text}</p>
              {m.reason && (
                <p className="text-xs text-muted-foreground">
                  <span className="font-medium">{t("plugins:migration.notice.reason")} </span>
                  {m.reason}
                </p>
              )}
              {(m.phase === "awaitingConsent" || m.canRetry) && (
                <div className="flex flex-wrap gap-2 pt-1">
                  {m.phase === "awaitingConsent" && (
                    <Button
                      size="sm"
                      disabled={busy !== null}
                      onClick={() => {
                        onReview(m)
                      }}
                    >
                      <ShieldQuestion /> {t("plugins:migration.notice.review")}
                    </Button>
                  )}
                  {m.canRetry && (
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={busy !== null}
                      aria-label={t(
                        declined
                          ? "plugins:migration.notice.askAgainNamed"
                          : "plugins:migration.notice.retryNamed",
                        { module: m.moduleName },
                      )}
                      onClick={() => {
                        onRetry(m)
                      }}
                    >
                      <RotateCcw />{" "}
                      {busy === m.module
                        ? t("common:status.working")
                        : declined
                          ? t("plugins:migration.notice.askAgain")
                          : t("plugins:migration.notice.retry")}
                    </Button>
                  )}
                </div>
              )}
            </li>
          )
        })}
      </ul>
    </section>
  )
}
