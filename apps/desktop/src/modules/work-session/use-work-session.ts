import { listen } from "@tauri-apps/api/event"
import { useCallback, useEffect, useState } from "react"

import { api } from "@/lib/api"
import { failureMessage } from "@/lib/failure"
import type { ElapsedFormat, WorkSessionSnapshot } from "@/lib/types"

import {
  WORK_SESSION_EVENT,
  isNotSupported,
  liveElapsedMs,
  nextTickMs,
  timerRunning,
} from "./model"

export type WorkSessionView =
  | { status: "loading" }
  /** The agent is older than protocol 8. */
  | { status: "unsupported" }
  | { status: "error"; error: string }
  | { status: "ready"; snapshot: WorkSessionSnapshot }

/**
 * The agent's work session: loaded once, then kept current by the
 * `agent-work-session` event. Nothing polls.
 */
export function useWorkSession() {
  const [view, setView] = useState<WorkSessionView>({ status: "loading" })
  const [generation, setGeneration] = useState(0)

  useEffect(() => {
    let cancelled = false
    api.getWorkSession().then(
      (snapshot) => {
        if (!cancelled) setView({ status: "ready", snapshot })
      },
      (error: unknown) => {
        if (cancelled) return
        setView(
          isNotSupported(error)
            ? { status: "unsupported" }
            : {
                status: "error",
                error: failureMessage(
                  "The work session could not be loaded",
                  error,
                  "Check that the agent is running in Settings › Agent.",
                ),
              },
        )
      },
    )
    const stop = listen<WorkSessionSnapshot>(WORK_SESSION_EVENT, ({ payload }) => {
      if (!cancelled) setView({ status: "ready", snapshot: payload })
    })
    return () => {
      cancelled = true
      void stop.then((unlisten) => {
        unlisten()
      })
    }
  }, [generation])

  const reload = useCallback(() => {
    setGeneration((value) => value + 1)
  }, [])
  const replace = useCallback((snapshot: WorkSessionSnapshot) => {
    setView({ status: "ready", snapshot })
  }, [])
  return { view, reload, replace }
}

/**
 * Elapsed time of `snapshot`, ticking locally while the timer runs: once per
 * minute boundary with `h:mm`, once per second with `h:mm:ss`.
 */
export function useLiveElapsed(snapshot: WorkSessionSnapshot, format: ElapsedFormat): number {
  const [now, setNow] = useState(() => Date.now())
  const running = timerRunning(snapshot.state)
  const elapsed = liveElapsedMs(snapshot, now)
  useEffect(() => {
    if (!running) return
    const timer = setTimeout(
      () => {
        setNow(Date.now())
      },
      nextTickMs(elapsed, format),
    )
    return () => {
      clearTimeout(timer)
    }
  }, [running, elapsed, format])
  return elapsed
}
