import { RefreshCw } from "lucide-react"

import { EmptyState } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"

import type { MarketplaceActions } from "./marketplace"
import type { UpdateCandidate } from "./model"

/**
 * Newer versions of installed plugins. The agent compares permissions: an
 * update that asks for more is staged and applied only after consent.
 */
export function UpdatesView({
  updates,
  actions,
}: {
  updates: UpdateCandidate[]
  actions: MarketplaceActions
}) {
  const { t } = useTranslation()
  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm text-muted-foreground">{t("marketplace:updates.intro")}</p>
        <Button
          variant="outline"
          size="sm"
          disabled={actions.busy !== null}
          onClick={actions.refresh}
        >
          <RefreshCw /> {t("marketplace:updates.check")}
        </Button>
      </div>
      {actions.blocked && <p className="text-sm text-muted-foreground">{actions.blocked}</p>}
      {updates.length === 0 ? (
        <EmptyState variant="plain" title={t("marketplace:updates.none")} />
      ) : (
        <ul className="grid divide-y border-y" aria-label={t("marketplace:updates.list")}>
          {updates.map((update) => {
            const { summary } = update.plugin
            const to = update.kind === "staged" ? update.pending.version : update.version
            const more =
              update.kind === "staged"
                ? update.pending.changes.added.length + update.pending.changes.broadened.length
                : 0
            return (
              <li
                key={summary.id}
                className="flex flex-wrap items-center gap-3 py-3"
                aria-label={t("marketplace:updates.item", {
                  name: summary.name,
                  from: summary.version,
                  to,
                })}
              >
                <div className="grid min-w-0 flex-1 gap-1">
                  <span className="text-sm font-medium">{summary.name}</span>
                  <span className="text-xs text-muted-foreground">
                    {summary.version} → {to}
                  </span>
                </div>
                {update.kind === "staged" && (
                  <Badge variant="outline">
                    {t("marketplace:updates.waiting", { count: more })}
                  </Badge>
                )}
                <Button
                  size="sm"
                  disabled={actions.busy !== null || actions.blocked !== null}
                  onClick={() => {
                    actions.update(update)
                  }}
                >
                  {update.kind === "staged"
                    ? t("marketplace:updates.review")
                    : t("marketplace:updates.update")}
                </Button>
              </li>
            )
          })}
        </ul>
      )}
    </>
  )
}
