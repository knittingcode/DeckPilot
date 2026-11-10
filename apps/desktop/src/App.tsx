import { listen } from "@tauri-apps/api/event"
import { useCallback, useEffect, useMemo, useState } from "react"

import { AppSidebar, type PageId } from "@/components/app-sidebar"
import { ErrorNotice, Loading } from "@/components/page"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useAsync } from "@/hooks/use-async"
import { AGENT_STATUS_EVENT, api } from "@/lib/api"
import { AppContext, type AppContextValue } from "@/lib/app-context"
import type { AgentConnection } from "@/lib/types"
import { ActionsPage } from "@/pages/actions"
import { DashboardPage } from "@/pages/dashboard"
import { DevicesPage } from "@/pages/devices"
import { ProfilesPage } from "@/pages/profiles"
import { ProjectsPage } from "@/pages/projects"
import { SettingsPage } from "@/pages/settings"

const PAGES: Record<PageId, () => React.JSX.Element> = {
  dashboard: DashboardPage,
  actions: ActionsPage,
  profiles: ProfilesPage,
  projects: ProjectsPage,
  devices: DevicesPage,
  settings: SettingsPage,
}

export function App() {
  const [page, setPage] = useState<PageId>("dashboard")
  const [revision, setRevision] = useState(0)
  const { state } = useAsync(() => api.listProfiles(), String(revision))

  const [agent, setAgent] = useState<AgentConnection | null>(null)

  const refresh = useCallback(() => {
    setRevision((value) => value + 1)
  }, [])

  // The backend forwards every agent state change (button presses, device
  // hotplug, profile switches from the tray); reload views when one arrives.
  useEffect(() => {
    api.agentStatus().then(setAgent, () => {
      setAgent({ running: false, status: null, canStart: false })
    })
    const unlisten = listen<AgentConnection>(AGENT_STATUS_EVENT, (event) => {
      setAgent(event.payload)
      refresh()
    })
    return () => {
      void unlisten.then((stop) => {
        stop()
      })
    }
  }, [refresh])

  const context = useMemo<AppContextValue | null>(
    () =>
      state.status === "ready"
        ? { profiles: state.data, refresh, revision, agent, setAgent }
        : null,
    [state, refresh, revision, agent],
  )

  if (!context) {
    return (
      <div className="p-6">
        {state.status === "error" ? <ErrorNotice error={state.error} /> : <Loading />}
      </div>
    )
  }

  const Current = PAGES[page]
  return (
    <AppContext value={context}>
      <TooltipProvider>
        <SidebarProvider>
          <AppSidebar current={page} onNavigate={setPage} />
          <SidebarInset>
            <Current />
          </SidebarInset>
        </SidebarProvider>
      </TooltipProvider>
    </AppContext>
  )
}
