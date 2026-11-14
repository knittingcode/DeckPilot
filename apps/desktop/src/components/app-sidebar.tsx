import {
  Boxes,
  Check,
  ChevronsUpDown,
  LayoutDashboard,
  LayoutGrid,
  Library,
  ListOrdered,
  Settings,
  UserRound,
  type LucideIcon,
} from "lucide-react"

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import { SHORTCUTS } from "@/hooks/use-navigation"
import { type PageId, useAppContext } from "@/lib/app-context"
import { cn } from "@/lib/utils"
import { moduleNavigation } from "@/modules"

interface NavGroup {
  label: string
  items: { id: PageId; label: string; icon: LucideIcon }[]
}

/** Navigation of the core application. Module pages are added to "Configure". */
const GROUPS: NavGroup[] = [
  { label: "Overview", items: [{ id: "dashboard", label: "Dashboard", icon: LayoutDashboard }] },
  {
    label: "Configure",
    items: [
      { id: "actions", label: "Action Library", icon: Library },
      { id: "workflows", label: "Workflows", icon: ListOrdered },
      { id: "profiles", label: "Profiles", icon: UserRound },
    ],
  },
  {
    label: "Devices",
    // Connected devices and supported models are in Settings › Devices.
    items: [{ id: "designer", label: "Layout Designer", icon: LayoutGrid }],
  },
]

function ProfileSwitcher() {
  const { profiles, editingProfileId, setEditingProfileId } = useAppContext()
  const editing = profiles.profiles.find((p) => p.id === editingProfileId)
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <SidebarMenuButton size="lg" aria-label="Choose the profile to edit">
          <div className="flex aspect-square size-8 items-center justify-center rounded-lg bg-sidebar-primary text-sidebar-primary-foreground">
            <Boxes className="size-4" />
          </div>
          <div className="grid flex-1 text-left text-sm leading-tight">
            <span className="truncate font-semibold">DeckPilot</span>
            <span className="truncate text-xs text-muted-foreground">
              {editing ? `Editing ${editing.name}` : "No profile"}
            </span>
          </div>
          <ChevronsUpDown className="ml-auto size-4" />
        </SidebarMenuButton>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="w-60" align="start">
        <DropdownMenuLabel>Edit profile</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {profiles.profiles.length === 0 && (
          <DropdownMenuItem disabled>No profiles yet</DropdownMenuItem>
        )}
        {profiles.profiles.map((profile) => (
          <DropdownMenuItem
            key={profile.id}
            onSelect={() => {
              setEditingProfileId(profile.id)
            }}
          >
            <Check className={cn(profile.id !== editingProfileId && "invisible")} />
            <span className="flex-1 truncate">{profile.name}</span>
            {profile.id === profiles.activeProfileId && (
              <span className="text-xs text-muted-foreground">on device</span>
            )}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export function AppSidebar({
  current,
  onNavigate,
}: {
  current: PageId
  onNavigate: (page: PageId) => void
}) {
  const { updates, modules, openSettings } = useAppContext()
  const updateAvailable = Boolean(updates?.available)
  const fromModules = moduleNavigation(modules)
  const groups = GROUPS.map((group) =>
    group.label === "Configure" ? { ...group, items: [...group.items, ...fromModules] } : group,
  )
  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <ProfileSwitcher />
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>
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
      <SidebarFooter>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              tooltip={`Settings (${SHORTCUTS.settings})`}
              onClick={() => {
                openSettings(updateAvailable ? "updates" : undefined)
              }}
            >
              <Settings />
              <span>Settings</span>
            </SidebarMenuButton>
            {updateAvailable && <SidebarMenuBadge>Update</SidebarMenuBadge>}
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  )
}
