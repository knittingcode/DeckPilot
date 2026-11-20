import { listen } from "@tauri-apps/api/event"
import { LockKeyhole, TriangleAlert } from "lucide-react"
import { type ReactNode, useCallback, useEffect, useState } from "react"

import { SensitiveOperations } from "@/components/authentication"
import { ModalIcon } from "@/components/modal-shell"
import { ErrorNotice, Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { useWindowChrome, windowDragHandlers } from "@/hooks/use-window-chrome"
import { i18n, useTranslation } from "@/i18n"
import { SECURITY_CHANGED_EVENT, api } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import { SecurityContext } from "@/lib/security-context"
import { promptWindow } from "@/lib/security-prompts"
import type { SecurityView } from "@/lib/types"

/**
 * Keeps the main window closed until the configuration is open. With the
 * default system-managed key that happens without any prompt. Otherwise
 * the prompts (unlock, a credential store problem, the confirmation the
 * security policy asks for, encryption of an earlier configuration) are
 * compact native windows of their own; this gate opens the right one and
 * shows the main window once a prompt reports success. It also provides
 * the authentication of sensitive operations.
 *
 * In a `secondary` window (the settings) the gate opens no prompt: the
 * main window does. The window closes itself when the configuration is
 * locked.
 */
export function SecurityGate({
  secondary = false,
  children,
}: {
  secondary?: boolean
  children: ReactNode
}) {
  const { t } = useTranslation()
  const [view, setView] = useState<SecurityView | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(() => {
    api.securityStatus().then(setView, (e: unknown) => {
      setError(
        failureMessage(
          i18n.t("errors:security.statusUnreadable"),
          e,
          i18n.t("errors:next.restart"),
        ),
      )
    })
  }, [])

  useEffect(refresh, [refresh])
  useEffect(() => {
    const stop = listen(SECURITY_CHANGED_EVENT, refresh)
    return () => {
      void stop.then((unlisten) => {
        unlisten()
      })
    }
  }, [refresh])

  const unlocked = view?.unlocked ?? false
  useEffect(() => {
    if (!view) return
    if (secondary) {
      if (!view.unlocked) void api.closeWindow().catch(() => undefined)
      return
    }
    if (view.unlocked) {
      void api.showMainWindow()
    } else {
      api.openWindow(promptWindow(view)).catch((e: unknown) => {
        // Without a prompt the user must at least see why.
        setError(
          failureMessage(
            i18n.t("errors:security.unlockWindowFailed"),
            e,
            i18n.t("errors:next.restart"),
          ),
        )
      })
    }
  }, [view, unlocked, secondary])

  useEffect(() => {
    if (error && !secondary) void api.showMainWindow()
  }, [error, secondary])

  if (error) {
    return (
      <Centered>
        <TriangleAlert className="size-6 text-destructive" aria-hidden />
        <h1 className="text-lg font-semibold">{t("security:cannotOpen.title")}</h1>
        <ErrorNotice error={error} />
      </Centered>
    )
  }
  if (!view) {
    return (
      <div className="p-6">
        <Loading />
      </div>
    )
  }
  if (view.unlocked) {
    return (
      <SecurityContext value={{ view, setView, refresh }}>
        <SensitiveOperations verification={view.verification}>{children}</SensitiveOperations>
      </SecurityContext>
    )
  }
  // Only visible if the main window is shown while locked, e.g. after
  // "Lock now"; the prompt window is open as well.
  return (
    <Centered>
      <ModalIcon icon={LockKeyhole} compact />
      <div className="grid gap-1">
        <h1 className="text-base leading-tight font-semibold">{t("security:locked.title")}</h1>
        <p className="text-sm text-muted-foreground">{t("security:locked.resumes")}</p>
      </div>
      <Button
        className="mt-1"
        onClick={() => {
          api.openWindow(promptWindow(view)).catch((e: unknown) => {
            setError(
              failureMessage(t("errors:security.unlockWindowFailed"), e, t("errors:next.restart")),
            )
          })
        }}
      >
        {t("security:locked.unlock")}
      </Button>
    </Centered>
  )
}

/**
 * A full-window state without the app's content (locked, or the
 * configuration cannot be opened). Its top strip stands in for the
 * toolbar: on macOS the window has no native titlebar (`Overlay`), so the
 * strip keeps it movable like the toolbar does.
 */
function Centered({ children }: { children: ReactNode }) {
  const chrome = useWindowChrome()
  return (
    <main className="relative flex min-h-svh flex-col items-center justify-center gap-3 bg-background p-6 text-center">
      <div
        aria-hidden
        data-slot="window-drag-strip"
        className="absolute inset-x-0 top-0 h-(--titlebar-height)"
        {...windowDragHandlers(chrome)}
      />
      {children}
    </main>
  )
}
