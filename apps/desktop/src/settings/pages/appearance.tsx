import { Monitor, Moon, Sun } from "lucide-react"

import { SettingsGroup, SettingsRow } from "@/components/settings-layout"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { type ThemePreference, useTheme } from "@/lib/theme"

export function AppearanceSettings() {
  const { preference, setPreference } = useTheme()
  return (
    <SettingsGroup>
      <SettingsRow
        label="Theme"
        description="System follows the light or dark appearance of your operating system."
      >
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          value={preference}
          onValueChange={(value) => {
            if (value) setPreference(value as ThemePreference)
          }}
          aria-label="Theme"
        >
          <ToggleGroupItem value="system" className="px-2.5">
            <Monitor /> System
          </ToggleGroupItem>
          <ToggleGroupItem value="light" className="px-2.5">
            <Sun /> Light
          </ToggleGroupItem>
          <ToggleGroupItem value="dark" className="px-2.5">
            <Moon /> Dark
          </ToggleGroupItem>
        </ToggleGroup>
      </SettingsRow>
    </SettingsGroup>
  )
}
