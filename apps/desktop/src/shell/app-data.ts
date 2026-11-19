import { listen } from "@tauri-apps/api/event"
import { useCallback, useEffect, useRef, useState } from "react"

import { type AsyncState, useAsync } from "@/hooks/use-async"
import { AGENT_STATUS_EVENT, CONFIGURATION_CHANGED_EVENT, UPDATE_STATE_EVENT, api } from "@/lib/api"
import type { AgentConnection, ModuleInfo, ProfilesView, UpdateState } from "@/lib/types"
import {
  DATA_CHANGED_EVENT,
  EDITING_PROFILE_EVENT,
  announce,
  onAnnouncement,
} from "@/lib/window-sync"
import { usePluginModules } from "@/plugins/use-plugin-modules"

const EDITING_KEY = "deckpilot.editingProfile"

function storedEditingProfile(): string | null {
  try {
    return localStorage.getItem(EDITING_KEY)
  } catch {
    return null
  }
}

function storeEditingProfile(id: string) {
  try {
    localStorage.setItem(EDITING_KEY, id)
  } catch {
    // Remembered for this session only.
  }
}

const STOPPED: AgentConnection = {
  running: false,
  status: null,
  canStart: false,
  compatibility: null,
  appVersion: "",
}

/** What the backend events this hook listens to carry. */
interface BackendEvents {
  [AGENT_STATUS_EVENT]: AgentConnection
  [UPDATE_STATE_EVENT]: UpdateState
  [CONFIGURATION_CHANGED_EVENT]: unknown
}

/** Subscribes to a backend event; the subscription ends with the effect. */
function useBackendEvent<K extends keyof BackendEvents>(
  name: K,
  handler: (payload: BackendEvents[K]) => void,
) {
  useEffect(() => {
    let stopped = false
    let stop: (() => void) | undefined
    listen<BackendEvents[K]>(name, (event) => {
      handler(event.payload)
    }).then(
      (unlisten) => {
        if (stopped) unlisten()
        else stop = unlisten
      },
      () => undefined,
    )
    return () => {
      stopped = true
      stop?.()
    }
  }, [name, handler])
}

export interface AppData {
  profiles: AsyncState<ProfilesView>
  /** The stored choice; the shell resolves it against the profiles. */
  editing: string | null
  setEditingProfileId: (id: string) => void
  refresh: () => void
  revision: number
  agent: AgentConnection | null
  setAgent: (agent: AgentConnection) => void
  updates: UpdateState | null
  setUpdates: (state: UpdateState) => void
  modules: ModuleInfo[] | null
  setModules: (modules: ModuleInfo[]) => void
  pluginModules: ModuleInfo[]
}

/**
 * The state every workspace window shows (the main window and the
 * settings window): profiles, modules, the agent, updates and the profile
 * being edited. Backend events reach every window; changes a window makes
 * itself are announced to the others (`lib/window-sync.ts`), so a module
 * enabled or a profile created in the settings shows in the main window at
 * once.
 *
 * `acceptEditing` decides whether another window's choice of the profile
 * to edit is taken (the main window refuses it while a layout has unsaved
 * changes); a refused choice is answered with this window's own.
 */
export function useAppData({
  acceptEditing = () => true,
}: { acceptEditing?: () => boolean } = {}): AppData {
  const [revision, setRevision] = useState(0)
  const [agent, setAgent] = useState<AgentConnection | null>(null)
  const [updates, setUpdates] = useState<UpdateState | null>(null)
  const [editing, setEditing] = useState<string | null>(storedEditingProfile)
  const [modules, setModulesState] = useState<ModuleInfo[] | null>(null)
  const { state: profiles } = useAsync(() => api.listProfiles(), String(revision))
  // Installed plugins for the action editors; always empty (and never
  // asked for) in builds without VITE_DECKPILOT_PLUGINS=1.
  const agentKey = agent
    ? `${String(agent.running)}:${agent.status?.version ?? ""}:${String(agent.protocol)}`
    : "unknown"
  const pluginModules = usePluginModules(agentKey)

  const reloadModules = useCallback(() => {
    api.listModules().then(setModulesState, () => undefined)
  }, [])

  useEffect(() => {
    api.listModules().then(setModulesState, () => {
      setModulesState([])
    })
    api.agentStatus().then(setAgent, () => {
      setAgent(STOPPED)
    })
    api.updateState().then(setUpdates, () => undefined)
  }, [])

  const bump = useCallback(() => {
    setRevision((value) => value + 1)
  }, [])

  const refresh = useCallback(() => {
    bump()
    announce(DATA_CHANGED_EVENT, {})
  }, [bump])

  const setModules = useCallback((next: ModuleInfo[]) => {
    setModulesState(next)
    announce(DATA_CHANGED_EVENT, { modules: true })
  }, [])

  const setEditingProfileId = useCallback((id: string) => {
    setEditing(id)
    storeEditingProfile(id)
    announce(EDITING_PROFILE_EVENT, { id })
  }, [])

  // Agent state changes (button presses, hotplug, tray) and update progress
  // arrive as backend events.
  const onAgent = useCallback(
    (payload: AgentConnection) => {
      setAgent(payload)
      bump()
    },
    [bump],
  )
  useBackendEvent(AGENT_STATUS_EVENT, onAgent)
  useBackendEvent(UPDATE_STATE_EVENT, setUpdates)
  // An import in its own window changed profiles and module settings.
  const onConfiguration = useCallback(() => {
    reloadModules()
    bump()
  }, [reloadModules, bump])
  useBackendEvent(CONFIGURATION_CHANGED_EVENT, onConfiguration)

  // What the other windows changed.
  const accept = useRef(acceptEditing)
  const own = useRef(editing)
  useEffect(() => {
    accept.current = acceptEditing
    own.current = editing
  })
  useEffect(() => {
    const stopData = onAnnouncement(DATA_CHANGED_EVENT, (payload) => {
      if (payload.modules) reloadModules()
      bump()
    })
    const stopEditing = onAnnouncement(EDITING_PROFILE_EVENT, ({ id }) => {
      if (accept.current()) {
        setEditing(id)
        return
      }
      // Refused: the other window follows this one again.
      if (own.current) {
        announce(EDITING_PROFILE_EVENT, { id: own.current })
      }
    })
    return () => {
      stopData()
      stopEditing()
    }
  }, [reloadModules, bump])

  return {
    profiles,
    editing,
    setEditingProfileId,
    refresh,
    revision,
    agent,
    setAgent,
    updates,
    setUpdates,
    modules,
    setModules,
    pluginModules,
  }
}

/** The profile shown in the editors: the stored choice if it still exists. */
export function resolveEditingProfile(profiles: ProfilesView, editing: string | null) {
  return profiles.profiles.some((p) => p.id === editing)
    ? editing
    : (profiles.activeProfileId ?? profiles.profiles[0]?.id ?? null)
}
