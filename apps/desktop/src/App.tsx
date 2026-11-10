import { useCallback, useMemo, useState } from "react"

import { AppSidebar, type PageId } from "@/components/app-sidebar"
import { ErrorNotice, Loading } from "@/components/page"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { AppContext, type AppContextValue } from "@/lib/app-context"
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

  const refresh = useCallback(() => {
    setRevision((value) => value + 1)
  }, [])

  const context = useMemo<AppContextValue | null>(
    () => (state.status === "ready" ? { profiles: state.data, refresh, revision } : null),
    [state, refresh, revision],
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
