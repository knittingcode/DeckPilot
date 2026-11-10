import { listen } from "@tauri-apps/api/event"
import { Suspense, lazy, useCallback, useEffect, useMemo, useState } from "react"

import { AppSidebar } from "@/components/app-sidebar"
import { SettingsSidebar } from "@/components/settings-sidebar"
import { SecurityGate } from "@/components/security-gate"
import { ErrorNotice, Loading } from "@/components/page"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useAsync } from "@/hooks/use-async"
import { useNavigation } from "@/hooks/use-navigation"
import {
  AGENT_STATUS_EVENT,
  CONFIGURATION_CHANGED_EVENT,
  NAVIGATE_EVENT,
  UPDATE_STATE_EVENT,
  api,
} from "@/lib/api"
import { AppContext, type AppContextValue, type CorePageId, type PageId } from "@/lib/app-context"
import { ThemeProvider } from "@/components/theme-provider"
import { type Location, locationOf } from "@/lib/navigation"
import type { AgentConnection, ModuleInfo, UpdateState } from "@/lib/types"
import { MODULE_VIEWS, isModulePageAvailable } from "@/modules"
import { SettingsView } from "@/settings/settings-view"
import { findSettingsPage } from "@/settings/registry"

// Pages load on first use, which keeps the start-up bundle small. Pages of
// feature modules come from the module registry.
const PAGES: Record<CorePageId, React.LazyExoticComponent<() => React.JSX.Element>> = {
  dashboard: lazy(() => import("@/pages/dashboard").then((m) => ({ default: m.DashboardPage }))),
  actions: lazy(() => import("@/pages/actions").then((m) => ({ default: m.ActionsPage }))),
  workflows: lazy(() => import("@/pages/workflows").then((m) => ({ default: m.WorkflowsPage }))),
  profiles: lazy(() => import("@/pages/profiles").then((m) => ({ default: m.ProfilesPage }))),
  models: lazy(() => import("@/pages/models").then((m) => ({ default: m.ModelsPage }))),
  designer: lazy(() =>
    import("@/pages/layout-designer").then((m) => ({ default: m.LayoutDesignerPage })),
  ),
}

const EDITING_KEY = "deckpilot.editingProfile"

function storedEditingProfile(): string | null {
  try {
    return localStorage.getItem(EDITING_KEY)
  } catch {
    return null
  }
}

function Studio() {
  const [revision, setRevision] = useState(0)
  const [agent, setAgent] = useState<AgentConnection | null>(null)
  const [updates, setUpdates] = useState<UpdateState | null>(null)
  const [editing, setEditing] = useState<string | null>(storedEditingProfile)
  const [modules, setModules] = useState<ModuleInfo[] | null>(null)
  const { state } = useAsync(() => api.listProfiles(), String(revision))

  // Which locations exist depends on the enabled modules.
  const exists = useMemo(
    () =>
      modules === null
        ? null
        : (location: Location) =>
            location.mode === "settings"
              ? Boolean(findSettingsPage(modules, location.page))
              : Object.hasOwn(PAGES, location.page) ||
                isModulePageAvailable(modules, location.page),
    [modules],
  )
  const navigation = useNavigation(exists)
  const { go } = navigation

  useEffect(() => {
    api.listModules().then(setModules, () => {
      setModules([])
    })
  }, [])

  const refresh = useCallback(() => {
    setRevision((value) => value + 1)
  }, [])

  const setEditingProfileId = useCallback((id: string) => {
    setEditing(id)
    try {
      localStorage.setItem(EDITING_KEY, id)
    } catch {
      // Remembered for this session only.
    }
  }, [])

  // Agent state changes (button presses, hotplug, tray) and update progress
  // arrive as backend events.
  useEffect(() => {
    api.agentStatus().then(setAgent, () => {
      setAgent({
        running: false,
        status: null,
        canStart: false,
        compatibility: null,
        appVersion: "",
      })
    })
    api.updateState().then(setUpdates, () => undefined)
    const unlistenAgent = listen<AgentConnection>(AGENT_STATUS_EVENT, (event) => {
      setAgent(event.payload)
      refresh()
    })
    const unlistenUpdates = listen<UpdateState>(UPDATE_STATE_EVENT, (event) => {
      setUpdates(event.payload)
    })
    // An import in its own window changed profiles and module settings.
    const unlistenChanges = listen(CONFIGURATION_CHANGED_EVENT, () => {
      api.listModules().then(setModules, () => undefined)
      refresh()
    })
    // Other windows ask for a page: `projects/projects`, `settings:security`.
    const unlistenNavigate = listen<string>(NAVIGATE_EVENT, (event) => {
      go(locationOf(event.payload))
    })
    return () => {
      void unlistenChanges.then((stop) => {
        stop()
      })
      void unlistenNavigate.then((stop) => {
        stop()
      })
      void unlistenAgent.then((stop) => {
        stop()
      })
      void unlistenUpdates.then((stop) => {
        stop()
      })
    }
  }, [refresh, go])

  const context = useMemo<AppContextValue | null>(() => {
    if (state.status !== "ready" || modules === null) return null
    const profiles = state.data
    const valid = profiles.profiles.some((p) => p.id === editing)
    const editingProfileId = valid
      ? editing
      : (profiles.activeProfileId ?? profiles.profiles[0]?.id ?? null)
    return {
      profiles,
      editingProfileId,
      setEditingProfileId,
      refresh,
      revision,
      agent,
      setAgent,
      updates,
      setUpdates,
      navigate: navigation.navigate,
      openSettings: navigation.openSettings,
      closeSettings: navigation.closeSettings,
      history: {
        canGoBack: navigation.canGoBack,
        canGoForward: navigation.canGoForward,
        back: navigation.back,
        forward: navigation.forward,
      },
      modules,
      setModules,
    }
  }, [state, modules, editing, setEditingProfileId, refresh, revision, agent, updates, navigation])

  if (!context) {
    return (
      <div className="p-6">
        {state.status === "error" ? <ErrorNotice error={state.error} /> : <Loading />}
      </div>
    )
  }

  const { location } = navigation
  const page = location.page
  // A module page disappears when its module is disabled.
  const Current =
    page in PAGES
      ? PAGES[page as CorePageId]
      : isModulePageAvailable(context.modules, page)
        ? MODULE_VIEWS[page]
        : undefined
  const View = Current ?? PAGES.dashboard
  return (
    <AppContext value={context}>
      <SidebarProvider>
        {location.mode === "settings" ? (
          <SettingsSidebar current={page} />
        ) : (
          <AppSidebar current={page as PageId} onNavigate={context.navigate} />
        )}
        <SidebarInset>
          <Suspense
            fallback={
              <div className="p-6">
                <Loading />
              </div>
            }
          >
            {location.mode === "settings" ? (
              <SettingsView key={page} id={page} />
            ) : (
              // Remount on profile change: open editors never write to another profile.
              <View key={context.editingProfileId ?? "none"} />
            )}
          </Suspense>
        </SidebarInset>
      </SidebarProvider>
    </AppContext>
  )
}

export function App() {
  return (
    <ThemeProvider>
      <TooltipProvider>
        <SecurityGate>
          <Studio />
        </SecurityGate>
        <Toaster position="bottom-right" />
      </TooltipProvider>
    </ThemeProvider>
  )
}
