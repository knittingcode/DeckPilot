import { Search } from "lucide-react"
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
 * window's toolbar and rail lead out of it.
 */
export function SettingsSidebar({ current }: { current: string }) {
  const { modules, updates, openSettings } = useAppContext()
  const [query, setQuery] = useState("")
  const groups = groupedSettings(searchSettings(settingsPages(modules), query))
  return (
    <Sidebar
      collapsible="none"
      aria-label="Settings"
      className="h-full shrink-0 border-r border-sidebar-border"
    >
      {/* The titlebar names the view ("Settings / …"); the sidebar starts with its search. */}
      <SidebarHeader className="px-3 pt-3 pb-1">
        <div className="relative">
          <Search
            aria-hidden
            className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground"
          />
          <SidebarInput
            type="search"
            placeholder="Search settings"
            aria-label="Search settings"
            className="h-7 pl-7 text-caption"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
            }}
          />
        </div>
      </SidebarHeader>
      <SidebarContent className="gap-0 pb-2">
        {groups.length === 0 && (
          <p role="status" className="px-4 py-2 text-caption text-muted-foreground">
            No settings found
          </p>
        )}
        {groups.map((group) => (
          <SidebarGroup key={group.id} className="px-2 py-1.5">
            <SidebarGroupLabel className="h-6 text-xs font-medium text-muted-foreground">
              {group.label}
            </SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu className="gap-px">
                {group.pages.map((page) => (
                  <SidebarMenuItem key={page.id}>
                    <SidebarMenuButton
                      isActive={current === page.id}
                      aria-current={current === page.id ? "page" : undefined}
                      className="h-7 text-caption data-[active=true]:font-medium [&>svg]:size-3.5 [&>svg]:text-muted-foreground data-[active=true]:[&>svg]:text-sidebar-accent-foreground"
                      onClick={() => {
                        openSettings(page.id)
                      }}
                    >
                      <page.icon />
                      <span>{page.title}</span>
                    </SidebarMenuButton>
                    {page.id === "updates" && updates?.available && (
                      <SidebarMenuBadge className="top-1 h-5 text-xs text-muted-foreground">
                        New
                      </SidebarMenuBadge>
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
