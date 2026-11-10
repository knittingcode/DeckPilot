import {
  Boxes,
  Cpu,
  FolderKanban,
  LayoutDashboard,
  Settings,
  UserRound,
  Zap,
  type LucideIcon,
} from "lucide-react"

import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import { useAppContext } from "@/lib/app-context"

export type PageId = "dashboard" | "actions" | "profiles" | "projects" | "devices" | "settings"

const NAVIGATION: { id: PageId; label: string; icon: LucideIcon }[] = [
  { id: "dashboard", label: "Dashboard", icon: LayoutDashboard },
  { id: "actions", label: "Actions", icon: Zap },
  { id: "profiles", label: "Profiles", icon: UserRound },
  { id: "projects", label: "Projects", icon: FolderKanban },
  { id: "devices", label: "Devices", icon: Cpu },
  { id: "settings", label: "Settings", icon: Settings },
]

export function AppSidebar({
  current,
  onNavigate,
}: {
  current: PageId
  onNavigate: (page: PageId) => void
}) {
  const { profiles } = useAppContext()
  const active = profiles.profiles.find((p) => p.id === profiles.activeProfileId)

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" className="pointer-events-none">
              <div className="flex aspect-square size-8 items-center justify-center rounded-lg bg-sidebar-primary text-sidebar-primary-foreground">
                <Boxes className="size-4" />
              </div>
              <div className="grid flex-1 text-left text-sm leading-tight">
                <span className="truncate font-semibold">DeckPilot</span>
                <span className="truncate text-xs text-muted-foreground">
                  {active ? active.name : "No active profile"}
                </span>
              </div>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Configuration</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {NAVIGATION.map((item) => (
                <SidebarMenuItem key={item.id}>
                  <SidebarMenuButton
                    isActive={current === item.id}
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
      </SidebarContent>
      <SidebarFooter>
        <p className="px-2 text-xs text-muted-foreground group-data-[collapsible=icon]:hidden">
          Configuration is stored locally on this computer.
        </p>
      </SidebarFooter>
    </Sidebar>
  )
}
