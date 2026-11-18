import { LayoutDashboard, LayoutGrid, Library, ListOrdered, type LucideIcon } from "lucide-react"

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
import { type PageId, useAppContext } from "@/lib/app-context"
import { moduleNavigation } from "@/modules"

interface NavGroup {
  label: string
  items: { id: PageId; label: string; icon: LucideIcon }[]
}

/**
 * The workspace pages besides the device editor. Module pages are added to
 * "Configure". Profiles, devices and everything else to set up are in the
 * settings window; device and profile are chosen in the toolbar.
 */
const RAIL_GROUPS: NavGroup[] = [
  {
    label: "Workspace",
    items: [
      { id: "designer", label: "Device Editor", icon: LayoutGrid },
      { id: "dashboard", label: "Dashboard", icon: LayoutDashboard },
    ],
  },
  {
    label: "Configure",
    items: [
      { id: "actions", label: "Action Library", icon: Library },
      { id: "workflows", label: "Workflows", icon: ListOrdered },
    ],
  },
]

/**
 * The slim navigation rail of the main window: icons with tooltips while
 * collapsed (the default), labels while expanded. Toggled from the toolbar
 * or with the sidebar shortcut.
 */
export function AppSidebar({
  current,
  onNavigate,
}: {
  current: PageId
  onNavigate: (page: PageId) => void
}) {
  const { modules } = useAppContext()
  const fromModules = moduleNavigation(modules)
  const groups = RAIL_GROUPS.map((group) =>
    group.label === "Configure" ? { ...group, items: [...group.items, ...fromModules] } : group,
  )
  return (
    <Sidebar collapsible="icon" aria-label="Workspace">
      <SidebarContent>
        {groups.map((group) => (
          <SidebarGroup key={group.label}>
            <SidebarGroupLabel>{group.label}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {group.items.map((item) => (
                  <SidebarMenuItem key={item.id}>
                    <SidebarMenuButton
                      isActive={current === item.id}
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
