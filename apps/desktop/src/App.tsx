import { listen } from "@tauri-apps/api/event"
import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react"
import { toast } from "sonner"

import { AppSidebar } from "@/components/app-sidebar"
import { AppTitlebar } from "@/components/app-titlebar"
import { AppStatusBar } from "@/components/status-bar"
import { ExecutionSync } from "@/components/execution-sync"
import { CoreStatusItems } from "@/components/status-sources"
import { SecurityGate } from "@/components/security-gate"
import { ErrorNotice, Loading } from "@/components/page"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useNavigation } from "@/hooks/use-navigation"
import { ExecutionsContext } from "@/hooks/use-executions"
import { StatusContext, createStatusStores } from "@/hooks/use-status"
import { TitlebarContext, useTitlebar } from "@/hooks/use-titlebar"
import { useWindowChrome } from "@/hooks/use-window-chrome"
import { NAVIGATE_EVENT, api } from "@/lib/api"
import { AppContext, type AppContextValue, type CorePageId, type PageId } from "@/lib/app-context"
import { failureMessage } from "@/lib/failure"
import { ThemeProvider } from "@/components/theme-provider"
import { type Location, browserStorage, locationOf } from "@/lib/navigation"
import { ExecutionStore } from "@/lib/executions"
import { TitlebarStore } from "@/lib/titlebar"
import { MODULE_VIEWS, isModulePageAvailable, moduleStatusItems } from "@/modules"
import { resolveEditingProfile, useAppData } from "@/shell/app-data"
import { EditorSelectionProvider } from "@/shell/editor-selection"
import { UNSAVED_WARNING } from "@/shell/editor-selection-context"
import { usePluginNavigation } from "@/shell/plugin-navigation"
import { DeviceSelector, ProfileSelector, ToolbarEnd } from "@/shell/toolbar"

type View = React.LazyExoticComponent<() => React.JSX.Element>

/** The build has the plugin marketplace (VITE_DECKPILOT_PLUGINS=1). */
const MARKETPLACE = import.meta.env.VITE_DECKPILOT_PLUGINS === "1"

// Pages load on first use, which keeps the start-up bundle small. Pages of
// feature modules come from the module registry. Profiles are in the
// settings window.
const PAGES: Partial<Record<CorePageId, View>> = {
  designer: lazy(() =>
    import("@/pages/layout-designer").then((m) => ({ default: m.LayoutDesignerPage })),
  ),
  dashboard: lazy(() => import("@/pages/dashboard").then((m) => ({ default: m.DashboardPage }))),
  actions: lazy(() => import("@/pages/actions").then((m) => ({ default: m.ActionsPage }))),
  workflows: lazy(() => import("@/pages/workflows").then((m) => ({ default: m.WorkflowsPage }))),
  ...(MARKETPLACE
    ? {
        marketplace: lazy(() =>
          import("@/pages/marketplace").then((m) => ({ default: m.MarketplacePage })),
        ),
      }
    : {}),
}

/** Titles of core pages before they register their own, e.g. while loading. */
const PAGE_TITLES: Partial<Record<CorePageId, string>> = {
  designer: "Device Editor",
  dashboard: "Dashboard",
  actions: "Action Library",
  workflows: "Workflows",
  marketplace: "Marketplace",
}

const DEFAULT_VIEW = PAGES.designer as View

/** Shown while a page loads; the titlebar shows progress meanwhile. */
function PageLoading() {
  useTitlebar({ progress: true })
  return (
    <div className="p-6">
      <Loading />
    </div>
  )
}

const RAIL_KEY = "deckpilot.rail"

function storedRail(): boolean {
  return browserStorage()?.getItem(RAIL_KEY) === "expanded"
}

/** Opens (or focuses) the settings window at `page`. */
function openSettingsWindow(page?: string) {
  api.openWindow("app.settings", page).catch((e: unknown) => {
    toast.error(failureMessage("The settings did not open", e, "Try again"))
  })
}

/**
 * The main window: the toolbar (device and profile, the page's context,
 * plugin entries, notifications, marketplace, settings), the workspace
 * rail, the current page (the device editor by default) and the status
 * bar. The settings are a window of their own.
 */
