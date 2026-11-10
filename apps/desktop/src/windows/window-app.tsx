import { Suspense, useEffect, useMemo, useRef, useState } from "react"

import { ErrorNotice, Loading } from "@/components/page"
import { ThemeProvider } from "@/components/theme-provider"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { api, errorMessage } from "@/lib/api"
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
        setError(errorMessage(e))
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

/** Reports the content's natural size whenever it changes. */
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
        return root ? naturalSize(root) : null
      },
      (size) => {
        api.fitWindow(size.width, size.height).catch(() => undefined)
      },
    )
    const resize = new ResizeObserver(reporter.schedule)
    const observeAll = () => {
      resize.disconnect()
      resize.observe(element)
      const root = target()
      if (root) {
        resize.observe(root)
        root
          .querySelectorAll<HTMLElement>("[data-modal-part], [data-modal-part=body] > *")
          .forEach((part) => {
            resize.observe(part)
          })
      }
      reporter.schedule()
    }
    // Content appears and changes (errors, lazily loaded views, wrapped
    // translations): re-observe and measure.
    const mutations = new MutationObserver(observeAll)
    mutations.observe(element, { childList: true, subtree: true, characterData: true })
    observeAll()
    return () => {
      mutations.disconnect()
      resize.disconnect()
      reporter.dispose()
    }
  }, [])
  return (
    <div ref={container} className={wide ? "h-svh w-full" : "w-max"}>
      {children}
    </div>
  )
}
