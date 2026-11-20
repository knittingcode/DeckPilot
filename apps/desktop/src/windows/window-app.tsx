import { Suspense, useEffect, useMemo, useRef, useState } from "react"

import { ErrorNotice, Loading } from "@/components/page"
import { DialogProvider } from "@/components/dialogs/dialog-service"
import { ThemeProvider } from "@/components/theme-provider"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { api } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import type { WindowSpec } from "@/lib/types"
import { coalescedReporter, naturalSize } from "@/lib/window-fit"
import { i18n, useTranslation } from "@/i18n"

import { windowView } from "./registry"
import { WindowContext, type WindowContextValue } from "./window-context"

/**
 * Root of a native window other than the main window. It renders the view
 * registered for the window's identifier and keeps the native window sized
 * to the content (see `src-tauri/src/windows.rs`).
 */
export function WindowApp({ id }: { id: string }) {
  const { t } = useTranslation()
  const [spec, setSpec] = useState<WindowSpec | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    api.currentWindow().then(
      (current) => {
        if (current?.id === id) setSpec(current)
        else setError(i18n.t("settings:window.notRegistered", { id }))
      },
      (e: unknown) => {
        setError(
          failureMessage(
            i18n.t("settings:window.loadFailed"),
            e,
            i18n.t("settings:window.loadFailedNext"),
          ),
        )
      },
    )
  }, [id])
  const View = windowView(id)
  return (
    <ThemeProvider>
      <TooltipProvider>
        <FitToContent wide={spec?.size === "large" || spec?.size === "fullscreen"}>
          {error || !View ? (
            <div className="w-[360px] p-6">
              <ErrorNotice error={error ?? t("settings:window.unknown", { id })} />
            </div>
          ) : spec ? (
            <DialogProvider>
              <Hosted spec={spec} View={View} />
            </DialogProvider>
          ) : (
            <div className="w-[360px] p-6">
              <Loading />
            </div>
          )}
        </FitToContent>
        <Toaster position="bottom-center" />
      </TooltipProvider>
    </ThemeProvider>
  )
}

function Hosted({
  spec,
  View,
}: {
  spec: WindowSpec
  View: NonNullable<ReturnType<typeof windowView>>
}) {
  const value = useMemo<WindowContextValue>(
    () => ({
      spec,
      close: () => {
        void api.closeWindow()
      },
    }),
    [spec],
  )
  return (
    <WindowContext value={value}>
      <Suspense fallback={<Loading />}>
        <View />
      </Suspense>
    </WindowContext>
  )
}

/**
 * Reports the shell's natural size together with the current viewport. The
 * window manager derives the space taken by native decorations (title bar,
 * borders) from the difference between the window size and the viewport,
 * so no platform-specific title bar height is assumed here.
 */
function FitToContent({ wide, children }: { wide: boolean; children: React.ReactNode }) {
  const container = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const element = container.current
    if (!element) return
    const target = () =>
      element.querySelector<HTMLElement>("[data-modal-root]") ??
      (element.firstElementChild as HTMLElement | null)
    const reporter = coalescedReporter(
      () => {
        const root = target()
        if (!root) return null
        return {
          ...naturalSize(root),
          viewportWidth: document.documentElement.clientWidth,
          viewportHeight: document.documentElement.clientHeight,
        }
      },
      (measurement) => {
        api
          .fitWindow(
            measurement.width,
            measurement.height,
            measurement.viewportWidth,
            measurement.viewportHeight,
          )
          .catch(() => undefined)
      },
    )
    // The root's size depends only on its content, so observing it catches
    // every change (errors appearing, lazily loaded views, wrapped text).
    const resize = new ResizeObserver(reporter.schedule)
    const observeRoot = () => {
      resize.disconnect()
      const root = target()
      if (root) resize.observe(root)
      reporter.schedule()
    }
    const mutations = new MutationObserver(observeRoot)
    mutations.observe(element, { childList: true, subtree: true })
    observeRoot()
    // A new viewport after a resize: measure again, so a title bar derived
    // from a viewport measured mid-resize is corrected (see window-fit.ts).
    window.addEventListener("resize", reporter.schedule)
    return () => {
      window.removeEventListener("resize", reporter.schedule)
      mutations.disconnect()
      resize.disconnect()
      reporter.dispose()
    }
  }, [])
  return (
    <div ref={container} className={wide ? "w-full" : "w-max"}>
      {children}
    </div>
  )
}
