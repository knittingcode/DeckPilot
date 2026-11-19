import { emitTo, listen } from "@tauri-apps/api/event"
import { Suspense, useCallback, useEffect, useMemo, useState } from "react"

import { AppTitlebar } from "@/components/app-titlebar"
import { ErrorNotice, Loading } from "@/components/page"
import { SecurityGate } from "@/components/security-gate"
import { SettingsSidebar } from "@/components/settings-sidebar"
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar"
import { TitlebarContext } from "@/hooks/use-titlebar"
import { NAVIGATE_EVENT, WORKSPACE_PAGE_EVENT, api } from "@/lib/api"
import { AppContext, type AppContextValue } from "@/lib/app-context"
import {
  DEFAULT_SETTINGS_PAGE,
  type NavigationState,
  back,
  browserStorage,
  canGoBack,
  canGoForward,
  current,
  forward,
  initialNavigation,
  openSettings,
} from "@/lib/navigation"
import { TitlebarStore } from "@/lib/titlebar"
import { resolveEditingProfile, useAppData } from "@/shell/app-data"

import { findSettingsPage } from "./registry"
import { SettingsView } from "./settings-view"
import { initialSettingsPage, rememberSettingsPage, settingsPageOf } from "./window-page"

/** The main window's chrome is not this window's: it has a native titlebar. */
const NATIVE_CHROME = { overlay: false, controlsInset: 0 }

/**
 * The settings window (`app.settings`): the settings sidebar and the
 * settings pages, with their own history. Pages of the main window that a
 * settings page links to open there. The window holds its own copy of the
 * app's state, kept in step with the main window (`useAppData`).
 */
export function SettingsWindow() {
  return (
    <SecurityGate secondary>
      <SettingsWorkspace />
    </SecurityGate>
  )
}

function SettingsWorkspace() {
  const data = useAppData()
  const { modules } = data
  const [titlebar] = useState(() => new TitlebarStore())
  const [history, setHistory] = useState<NavigationState>(() =>
    initialNavigation({
      mode: "settings",
      page: initialSettingsPage(window.location.search, browserStorage()),
    }),
  )

  const open = useCallback((page?: string) => {
    setHistory((state) => openSettings(state, page))
  }, [])

  // A page of the main window: shown there.
  const navigate = useCallback(
    (page: string) => {
      const settings = settingsPageOf(page)
      if (settings) {
        open(settings)
        return
      }
      void emitTo("main", NAVIGATE_EVENT, page)
        .catch(() => undefined)
        .then(() => api.showMainWindow())
        .catch(() => undefined)
    },
    [open],
  )

  // The main window asks for another page while this window is open.
  useEffect(() => {
    let stopped = false
    let stop: (() => void) | undefined
    listen<string>(WORKSPACE_PAGE_EVENT, (event) => {
      navigate(event.payload)
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
  }, [navigate])

  const location = current(history)
  const page =
    modules && findSettingsPage(modules, location.page) ? location.page : DEFAULT_SETTINGS_PAGE

  useEffect(() => {
    rememberSettingsPage(browserStorage(), page)
  }, [page])

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
      openSettings: open,
      closeSettings: () => {
        void api.showMainWindow().catch(() => undefined)
      },
      history: {
        canGoBack: canGoBack(history),
        canGoForward: canGoForward(history),
        back: () => {
          setHistory(back)
        },
        forward: () => {
          setHistory(forward)
        },
      },
      modules,
      setModules: data.setModules,
      ...(data.pluginModules.length > 0 ? { pluginModules: data.pluginModules } : {}),
    }
  }, [data, modules, navigate, open, history])

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

  const fallbackTitle = findSettingsPage(context.modules, page)?.title ?? "Settings"
  return (
    <AppContext value={context}>
      <TitlebarContext value={titlebar}>
        <SidebarProvider className="h-svh min-h-0 flex-col overflow-hidden">
          <AppTitlebar
            store={titlebar}
            chrome={NATIVE_CHROME}
            history={context.history}
            fallbackTitle={fallbackTitle}
          />
          <div className="flex min-h-0 flex-1">
            <SettingsSidebar current={page} />
            <SidebarInset className="min-h-0 overflow-y-auto">
              <Suspense
                fallback={
                  <div className="p-6">
                    <Loading />
                  </div>
                }
              >
                <SettingsView key={page} id={page} />
              </Suspense>
            </SidebarInset>
          </div>
        </SidebarProvider>
      </TitlebarContext>
    </AppContext>
  )
}
