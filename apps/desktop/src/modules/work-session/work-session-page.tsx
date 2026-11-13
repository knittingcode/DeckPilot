import { Keyboard } from "lucide-react"
import { useState } from "react"

import { ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
import { useEditingProfile } from "@/hooks/use-profile"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import type { WorkSessionSettings, WorkSessionSnapshot } from "@/lib/types"

import { HistoryCard } from "./history-card"
import { DEFAULT_SETTINGS } from "./model"
import { SettingsCard } from "./settings-card"
import { StatusCard } from "./status-card"
import { useWorkSession } from "./use-work-session"

/**
 * The work session: its live state, its settings and its history. There
 * are no buttons that start, check or finish a session; only its key does
 * that (D11).
 */
export function WorkSessionPage() {
  return (
    <Page
      title="Work Session"
      description="Time worked, started and finished from a key on the device"
      profileScoped
    >
      <WorkSession />
    </Page>
  )
}

function WorkSession() {
  const { view, replace } = useWorkSession()
  if (view.status === "loading") return <Loading />
  if (view.status === "unsupported") {
    return (
      <Alert>
        <AlertTitle>Work sessions need a newer agent</AlertTitle>
        <AlertDescription>
          The running agent does not support work sessions. Restart the agent in Settings › Agent so
          the one installed with this app runs.
        </AlertDescription>
      </Alert>
    )
  }
  if (view.status === "error") return <ErrorNotice error={view.error} />
  const snapshot = view.snapshot
  return (
    <RequireProfile>
      {(profileId) => (
        <Workspace
          profileId={profileId}
          snapshot={snapshot}
          onSnapshot={replace}
          reloadKey={`${snapshot.session ?? "none"}:${snapshot.state}`}
        />
      )}
    </RequireProfile>
  )
}

function Workspace({
  profileId,
  snapshot,
  onSnapshot,
  reloadKey,
}: {
  profileId: string
  snapshot: WorkSessionSnapshot
  onSnapshot: (snapshot: WorkSessionSnapshot) => void
  reloadKey: string
}) {
  const { state: profile } = useEditingProfile()
  const settings = useAsync(() => api.getWorkSessionSettings(profileId), profileId)
  const [saved, setSaved] = useState<WorkSessionSettings | null>(null)
  const current: WorkSessionSettings =
    saved ?? (settings.state.status === "ready" ? settings.state.data : DEFAULT_SETTINGS)
  const projects = profile.status === "ready" ? profile.data.profile.projects : []
  const projectName = (id: string) => {
    const project = projects.find((p) => p.id === id)
    return project ? `${project.key} · ${project.name}` : "Project of another profile"
  }
  return (
    <>
      <StatusCard
        snapshot={snapshot}
        format={current.elapsedFormat}
        projectName={projectName}
        onChange={onSnapshot}
      />
      {profile.status === "error" ? (
        <ErrorNotice error={profile.error} />
      ) : (
        <SettingsCard
          profileId={profileId}
          settings={settings.state}
          workflows={profile.status === "ready" ? profile.data.profile.workflows : []}
          onSaved={setSaved}
        />
      )}
      <AssignCard />
      <HistoryCard reloadKey={reloadKey} projectName={projectName} />
    </>
  )
}

/** How a key gets the work session; the assignment itself is the gesture editor's. */
function AssignCard() {
  const { navigate } = useAppContext()
  return (
    <Card>
      <CardHeader>
        <CardTitle>Work Session key</CardTitle>
        <CardDescription>
          Any key can control the work session; no action is created for it.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3 text-sm">
        <ol className="grid list-decimal gap-1 pl-5">
          <li>Open the Layout Designer and select a key.</li>
          <li>
            In its gestures, choose <span className="font-medium">Changes with state</span> and then{" "}
            <span className="font-medium">Work Session</span>.
          </li>
          <li>
            The key then shows the state and the elapsed time. Click it to start; when working,
            click it to run the end check, or hold it if no verification workflow is set; click it
            again when it shows Finish Work.
          </li>
        </ol>
        <div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              navigate("designer")
            }}
          >
            <Keyboard aria-hidden />
            Open Layout Designer
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}
