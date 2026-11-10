import { useCallback, useEffect, useEffectEvent, useState } from "react"

import { errorMessage } from "@/lib/api"

export type AsyncState<T> =
  { status: "loading" } | { status: "error"; error: string } | { status: "ready"; data: T }

/**
 * Runs `load` on mount and whenever `key` changes; `reload` re-runs it on
 * demand. Previous data stays visible while a reload is in flight.
 */
export function useAsync<T>(load: () => Promise<T>, key: string) {
  const [state, setState] = useState<AsyncState<T>>({ status: "loading" })
  const [generation, setGeneration] = useState(0)
  const run = useEffectEvent(load)

  useEffect(() => {
    let cancelled = false
    run().then(
      (data) => {
        if (!cancelled) setState({ status: "ready", data })
      },
      (error: unknown) => {
        if (!cancelled) setState({ status: "error", error: errorMessage(error) })
      },
    )
    return () => {
      cancelled = true
    }
  }, [key, generation])

  const reload = useCallback(() => {
    setGeneration((value) => value + 1)
  }, [])

  return { state, reload }
}
