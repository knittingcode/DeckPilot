// The device shows the editor's draft while the editor is open (IPC
// protocol 16, docs/editor-navigation-and-simulation.md, section 3): the
// agent keeps the unsaved layout in memory for the selected device, which
// shows and runs it; nothing is saved. Navigation goes both ways: the
// editor's page moves the device, and the device's own navigation (a key,
// Auto Exit) moves the editor.
//
// The session lives in the app's backend (`commands/live_preview.rs`): it
// sends only the latest draft and only what changed, and begins again
// after the agent restarted.

import { listen } from "@tauri-apps/api/event"
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react"

import { api } from "@/lib/api"
import type {
  AgentConnection,
  DeviceInfo,
  DeviceLayout,
  DeviceNavigation,
  LiveNavigation,
  LiveSyncEvent,
} from "@/lib/types"

/** The backend's event with a {@link LiveSyncEvent}. */
export const LIVE_PREVIEW_EVENT = "live-preview"
/** The IPC protocol that brought the live preview. */
export const LIVE_PREVIEW_PROTOCOL = 16
/** Edits are sent this long after the last one... */
export const LIVE_DEBOUNCE_MS = 40
/** ...but at least this often while they keep coming (a colour drag). */
export const LIVE_MAX_WAIT_MS = 120

/**
 * Where the live preview stands:
 *
 * - `off`: no device to preview on (none selected, or not connected).
 * - `otherProfile`: the device runs another profile; changes reach it when
 *   saved and that profile is active.
 * - `unsupported`: the agent predates the live preview; changes reach the
 *   device when saved.
 * - `stopped`: the agent is not running.
 * - `connecting`: asked the agent, no answer yet.
 * - `live`: the device shows the draft.
 * - `suspended`: the device shows another layout or profile for now.
 * - `disconnected`: the agent went away; it shows the saved configuration.
 * - `refused`: the agent refused the draft (e.g. too large); see `problem`.
 */
export type LiveStatus =
  | "off"
  | "otherProfile"
  | "unsupported"
  | "stopped"
  | "connecting"
  | "live"
  | "suspended"
  | "disconnected"
  | "refused"

export interface LivePreviewTarget {
  agent: AgentConnection | null
  device: DeviceInfo | null
  profileId: string
  layoutId: string
  /** The page the editor shows and the pages it came through. */
  pageId: string
  trail: string[]
  draft: DeviceLayout
  /**
   * The device went somewhere by itself (or shows something else when the
   * session starts): the editor follows it where it can.
   */
  onDevice: (navigation: DeviceNavigation) => void
}

/** Why the live preview cannot run, or `null` when it can be asked. */
export function liveEligibility(
  agent: AgentConnection | null,
  device: DeviceInfo | null,
  profileId: string,
): Exclude<LiveStatus, "connecting" | "live" | "suspended" | "disconnected" | "refused"> | null {
  if (!agent?.running) return "stopped"
  if (!device || device.state !== "connected") return "off"
  const protocol = agent.protocol ?? agent.status?.protocol
  const capable =
    agent.capabilities?.includes("livePreview") === true ||
    (agent.capabilities === undefined &&
      protocol !== undefined &&
      protocol >= LIVE_PREVIEW_PROTOCOL)
  if (!capable) return "unsupported"
  if ((agent.status?.activeProfile?.id ?? null) !== profileId) return "otherProfile"
  return null
}

function sameStack(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((page, i) => page === b[i])
}

/** The editor's place as a page stack. */
export function stackOf(pageId: string, trail: string[]): string[] {
  return [...trail, pageId]
}

/**
 * The live preview of the editor's draft on `device`. Returns its status
 * and `navigate`, for navigation the place sync cannot see (another
 * layout, Back to previous layout), and `flush`, which sends the draft at
 * once (before saving).
 */