function Studio() {
  // Unsaved layout changes: shared by the editor, the toolbar's selectors
  // and the profile choices other windows announce.
  const dirtyRef = useRef(false)
  const acceptEditing = useCallback(() => {
    if (!dirtyRef.current) return true
    toast.warning(UNSAVED_WARNING)
    return false
  }, [dirtyRef])
  const data = useAppData({ acceptEditing })
  const { modules } = data

  // Which locations exist depends on the enabled modules.
  const exists = useMemo(
    () =>
      modules === null
        ? null
        : (location: Location) =>
            location.mode === "workspace" &&
            (Object.hasOwn(PAGES, location.page) || isModulePageAvailable(modules, location.page)),
    [modules],
  )
  const navigation = useNavigation(exists, openSettingsWindow)
  const [titlebar] = useState(() => new TitlebarStore())
  const [status] = useState(createStatusStores)
  const [executions] = useState(() => new ExecutionStore())
  const [rail, setRailState] = useState(storedRail)
  const chrome = useWindowChrome()
  const plugins = usePluginNavigation()
  const { go } = navigation

  const setRail = useCallback((open: boolean) => {
    setRailState(open)
    try {
      browserStorage()?.setItem(RAIL_KEY, open ? "expanded" : "collapsed")
    } catch {
      // Remembered for this session only.
    }
  }, [])

  // Other windows ask for a page: `projects/projects`, `settings:security`.
  useEffect(() => {
    let stopped = false
    let stop: (() => void) | undefined
    listen<string>(NAVIGATE_EVENT, (event) => {
      go(locationOf(event.payload))
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
  }, [go])

  const context = useMemo<AppContextValue | null>(() => {
    if (data.profiles.status !== "ready" || modules === null) return null
    const profiles = data.profiles.data
    return {
      profiles,
      editingProfileId: resolveEditingProfile(profiles, data.editing),
      setEditingProfileId: data.setEditingProfileId,
      refresh: data.refresh,
      revision: data.revision,
      agent: data.agent,
      setAgent: data.setAgent,
      updates: data.updates,
      setUpdates: data.setUpdates,
      navigate: navigation.navigate,
      openSettings: navigation.openSettings,
      closeSettings: () => undefined,
      history: {
        canGoBack: navigation.canGoBack,
        canGoForward: navigation.canGoForward,
        back: navigation.back,
        forward: navigation.forward,
      },
      modules,
      setModules: data.setModules,
      ...(data.pluginModules.length > 0 ? { pluginModules: data.pluginModules } : {}),
    }
  }, [data, modules, navigation])

  if (!context) {
    return (
      <div className="p-6">
        {data.profiles.status === "error" ? (
          <ErrorNotice error={data.profiles.error} />
        ) : (
          <Loading />
        )}
      </div>
    )
  }

  const page = navigation.location.page
  // A module page disappears when its module is disabled.
  const Current = Object.hasOwn(PAGES, page)
    ? PAGES[page as CorePageId]
    : isModulePageAvailable(context.modules, page)
      ? MODULE_VIEWS[page]
      : undefined
  const ViewComponent = Current ?? DEFAULT_VIEW
  const fallbackTitle =
    PAGE_TITLES[page as CorePageId] ??
    context.modules
      .flatMap((m) => m.pages.map((p) => ({ id: `${m.id}/${p.id}`, title: p.title })))
      .find((p) => p.id === page)?.title ??
    "DeckPilot"
  const openEditor = () => {
    context.navigate("designer")
  }
  return (
    <AppContext value={context}>
      <TitlebarContext value={titlebar}>
        <StatusContext value={status}>
          <ExecutionsContext value={executions}>
            <EditorSelectionProvider dirtyRef={dirtyRef}>
              <SidebarProvider
                open={rail}
                onOpenChange={setRail}
                className="h-svh min-h-0 flex-col overflow-hidden"
                style={{ "--statusbar-height": "var(--statusbar-size)" } as React.CSSProperties}
              >
                <AppTitlebar
                  store={titlebar}
                  chrome={chrome}
                  history={context.history}
                  fallbackTitle={fallbackTitle}
                  leading={
                    <>
                      <DeviceSelector onChosen={openEditor} />
                      <ProfileSelector />
                    </>
                  }
                  trailing={
                    <ToolbarEnd
                      plugins={plugins}
                      marketplace={MARKETPLACE}
                      marketplaceOpen={page === "marketplace"}
                      onMarketplace={() => {
                        context.navigate("marketplace")
                      }}
                    />
                  }
                />
                <div className="flex min-h-0 flex-1">
                  <AppSidebar current={page as PageId} onNavigate={context.navigate} />
                  <SidebarInset className="min-h-0 overflow-y-auto">
                    <Suspense fallback={<PageLoading />}>
                      {/* Remount on profile change: open editors never write to another profile. */}
                      <ViewComponent key={context.editingProfileId ?? "none"} />
                    </Suspense>
                  </SidebarInset>
                </div>
                <AppStatusBar stores={status} />
                <CoreStatusItems />
                <ExecutionSync />
                {moduleStatusItems(context.modules).map(({ key, component: Item }) => (
                  <Item key={key} />
                ))}
              </SidebarProvider>
            </EditorSelectionProvider>
          </ExecutionsContext>
        </StatusContext>
      </TitlebarContext>
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
