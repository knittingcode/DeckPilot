import { ScrollText } from "lucide-react"
import { useCallback, useEffect, useRef, useState } from "react"

import { ModalShell } from "@/components/modal-shell"
import { api, errorMessage } from "@/lib/api"
import type { LogView } from "@/lib/types"

import { useWindow } from "./window-context"

const LINES = 500

/**
 * The end of the background agent's log, for troubleshooting. Logs never
 * contain configuration values, keys or passwords. Available while the
 * configuration is locked, e.g. when the system key cannot be read.
 */
export function LogsWindow() {
  const { close } = useWindow()
  const [view, setView] = useState<LogView | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const end = useRef<HTMLDivElement>(null)
  // State changes only in the callbacks, never synchronously in an effect.
  const fetchLog = useCallback(
    () =>
      api.readAgentLog(LINES).then(
        (next) => {
          setView(next)
          setError(null)
        },
        (e: unknown) => {
          setError(errorMessage(e))
        },
      ),
    [],
  )
  useEffect(() => {
    void fetchLog()
  }, [fetchLog])
  const load = () => {
    setBusy(true)
    void fetchLog().finally(() => {
      setBusy(false)
    })
  }
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" })
  }, [view])
  return (
    <ModalShell
      size="large"
      icon={ScrollText}
      title="Background agent log"
      description={
        view?.file ? (
          <span className="font-mono text-xs break-all select-text">{view.file}</span>
        ) : view ? (
          `No log in ${view.directory} yet.`
        ) : undefined
      }
      loading={!view && !error}
      error={error}
      onClose={close}
      secondary={{ label: "Close", onClick: close }}
      primary={{
        label: "Refresh",
        loadingLabel: "Loading…",
        loading: busy && Boolean(view),
        onClick: load,
      }}
    >
      {view && view.lines.length > 0 && (
        <div className="grid gap-1">
          {view.truncated && (
            <p className="text-xs text-muted-foreground">
              Showing the last {view.lines.length} lines.
            </p>
          )}
          <pre
            className="rounded-md bg-muted p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap select-text"
            aria-label="Log lines"
          >
            {view.lines.join("\n")}
            <div ref={end} />
          </pre>
        </div>
      )}
    </ModalShell>
  )
}
