import { Monitor, Moon, Sun } from "lucide-react"

import { AgentSettings } from "@/components/agent-settings"
import { ErrorNotice, Loading, Page } from "@/components/page"
import { UpdatesCard } from "@/components/updates-card"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { osLabel } from "@/lib/format"
import { type ThemePreference, useTheme } from "@/lib/theme"

export function SettingsPage() {
  const { preference, setPreference } = useTheme()
  const { revision } = useAppContext()
  const { state } = useAsync(
    () => Promise.all([api.getOverview(), api.launchInfo()]),
    String(revision),
  )
  return (
    <Page title="Settings" description="Appearance, agent, updates and diagnostics">
      <Card>
        <CardHeader>
          <CardTitle>Appearance</CardTitle>
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
      <AgentSettings />
      <UpdatesCard />
      <Card>
        <CardHeader>
          <CardTitle>About</CardTitle>
          <CardDescription>DeckPilot is open source under the Apache License 2.0.</CardDescription>
        </CardHeader>
        <CardContent>
          {state.status === "loading" && <Loading />}
          {state.status === "error" && <ErrorNotice error={state.error} />}
          {state.status === "ready" && (
            <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-[12rem_1fr]">
              <dt className="text-muted-foreground">Version</dt>
              <dd>
                {state.data[0].appVersion}
                {state.data[0].development && " (development session)"}
              </dd>
              <dt className="text-muted-foreground">Operating system</dt>
              <dd>{osLabel(state.data[0].os)}</dd>
              <dt className="text-muted-foreground">Database</dt>
              <dd className="font-mono text-xs break-all select-text">
                {state.data[0].databasePath}
              </dd>
              <dt className="text-muted-foreground">Previous version</dt>
              <dd>{state.data[1].launch?.previousVersion ?? "—"}</dd>
              <dt className="text-muted-foreground">Database encryption</dt>
              <dd>
                {state.data[1].encryption.enabled
                  ? "Enabled"
                  : "Not enabled (planned; no password is requested)"}
              </dd>
            </dl>
          )}
        </CardContent>
      </Card>
    </Page>
  )
}
