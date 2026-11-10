import { ChevronLeft, ChevronRight, Info } from "lucide-react"
import { useState } from "react"

import { ErrorNotice, Loading, Page } from "@/components/page"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { osLabel } from "@/lib/format"

export function DashboardPage() {
  const { revision, refresh } = useAppContext()
  const { state, reload } = useAsync(() => api.getOverview(), String(revision))
  const [error, setError] = useState<string | null>(null)

  const cycle = async (profileId: string, direction: "next" | "previous") => {
    setError(null)
    try {
      await api.cycleProject(profileId, direction)
      reload()
      refresh()
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  return (
    <Page title="Dashboard" description="Overview of the local configuration">
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {error && <ErrorNotice error={error} />}
      {state.status === "ready" && (
        <>
          <div className="grid gap-4 md:grid-cols-2">
            <Card>
              <CardHeader>
                <CardDescription>Active profile</CardDescription>
                <CardTitle className="text-xl">
                  {state.data.activeProfile?.name ?? "None"}
                </CardTitle>
              </CardHeader>
              <CardContent className="text-sm text-muted-foreground">
                {state.data.profileCount} profile{state.data.profileCount === 1 ? "" : "s"} stored
              </CardContent>
            </Card>
            <Card>
              <CardHeader>
                <CardDescription>Active project</CardDescription>
                <CardTitle className="text-xl">
                  {state.data.activeProject
                    ? `${state.data.activeProject.key} · ${state.data.activeProject.name}`
                    : "None"}
                </CardTitle>
              </CardHeader>
              <CardContent className="flex gap-2">
                {state.data.activeProfile && (
                  <ProjectCycler
                    profileId={state.data.activeProfile.id}
                    disabled={!state.data.activeProject}
                    onCycle={cycle}
                  />
                )}
              </CardContent>
            </Card>
          </div>

          <div className="grid gap-4 sm:grid-cols-3">
            <Stat label="Action types" value={state.data.actionTypeCount} />
            <Stat label="Device models" value={state.data.deviceModelCount} />
            <Stat label="Resource bindings" value={state.data.resourceBindingCount} />
          </div>

          <Alert>
            <Info />
            <AlertTitle>Device handling is not available yet</AlertTitle>
            <AlertDescription>
              This version manages configuration only. The background agent that talks to Stream
              Deck hardware is planned for the next milestone.
            </AlertDescription>
          </Alert>

          <Card>
            <CardHeader>
              <CardTitle>System</CardTitle>
            </CardHeader>
            <CardContent className="grid gap-2 text-sm sm:grid-cols-[10rem_1fr]">
              <span className="text-muted-foreground">Version</span>
              <span>{state.data.appVersion}</span>
              <span className="text-muted-foreground">Operating system</span>
              <span>{osLabel(state.data.os)}</span>
              <span className="text-muted-foreground">Database</span>
              <span className="font-mono text-xs break-all select-text">
                {state.data.databasePath}
              </span>
            </CardContent>
          </Card>
        </>
      )}
    </Page>
  )
}

function ProjectCycler({
  profileId,
  disabled,
  onCycle,
}: {
  profileId: string
  disabled: boolean
  onCycle: (profileId: string, direction: "next" | "previous") => Promise<void>
}) {
  return (
    <>
      <Button
        variant="outline"
        size="sm"
        disabled={disabled}
        onClick={() => void onCycle(profileId, "previous")}
      >
        <ChevronLeft /> Previous
      </Button>
      <Button
        variant="outline"
        size="sm"
        disabled={disabled}
        onClick={() => void onCycle(profileId, "next")}
      >
        Next <ChevronRight />
      </Button>
    </>
  )
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <Card>
      <CardHeader>
        <CardDescription>{label}</CardDescription>
        <CardTitle className="text-2xl tabular-nums">{value}</CardTitle>
      </CardHeader>
    </Card>
  )
}
