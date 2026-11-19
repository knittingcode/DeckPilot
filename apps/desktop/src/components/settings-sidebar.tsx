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
 * Navigation of the settings, a view of the main window: the categories and
 * their search, beside the main window's rail. It is part of the view's
 * layout (not a fixed sidebar), so it never covers the rail; the main
 * window's toolbar provides back, forward and the way out.
 */
export function SettingsSidebar({ current }: { current: string }) {
  const { modules, updates, openSettings } = useAppContext()
  const [query, setQuery] = useState("")
  const groups = groupedSettings(searchSettings(settingsPages(modules), query))
  return (
    <Sidebar collapsible="none" aria-label="Settings" className="h-full shrink-0 border-r">
      <SidebarHeader>
        <h2 className="px-2 pt-1 text-lg font-semibold">Settings</h2>
        <SidebarInput
          type="search"
          placeholder="Search settings"
          aria-label="Search settings"
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
                      aria-current={current === page.id ? "page" : undefined}
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
