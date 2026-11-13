import { Suspense, useEffect, useMemo, useRef, useState } from "react"

import { ErrorNotice, Loading } from "@/components/page"
import { ThemeProvider } from "@/components/theme-provider"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { api } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import type { WindowSpec } from "@/lib/types"
import { coalescedReporter, naturalSize } from "@/lib/window-fit"

import { windowView } from "./registry"
import { WindowContext, type WindowContextValue } from "./window-context"

/**
 * Root of a native window other than the main window. It renders the view
 * registered for the window's identifier and keeps the native window sized
 * to the content (see `src-tauri/src/windows.rs`).
 */
export function WindowApp({ id }: { id: string }) {
  const [spec, setSpec] = useState<WindowSpec | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    api.currentWindow().then(
      (current) => {
        if (current?.id === id) setSpec(current)
        else setError(`this window is not registered as ${id}`)
      },
      (e: unknown) => {
        setError(failureMessage("This window could not load", e, "Close it and try again"))
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
              <ErrorNotice error={error ?? `unknown window ${id}`} />
            </div>
          ) : spec ? (
            <Hosted spec={spec} View={View} />
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
    return () => {
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
