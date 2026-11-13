import { listen } from "@tauri-apps/api/event"
import { LockKeyhole, TriangleAlert } from "lucide-react"
import { type ReactNode, useCallback, useEffect, useState } from "react"

import { SensitiveOperations } from "@/components/authentication"
import { ErrorNotice, Loading } from "@/components/page"
import { Button } from "@/components/ui/button"
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
 */
export function SecurityGate({ children }: { children: ReactNode }) {
  const [view, setView] = useState<SecurityView | null>(null)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(() => {
    api.securityStatus().then(setView, (e: unknown) => {
      setError(failureMessage("The protection status could not be read", e, "Restart DeckPilot"))
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
        setError(failureMessage("The unlock window did not open", e, "Restart DeckPilot"))
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
      <LockKeyhole className="size-6 text-muted-foreground" aria-hidden />
      <h1 className="text-lg font-semibold">DeckPilot is locked</h1>
      <Button
        onClick={() => {
          api.openWindow(promptWindow(view)).catch((e: unknown) => {
            setError(failureMessage("The unlock window did not open", e, "Restart DeckPilot"))
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
