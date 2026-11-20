import { LayoutDashboard, LayoutGrid, LayoutTemplate, Library, type LucideIcon } from "lucide-react"

import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import { type TranslationKey, useTranslation } from "@/i18n"
import { type PageId, useAppContext } from "@/lib/app-context"
import { usePluginFeatures } from "@/lib/plugin-features"
import { moduleNavigation } from "@/modules"
import { hostPages } from "@/modules/host-views"

interface NavGroup {
  id: "workspace" | "configure"
  label: TranslationKey
  items: { id: PageId; label: TranslationKey; icon: LucideIcon }[]
}

/**
 * The workspace pages besides the device editor. The Workflows and Projects
 * screens (while their plugins activate them) and module pages are added to
 * "Configure". Profiles, devices and everything else to set up are in the
 * settings window; device and profile are chosen in the toolbar.
 */
const RAIL_GROUPS: NavGroup[] = [
  {
    id: "workspace",
    label: "navigation:rail.workspace",
    items: [
      { id: "designer", label: "navigation:pages.designer", icon: LayoutGrid },
      { id: "layouts", label: "navigation:pages.layouts", icon: LayoutTemplate },
      { id: "dashboard", label: "navigation:pages.dashboard", icon: LayoutDashboard },
    ],
  },
  {
    id: "configure",
    label: "navigation:rail.configure",
    items: [{ id: "actions", label: "navigation:pages.actions", icon: Library }],
  },
]

/**
 * The slim navigation rail of the main window, always collapsed to icons
 * (the provider in `App.tsx` keeps it closed; nothing expands it). Each icon
 * has its name as accessible label and as tooltip; the page on screen is
 * marked with a fill and a bar at the rail's edge, and `aria-current`.
 */
export function AppSidebar({
  current,
  onNavigate,
}: {
  /** The workspace page on screen; none while the settings are. */
  current: PageId | null
  onNavigate: (page: PageId) => void
}) {
  const { t } = useTranslation()
  const { modules } = useAppContext()
  const features = usePluginFeatures()
  // Module pages are named by their module (its manifest), not translated here.
  const fromModules = moduleNavigation(modules)
  const fromPlugins = hostPages(features).map((page) => ({
    id: page.id,
    label: t(page.label),
    icon: page.icon,
  }))
  const groups = RAIL_GROUPS.map((group) => {
    const items = group.items.map((item) => ({ ...item, label: t(item.label) }))
    return {
      id: group.id,
      label: t(group.label),
      items: group.id === "configure" ? [...items, ...fromPlugins, ...fromModules] : items,
    }
  })
  return (
    <Sidebar collapsible="icon" aria-label={t("navigation:rail.label")}>
      <SidebarContent>
        {groups.map((group) => (
          <SidebarGroup key={group.id}>
            <SidebarGroupLabel>{group.label}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {group.items.map((item) => (
                  <SidebarMenuItem key={item.id}>
                    <SidebarMenuButton
                      isActive={current === item.id}
                      className="relative data-[active=true]:before:absolute data-[active=true]:before:inset-y-1.5 data-[active=true]:before:left-0 data-[active=true]:before:w-0.5 data-[active=true]:before:rounded-full data-[active=true]:before:bg-sidebar-primary"
                      aria-current={current === item.id ? "page" : undefined}
                      tooltip={item.label}
                      onClick={() => {
                        onNavigate(item.id)
                      }}
                    >
                      <item.icon />
                      <span>{item.label}</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
      </SidebarContent>
    </Sidebar>
  )
}
