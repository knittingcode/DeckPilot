import { ArrowLeft } from "lucide-react"
import { useState } from "react"

import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInput,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import { useAppContext } from "@/lib/app-context"
import { groupedSettings, searchSettings, settingsPages } from "@/settings/registry"

/**
 * Navigation of the settings. Replaces the workspace sidebar while settings
 * are open; "Back" returns to the workspace page that was open before.
 */
export function SettingsSidebar({ current }: { current: string }) {
  const { modules, updates, openSettings, closeSettings } = useAppContext()
  const [query, setQuery] = useState("")
  const groups = groupedSettings(searchSettings(settingsPages(modules), query))
  return (
    <Sidebar collapsible="icon" aria-label="Settings">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton tooltip="Back to DeckPilot" onClick={closeSettings}>
              <ArrowLeft />
              <span>Back to DeckPilot</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
        <h2 className="px-2 pt-1 text-lg font-semibold group-data-[collapsible=icon]:hidden">
          Settings
        </h2>
        <SidebarInput
          type="search"
          placeholder="Search settings"
          aria-label="Search settings"
          className="group-data-[collapsible=icon]:hidden"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
          }}
        />
      </SidebarHeader>
      <SidebarContent>
        {groups.length === 0 && (
          <p className="px-4 py-2 text-sm text-muted-foreground">No settings found</p>
        )}
        {groups.map((group) => (
          <SidebarGroup key={group.id}>
            <SidebarGroupLabel>{group.label}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {group.pages.map((page) => (
                  <SidebarMenuItem key={page.id}>
                    <SidebarMenuButton
                      isActive={current === page.id}
                      tooltip={page.title}
                      onClick={() => {
                        openSettings(page.id)
                      }}
                    >
                      <page.icon />
                      <span>{page.title}</span>
                    </SidebarMenuButton>
                    {page.id === "updates" && updates?.available && (
                      <SidebarMenuBadge>New</SidebarMenuBadge>
                    )}
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
