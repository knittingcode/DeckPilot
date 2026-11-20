import { Bell, BellDot, CircleAlert, Info, TriangleAlert, X } from "lucide-react"
import { useState } from "react"

import { Button } from "@/components/ui/button"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { useSnapshot } from "@/hooks/use-status"
import { type TranslationKey, useTranslation } from "@/i18n"
import { formatTimestamp } from "@/lib/format"
import { NotificationStore, type Severity } from "@/lib/status"
import { cn } from "@/lib/utils"

const SEVERITY: Record<Severity, { icon: typeof Info; className: string }> = {
  info: { icon: Info, className: "text-muted-foreground" },
  warning: { icon: TriangleAlert, className: "text-amber-600 dark:text-amber-400" },
  error: { icon: CircleAlert, className: "text-destructive" },
}

const SEVERITY_LABEL = {
  info: "notifications:severity.info",
  warning: "notifications:severity.warning",
  error: "notifications:severity.error",
} as const satisfies Record<Severity, TranslationKey>

/**
 * The notifications of the session (workflow actions, the agent, updates)
 * behind a toolbar button with the number of unread ones. They count as
 * read once the list closes.
 */
export function NotificationsButton({
  store,
  className,
}: {
  store: NotificationStore
  className?: string
}) {
  const { t } = useTranslation()
  const notifications = useSnapshot(store)
  const [open, setOpen] = useState(false)
  const unread = notifications.filter((n) => !n.read)
  const worst = unread.some((n) => n.severity === "error")
    ? "error"
    : unread.some((n) => n.severity === "warning")
      ? "warning"
      : null
  const list = NotificationStore.newestFirst(notifications)
  const label =
    unread.length > 0
      ? t("notifications:center.unread", { count: unread.length })
      : t("notifications:center.title")
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        // Seen once the list closes.
        if (!next) store.markAllRead()
      }}
    >
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={cn("relative", className)}
          aria-label={label}
          title={label}
        >
          {unread.length > 0 ? (
            <BellDot className={worst ? SEVERITY[worst].className : undefined} />
          ) : (
            <Bell />
          )}
          {unread.length > 0 && (
            <span
              aria-hidden
              data-slot="notifications-badge"
              className="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[0.625rem] leading-none font-medium text-primary-foreground tabular-nums"
            >
              {unread.length > 99 ? "99+" : unread.length}
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        side="bottom"
        align="end"
        sideOffset={6}
        collisionPadding={8}
        className="w-96 p-0 text-sm"
      >
        <div className="flex items-center border-b px-3 py-2">
          <h2 className="flex-1 text-xs font-medium text-muted-foreground uppercase">
            {t("notifications:center.title")}
          </h2>
          {list.length > 0 && (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-xs"
              onClick={() => {
                store.clear()
              }}
            >
              <X /> {t("common:actions.clear")}
            </Button>
          )}
        </div>
        {list.length === 0 ? (
          <p className="px-3 py-6 text-center text-sm text-muted-foreground">
            {t("notifications:center.empty")}
          </p>
        ) : (
          <ul
            className="relative grid max-h-80 overflow-y-auto"
            aria-label={t("notifications:center.title")}
          >
            {list.map((n) => {
              const { icon: Icon, className: tone } = SEVERITY[n.severity]
              return (
                <li
                  key={n.key}
                  className={cn(
                    "flex gap-2 border-b px-3 py-2 last:border-b-0",
                    !n.read && "bg-accent/40",
                  )}
                >
                  <Icon
                    className={cn("mt-0.5 size-4 shrink-0", tone)}
                    aria-label={t(SEVERITY_LABEL[n.severity])}
                  />
                  <div className="grid min-w-0 flex-1 gap-0.5">
                    <span className="font-medium">
                      {n.title}
                      {n.count > 1 && (
                        <span className="ml-1 text-xs font-normal text-muted-foreground">
                          ×{n.count}
                        </span>
                      )}
                    </span>
                    {n.message && (
                      <span className="text-xs text-muted-foreground">{n.message}</span>
                    )}
                    <span className="text-xs text-muted-foreground">
                      {formatTimestamp(Math.floor(n.at / 1000))}
                    </span>
                  </div>
                </li>
              )
            })}
          </ul>
        )}
      </PopoverContent>
    </Popover>
  )
}
