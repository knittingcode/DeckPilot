import { listen } from "@tauri-apps/api/event"
import { LockKeyhole, TriangleAlert } from "lucide-react"
import { type ReactNode, useCallback, useEffect, useState } from "react"

import { ErrorNotice, Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
import { SECURITY_CHANGED_EVENT, api, errorMessage } from "@/lib/api"
import { SecurityContext } from "@/lib/security-context"
import { promptWindow } from "@/lib/security-prompts"
import type { SecurityView } from "@/lib/types"

/**
 * Keeps the main window closed until the configuration is unlocked. The
 * password prompts (first-run setup, unlock, encryption of an earlier
 * configuration, confirmation after an update) are compact native windows
 * of their own; this gate opens the right one and shows the main window
 * once a prompt reports success.
 */
export function SecurityGate({ children }: { children: ReactNode }) {
  const [view, setView] = useState<SecurityView | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(() => {
    api.securityStatus().then(setView, (e: unknown) => {
      setError(errorMessage(e))
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
    if (view.unlocked) {
      void api.showMainWindow()
    } else {
      api.openWindow(promptWindow(view)).catch((e: unknown) => {
        // Without a prompt the user must at least see why.
        setError(errorMessage(e))
      })
    }
  }, [view, unlocked])

  useEffect(() => {
    if (error) void api.showMainWindow()
  }, [error])

  if (error) {
    return (
      <Centered>
        <TriangleAlert className="size-6 text-destructive" aria-hidden />
        <h1 className="text-lg font-semibold">DeckPilot cannot open its configuration</h1>
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
    return <SecurityContext value={{ view, setView, refresh }}>{children}</SecurityContext>
  }
  // Only visible if the main window is shown while locked, e.g. after
  // "Lock now"; the prompt window is open as well.
  return (
    <Centered>
      <LockKeyhole className="size-6 text-muted-foreground" aria-hidden />
      <h1 className="text-lg font-semibold">DeckPilot is locked</h1>
      <Button
        onClick={() => {
          api.openWindow(promptWindow(view)).catch((e: unknown) => {
            setError(errorMessage(e))
          })
        }}
      >
        Unlock…
      </Button>
    </Centered>
  )
}

function Centered({ children }: { children: ReactNode }) {
  return (
    <main className="flex min-h-svh flex-col items-center justify-center gap-3 p-6 text-center">
      {children}
    </main>
  )
}