export function useLivePreview(target: LivePreviewTarget) {
  const { agent, device, profileId, layoutId, pageId, trail, draft } = target
  const blocked = liveEligibility(agent, device, profileId)
  const deviceKey = device ? `${device.id.model}/${device.id.serial}` : ""
  const [state, setState] = useState<{
    key: string
    status: LiveStatus
    problem: string | null
    applied: number
  }>({ key: "", status: "connecting", problem: null, applied: 0 })
  const key = `${deviceKey}|${profileId}|${layoutId}`

  // What the effects share without re-running.
  const session = useRef<number | null>(null)
  const revision = useRef(0)
  const latest = useRef({ draft, pageId, trail, layoutId, onDevice: target.onDevice })
  // Before the effects below read it.
  useLayoutEffect(() => {
    latest.current = { draft, pageId, trail, layoutId, onDevice: target.onDevice }
  })
  // The place the device was last known at (reported or answered).
  const remote = useRef<string[] | null>(null)
  // A navigation of the editor not yet answered: device events before the
  // answer are older than it.
  const pending = useRef(0)
  const seq = useRef(0)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const since = useRef<number | null>(null)

  const sendDraft = useCallback(() => {
    if (timer.current) clearTimeout(timer.current)
    timer.current = null
    since.current = null
    const id = session.current
    if (id === null) return
    revision.current += 1
    void api
      .livePreviewUpdate(id, { revision: revision.current, layout: latest.current.draft })
      .catch(() => undefined)
  }, [])

  const navigate = useCallback(
    (navigation: LiveNavigation) => {
      const id = session.current
      if (id === null) return
      // The draft first: a page that exists only in it must reach the
      // agent before the device is sent there.
      if (timer.current) sendDraft()
      pending.current += 1
      if (navigation.type === "show") remote.current = navigation.stack
      void api.livePreviewUpdate(id, { navigation }).catch(() => {
        pending.current = Math.max(0, pending.current - 1)
      })
    },
    [sendDraft],
  )

  // The session: begun for the device, profile and layout, ended when any
  // of them changes or the editor goes away.
  useEffect(() => {
    if (blocked !== null || !device) return
    let active = true
    let id: number | null = null
    const early: LiveSyncEvent[] = []
    const set = (status: LiveStatus, problem: string | null = null) => {
      setState((s) => ({ ...s, key, status, problem }))
    }
    remote.current = null
    pending.current = 0
    seq.current = 0
    const follow = (navigation: DeviceNavigation) => {
      if (navigation.seq <= seq.current) return
      seq.current = navigation.seq
      remote.current = navigation.stack
      set(navigation.inScope && navigation.connected ? "live" : "suspended")
      latest.current.onDevice(navigation)
    }
    const handle = (event: LiveSyncEvent) => {
      switch (event.kind) {
        case "started": {
          seq.current = 0
          pending.current = 0
          set(event.navigation.inScope ? "live" : "suspended")
          if (event.resumed) {
            // The backend sent the latest draft and the place again.
            seq.current = event.navigation.seq
            return
          }
          // The whole draft once: what changed is sent from then on.
          sendDraft()
          const { navigation } = event
          const here = stackOf(latest.current.pageId, latest.current.trail)
          if (!navigation.inScope && navigation.profile === profileId) {
            // The editor chose this layout: the device follows.
            navigate({ type: "show", layout: latest.current.layoutId, stack: here })
          } else {
            follow(navigation)
          }
          return
        }
        case "applied":
          setState((s) => (s.key === key ? { ...s, applied: event.revision } : s))
          return
        case "navigated":
          if (event.own) {
            pending.current = Math.max(0, pending.current - 1)
            seq.current = Math.max(seq.current, event.navigation.seq)
            remote.current = event.navigation.stack
            set(event.navigation.inScope ? "live" : "suspended")
            return
          }
          if (pending.current > 0) return
          follow(event.navigation)
          return
        case "refused":
          pending.current = 0
          set("refused", event.message)
          return
        case "ended":
          set("connecting")
          return
        case "disconnected":
          set("disconnected")
          return
        case "unsupported":
          set("unsupported")
          return
      }
    }
    const unlisten = listen<LiveSyncEvent>(LIVE_PREVIEW_EVENT, ({ payload }) => {
      if (id === null) early.push(payload)
      else if (payload.session === id) handle(payload)
    })
    // Begun once the effect stays: an effect that is set up and cleaned up
    // at once (React's development double run, a quick re-render) begins
    // no session, so the agent sees one session per editor and device.
    const deviceId = device.id
    const begin = setTimeout(() => {
      api.livePreviewBegin(deviceId, profileId, layoutId).then(
        (number) => {
          if (!active) {
            void api.livePreviewEnd(number).catch(() => undefined)
            return
          }
          id = number
          session.current = number
          revision.current = 0
          for (const event of early.splice(0)) if (event.session === number) handle(event)
        },
        (error: unknown) => {
          if (active) set("refused", String(error))
        },
      )
    }, 0)
    return () => {
      active = false
      clearTimeout(begin)
      if (timer.current) clearTimeout(timer.current)
      timer.current = null
      if (id !== null) void api.livePreviewEnd(id).catch(() => undefined)
      session.current = null
      void unlisten.then((stop) => {
        stop()
      })
    }
    // The device by its id: a new status object is the same device.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [blocked, key, sendDraft, navigate, profileId, layoutId])

  // Edits: debounced, at least every LIVE_MAX_WAIT_MS while they keep coming.
  useEffect(() => {
    if (session.current === null) return
    const now = Date.now()
    since.current ??= now
    if (timer.current) clearTimeout(timer.current)
    const wait = Math.max(0, Math.min(LIVE_DEBOUNCE_MS, since.current + LIVE_MAX_WAIT_MS - now))
    timer.current = setTimeout(sendDraft, wait)
  }, [draft, sendDraft])

  // The editor's place: sent when the editor moved (not when it followed
  // the device).
  const place = stackOf(pageId, trail)
  const placeKey = place.join("/")
  useEffect(() => {
    if (session.current === null || remote.current === null) return
    if (sameStack(remote.current, place)) return
    navigate({ type: "show", layout: layoutId, stack: place })
    // `place` is its key.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [placeKey, layoutId, navigate])

  const current = state.key === key ? state : { status: "connecting" as const, problem: null }
  const status: LiveStatus = blocked ?? current.status
  return {
    status,
    problem: blocked === null ? current.problem : null,
    /** The device shows the editor's draft (and runs it). */
    live: status === "live",
    navigate,
    /** Sends a pending draft at once (before saving). */
    flush: () => {
      if (timer.current) sendDraft()
    },
  }
}
