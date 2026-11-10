import { Monitor, Moon, Sun } from "lucide-react"

import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { type ThemePreference, useTheme } from "@/lib/theme"

export function AppearanceSettings() {
  const { preference, setPreference } = useTheme()
  return (
    <Card>
      <CardHeader>
        <CardTitle>Theme</CardTitle>
        <CardDescription>
          System follows the light or dark appearance of your operating system.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <ToggleGroup
          type="single"
          variant="outline"
          value={preference}
          onValueChange={(value) => {
            if (value) setPreference(value as ThemePreference)
          }}
          aria-label="Theme"
        >
          <ToggleGroupItem value="system" className="px-3">
            <Monitor /> System
          </ToggleGroupItem>
          <ToggleGroupItem value="light" className="px-3">
            <Sun /> Light
          </ToggleGroupItem>
          <ToggleGroupItem value="dark" className="px-3">
            <Moon /> Dark
          </ToggleGroupItem>
        </ToggleGroup>
      </CardContent>
    </Card>
  )
}
