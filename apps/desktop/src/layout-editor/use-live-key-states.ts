import { listen } from "@tauri-apps/api/event"
import { useEffect, useMemo, useState } from "react"

import { i18n } from "@/i18n"
import { api } from "@/lib/api"
import type { AgentConnection, KeyStateEntry, KeyStatesEvent, LivePreview } from "@/lib/types"

/** The backend's event with a {@link KeyStatesEvent} (commands/key_states.rs). */
export const KEY_STATES_EVENT = "key-states"

/**
 * Where the preview's live values stand:
 *
 * - `off`: the page has no actions to watch.
 * - `connecting`: asked the agent, no answer yet.
 * - `live`: the agent reports the states of this page's keys.
 * - `inactiveProfile`: only the active profile's keys have live states.
 * - `stopped`: the agent is not running.
 * - `disconnected`: the agent went away; the last values are stale.
 * - `unsupported`: the agent predates live key states (protocol 11).
 */
export type LiveStatus =
  "off" | "connecting" | "live" | "inactiveProfile" | "stopped" | "disconnected" | "unsupported"

const NONE: KeyStateEntry[] = []

/** The states of a new answer merged into the known ones, by action. */
export function mergeStates(known: KeyStateEntry[], changed: KeyStateEntry[]): KeyStateEntry[] {
  const byAction = new Map(known.map((entry) => [entry.action, entry]))
  for (const entry of changed) byAction.set(entry.action, entry)
  return [...byAction.values()]
}

/**
 * What the preview is drawn with: the current states, the last known ones
 * marked stale while the agent is away, or nothing (no live values).
 */
export function livePreview(status: LiveStatus, states: KeyStateEntry[]): LivePreview | null {
  if (states.length === 0) return null
  if (status === "live") return { connected: true, states }
  if (status === "disconnected" || status === "stopped") return { connected: false, states }
  return null
}

/**
 * The live states of `actions`' keys (the actions on the page the editor
 * shows), from the agent: the same states the device's keys show, read by
 * the agent's providers and never here. One watch at a time; it is set up
 * again when the page, the profile, the agent's active profile or project
 * change or the agent comes back, and stopped when the editor goes away.
 * Nothing is asked while there is nothing to watch or no agent.
 */
export function useLiveKeyStates(
  profileId: string,
  actions: string[],
  agent: AgentConnection | null,
): { live: LivePreview | null; status: LiveStatus } {
  const running = agent?.running === true
  const supported = agent?.capabilities?.includes("keyStates") === true
  const activeProfile = agent?.status?.activeProfile?.id ?? null
  const activeProject = agent?.status?.activeProject?.id ?? null
  // Order does not matter, and an unchanged set does not watch again.
  const watched = useMemo(() => [...new Set(actions)].sort(), [actions])
  const watchKey = watched.join(",")
  const [state, setState] = useState<{
    key: string
    /** The actions the states are of. */
    watchKey: string
    status: LiveStatus
    states: KeyStateEntry[]
  }>({ key: "", watchKey: "", status: "off", states: NONE })

  const watching = watched.length > 0 && running && supported
  const key = [profileId, watchKey, activeProfile, activeProject].join("|")
  useEffect(() => {
    if (!watching) return
    let active = true
    let watch: number | null = null
    // Events can arrive before the watch's number does.
    const early: KeyStatesEvent[] = []
    const update = (next: (known: KeyStateEntry[]) => Pick<typeof state, "status" | "states">) => {
      setState((current) => ({
        key,
        watchKey,
        ...next(current.watchKey === watchKey ? current.states : NONE),
      }))
    }
    const apply = (event: KeyStatesEvent) => {
      switch (event.kind) {
        case "snapshot":
          update(() => ({
            status: event.live ? "live" : "inactiveProfile",
            states: event.live ? event.states : NONE,
          }))
          break
        case "changed":
          update((known) => ({ status: "live", states: mergeStates(known, event.states) }))
          break
        case "disconnected":
          update((known) => ({ status: "disconnected", states: known }))
          break
        case "unsupported":
          update(() => ({ status: "unsupported", states: NONE }))
          break
      }
    }
    const stop = listen<KeyStatesEvent>(KEY_STATES_EVENT, ({ payload }) => {
      if (!active) return
      if (watch === null) early.push(payload)
      else if (payload.watch === watch) apply(payload)
    })
    void stop.then(() => {
      if (!active) return
      api.watchKeyStates(profileId, watched).then(
        (number) => {
          if (!active) return
          watch = number
          for (const event of early.splice(0)) if (event.watch === number) apply(event)
        },
        () => {
          if (active) update((known) => ({ status: "disconnected", states: known }))
        },
      )
    })
    return () => {
      active = false
      void stop.then((unlisten) => {
        unlisten()
      })
      // Ends the agent's watch: no states are read for this page any more.
      api.watchKeyStates(profileId, []).catch(() => undefined)
    }
    // `key` and `watching` cover every input of the watch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, watching])

  // What was known of these actions stays, marked stale, while the agent
  // is away.
  const known = state.watchKey === watchKey ? state.states : NONE
  let status: LiveStatus
  let states = known
  if (watched.length === 0) status = "off"
  // Before the first status the agent is not known to be stopped.
  else if (!running) status = agent ? "stopped" : "connecting"
  else if (!supported) {
    status = "unsupported"
    states = NONE
  } else if (state.key === key) status = state.status
  else status = "connecting"
  // The same object while nothing changed, so the preview is not drawn again.
  const live = useMemo(() => livePreview(status, states), [status, states])
  return { live, status }
}

/** What the preview's live values are, in words; `null`: nothing to say. */
export function liveValuesText(status: LiveStatus, live: LivePreview | null): string | null {
  switch (status) {
    case "off":
      return null
    case "connecting":
      return null
    case "live":
      return live ? i18n.t("editor:liveStates.live") : null
    case "inactiveProfile":
      return i18n.t("editor:liveStates.inactiveProfile")
    case "stopped":
      return live ? i18n.t("editor:liveStates.stoppedStale") : i18n.t("editor:liveStates.stopped")
    case "disconnected":
      return i18n.t("editor:liveStates.disconnected")
    case "unsupported":
      return i18n.t("editor:liveStates.unsupported")
  }
}
