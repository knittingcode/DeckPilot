import { listen } from "@tauri-apps/api/event"
import {
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import { toast } from "sonner"

import { AppSidebar } from "@/components/app-sidebar"
import { AppTitlebar } from "@/components/app-titlebar"
import { AppStatusBar } from "@/components/status-bar"
import { ExecutionSync } from "@/components/execution-sync"
import { CoreStatusItems } from "@/components/status-sources"
import { SecurityGate } from "@/components/security-gate"
import { DialogProvider } from "@/components/dialogs/dialog-service"
import { SettingsSidebar } from "@/components/settings-sidebar"
import { ErrorNotice, Loading } from "@/components/page"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { useNavigation } from "@/hooks/use-navigation"
import { ExecutionsContext } from "@/hooks/use-executions"
import { StatusContext, createStatusStores } from "@/hooks/use-status"
import { TitlebarContext, useTitlebar } from "@/hooks/use-titlebar"
import { useWindowChrome } from "@/hooks/use-window-chrome"
import { NAVIGATE_EVENT } from "@/lib/api"
import { AppContext, type AppContextValue, type CorePageId, type PageId } from "@/lib/app-context"
import { ThemeProvider } from "@/components/theme-provider"
import { type Location, browserStorage, locationOf } from "@/lib/navigation"
import { ExecutionStore } from "@/lib/executions"
import { removeObsoleteLibraryKeys } from "@/layout-editor/library-sidebar"
import { TitlebarStore } from "@/lib/titlebar"
import { cn } from "@/lib/utils"
import {
  CORE_PAGE_CONTEXTS,
  type ToolbarContext,
  corePageContext,
  showsDevice,
  showsLayout,
  showsProfile,
  toolbarContext,
} from "@/lib/toolbar-context"
import {
  MODULE_VIEWS,
  isModulePageAvailable,
  modulePageContext,
  moduleStatusItems,
} from "@/modules"
import { hostPage, hostStatusItems, isHostPageAvailable } from "@/modules/host-views"
import { pluginFeatures } from "@/lib/plugin-features"
import { findSettingsPage } from "@/settings/registry"
import { SettingsView } from "@/settings/settings-view"
import { resolveEditingProfile, useAppData } from "@/shell/app-data"
import { EditorSelectionProvider } from "@/shell/editor-selection"
import { unsavedWarning } from "@/shell/editor-selection-context"
import { reportUnsaved, useUnsavedGuards } from "@/shell/unsaved-guards"
import {
  PluginNavigationContext,
  navigationKey,
  parsePluginViewPage,
  pluginViewPage,
  usePluginNavigation,
} from "@/shell/plugin-navigation"
import { DeviceSelector, LayoutSelector, ProfileSelector, ToolbarEnd } from "@/shell/toolbar"
import { ViewActiveContext } from "@/shell/view-activity"

type View = React.LazyExoticComponent<() => React.JSX.Element>

/** The build has the plugin marketplace (VITE_DECKPILOT_PLUGINS=1). */
const MARKETPLACE = import.meta.env.VITE_DECKPILOT_PLUGINS === "1"

// Pages load on first use, which keeps the start-up bundle small. Pages of
// feature modules come from the module registry, the Workflows and Projects
// screens from the plugins that activate them (`modules/host-views.ts`).
// Profiles, devices and the rest of the configuration are settings pages.
const PAGES: Partial<Record<CorePageId, View>> = {
  designer: lazy(() =>
    import("@/pages/layout-designer").then((m) => ({ default: m.LayoutDesignerPage })),
  ),
  dashboard: lazy(() => import("@/pages/dashboard").then((m) => ({ default: m.DashboardPage }))),
  layouts: lazy(() => import("@/pages/layouts").then((m) => ({ default: m.LayoutsPage }))),
  actions: lazy(() => import("@/pages/actions").then((m) => ({ default: m.ActionsPage }))),
  ...(MARKETPLACE
    ? {
        marketplace: lazy(() =>
          import("@/pages/marketplace").then((m) => ({ default: m.MarketplacePage })),
        ),
      }
    : {}),
}

/**
 * Titles of core pages before they register their own, e.g. while loading.
 * The device editor has none in the toolbar (its selectors say what is
 * edited).
 */
const PAGE_TITLES: Partial<Record<CorePageId, string>> = {
  designer: "",
  dashboard: "Dashboard",
  layouts: "Layouts",
  actions: "Action Library",
  marketplace: "Marketplace",
}

const DEFAULT_VIEW = PAGES.designer as View

/**
 * Shows plugin dialogs (plugin API 1.5) in builds with plugins; loaded on
 * first use, and absent from other builds.
 */
const PLUGIN_DIALOG_HOST = MARKETPLACE
  ? lazy(() => import("@/plugins/dialog-host").then((m) => ({ default: m.PluginDialogHost })))
  : null

/** The view of a plugin's toolbar entry (`plugin-view/<plugin>/<entry>`). */
const PLUGIN_VIEW: View = lazy(() =>
  import("@/pages/plugin-view").then((m) => ({ default: m.PluginViewPage })),
)

/** Shown while a page loads; the titlebar shows progress meanwhile. */
function PageLoading() {
  useTitlebar({ progress: true })
  return (
    <div className="p-6">
      <Loading />
    </div>
  )
}

/** Where earlier versions remembered an expanded rail; the rail is always collapsed now. */
const OBSOLETE_RAIL_KEY = "deckpilot.rail"

/**
 * The main window: the toolbar (device and profile, the page's context,
 * plugin entries, notifications, marketplace, settings), the workspace
 * rail, the current view and the status bar. The view is a workspace page
 * (the device editor by default) or the settings: their sidebar and the
 * selected settings page. One history covers both.
 *
 * While the settings are on screen the workspace page under them stays
 * mounted, hidden, so it comes back exactly as it was (the device editor's
 * page, key and inspector). Meanwhile its toolbar contributions are parked
 * in a store nobody shows, and its window-wide shortcuts are off
 * (`ViewActiveContext`).
 */
function Studio() {
  // Unsaved layout changes: shared by the editor, the toolbar's selectors
  // and the profile choices other windows announce.
  const dirtyRef = useRef(false)
  const acceptEditing = useCallback(() => {
    if (!dirtyRef.current) return true
    toast.warning(unsavedWarning())
    return false
  }, [dirtyRef])
  const data = useAppData({ acceptEditing })
  const { modules } = data
  // What the plugins provide now: the Workflows and Projects screens, the
  // active project's status item. Follows every plugin change.
  const features = useMemo(() => pluginFeatures(data.pluginModules), [data.pluginModules])

  // Which locations exist depends on the enabled modules and plugins.
  const exists = useMemo(
    () =>
      modules === null
        ? null
        : (location: Location) =>
            location.mode === "settings"
              ? findSettingsPage(modules, location.page) !== undefined
              : Object.hasOwn(PAGES, location.page) ||
                isModulePageAvailable(modules, location.page) ||
                isHostPageAvailable(features, location.page) ||
                (MARKETPLACE && parsePluginViewPage(location.page) !== null),
    [modules, features],
  )
  const navigation = useNavigation(exists)
  // Leaving the device editor, closing the window or quitting asks over
  // unsaved layout edits.
  const markClean = useCallback(() => {
    if (dirtyRef.current) reportUnsaved(false)
    dirtyRef.current = false
  }, [dirtyRef])
  const navigate = useUnsavedGuards(dirtyRef, navigation.location, navigation.navigate, markClean)
  const [titlebar] = useState(() => new TitlebarStore())
  // Where a hidden workspace page's toolbar contributions wait.
  const [parked] = useState(() => new TitlebarStore())
  const inSettings = navigation.location.mode === "settings"
  // The workspace page's scroll position survives the settings.
  const workspaceRef = useRef<HTMLElement>(null)
  const workspaceScroll = useRef(0)
  useLayoutEffect(() => {
    const element = workspaceRef.current
    if (!element || inSettings) return
    element.scrollTop = workspaceScroll.current
  }, [inSettings])
  const [status] = useState(createStatusStores)
  const [executions] = useState(() => new ExecutionStore())
  const chrome = useWindowChrome()
  // Plugin toolbar entries, read again whenever the plugins are.
  const plugins = usePluginNavigation(data.pluginModules)
  const { go } = navigation

  useEffect(() => {
    try {
      browserStorage()?.removeItem(OBSOLETE_RAIL_KEY)
    } catch {
      // Nothing is read from it any more.
    }
    // The Action Library has one fixed width now and is always shown.
    removeObsoleteLibraryKeys()
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
      navigate,
      openSettings: navigation.openSettings,
      closeSettings: navigation.closeSettings,
      modules,
      setModules: data.setModules,
      ...(data.pluginModules.length > 0 ? { pluginModules: data.pluginModules } : {}),
    }
  }, [data, modules, navigation, navigate])

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

  // The workspace page on screen, or the one kept under the settings.
  const page = inSettings ? navigation.workspace : navigation.location.page
  const settingsPage = inSettings ? navigation.location.page : null
  // A module page disappears when its module is disabled, a host page
  // when its plugin is.
  const pluginView = MARKETPLACE ? parsePluginViewPage(page) : null
  const host = isHostPageAvailable(features, page) ? hostPage(page) : undefined
  const Current = Object.hasOwn(PAGES, page)
    ? PAGES[page as CorePageId]
    : host
      ? host.view
      : isModulePageAvailable(context.modules, page)
        ? MODULE_VIEWS[page]
        : pluginView
          ? PLUGIN_VIEW
          : undefined
  const ViewComponent = Current ?? DEFAULT_VIEW
  // Settings pages show their title themselves; the toolbar shows where they are.
  const fallbackTitle = settingsPage
    ? ""
    : (PAGE_TITLES[page as CorePageId] ??
      host?.fallbackTitle ??
      context.modules
        .flatMap((m) => m.pages.map((p) => ({ id: `${m.id}/${p.id}`, title: p.title })))
        .find((p) => p.id === page)?.title ??
      plugins.find(
        (p) => pluginView && navigationKey(p) === `${pluginView.plugin}/${pluginView.entry}`,
      )?.label ??
      "DeckPilot")
  const openEditor = () => {
    context.navigate("designer")
  }
  // Which selectors the toolbar shows: those of the view on screen. The
  // selection itself stays in the shell while they are hidden.
  const openPlugin = pluginView
    ? plugins.find((p) => navigationKey(p) === `${pluginView.plugin}/${pluginView.entry}`)
    : undefined
  const viewContext: ToolbarContext = inSettings
    ? "none"
    : !Current
      ? CORE_PAGE_CONTEXTS.designer
      : pluginView
        ? toolbarContext(openPlugin?.view.context)
        : (corePageContext(page) ?? host?.context ?? modulePageContext(page))
  const selectors =
    showsDevice(viewContext) || showsProfile(viewContext) ? (
      <>
        {showsDevice(viewContext) && <DeviceSelector onChosen={openEditor} />}
        {showsProfile(viewContext) && <ProfileSelector />}
        {showsLayout(viewContext) && <LayoutSelector />}
      </>
    ) : undefined
  return (
    <AppContext value={context}>
      <TitlebarContext value={titlebar}>
        <StatusContext value={status}>
          <ExecutionsContext value={executions}>
            <EditorSelectionProvider dirtyRef={dirtyRef}>
              <SidebarProvider
                // The rail is always collapsed to icons: no toggle, no ⌘B.
                open={false}
                keyboardShortcut={false}
                className="h-svh min-h-0 flex-col overflow-hidden"
                style={{ "--statusbar-height": "var(--statusbar-size)" } as React.CSSProperties}
              >
                <AppTitlebar
                  store={titlebar}
                  chrome={chrome}
                  fallbackTitle={fallbackTitle}
                  leading={selectors}
                  context={viewContext}
                  trailing={
                    <ToolbarEnd
                      plugins={plugins}
                      pluginOpen={
                        pluginView && !inSettings
                          ? `${pluginView.plugin}/${pluginView.entry}`
                          : null
                      }
                      onPlugin={(item) => {
                        context.navigate(pluginViewPage(item) as PageId)
                      }}
                      marketplace={MARKETPLACE}
                      marketplaceOpen={!inSettings && page === "marketplace"}
                      onMarketplace={() => {
                        context.navigate("marketplace")
                      }}
                      settingsOpen={inSettings}
                    />
                  }
                />
                <div className="flex min-h-0 flex-1">
                  <AppSidebar
                    current={inSettings ? null : (page as PageId)}
                    onNavigate={context.navigate}
                  />
                  <SidebarInset
                    ref={workspaceRef}
                    hidden={inSettings}
                    onScroll={(event) => {
                      if (!inSettings) workspaceScroll.current = event.currentTarget.scrollTop
                    }}
                    className={cn("relative min-h-0 overflow-y-auto", inSettings && "hidden")}
                  >
                    <TitlebarContext value={inSettings ? parked : titlebar}>
                      <ViewActiveContext value={!inSettings}>
                        <PluginNavigationContext value={{ items: plugins, page }}>
                          <Suspense fallback={<PageLoading />}>
                            {/* Remount on profile change: open editors never write to another profile. */}
                            <ViewComponent
                              key={`${context.editingProfileId ?? "none"}${pluginView ? `:${page}` : ""}`}
                            />
                          </Suspense>
                        </PluginNavigationContext>
                      </ViewActiveContext>
                    </TitlebarContext>
                  </SidebarInset>
                  {settingsPage !== null && (
                    <SidebarInset data-view="settings" className="min-h-0 flex-row overflow-hidden">
                      <SettingsSidebar current={settingsPage} />
                      <div className="relative min-h-0 min-w-0 flex-1 overflow-y-auto">
                        <Suspense fallback={<PageLoading />}>
                          <SettingsView key={settingsPage} id={settingsPage} />
                        </Suspense>
                      </div>
                    </SidebarInset>
                  )}
                </div>
                <AppStatusBar stores={status} />
                <CoreStatusItems />
                <ExecutionSync />
                {moduleStatusItems(context.modules).map(({ key, component: Item }) => (
                  <Item key={key} />
                ))}
                {hostStatusItems(features).map(({ key, component: Item }) => (
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
          {/* Only while unlocked: no app dialog can appear over the locked state. */}
          <DialogProvider>
            <Studio />
            {PLUGIN_DIALOG_HOST && (
              <Suspense fallback={null}>
                <PLUGIN_DIALOG_HOST />
              </Suspense>
            )}
          </DialogProvider>
        </SecurityGate>
        <Toaster position="bottom-right" />
      </TooltipProvider>
    </ThemeProvider>
  )
}
