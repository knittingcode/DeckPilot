import { MigrationNotesCard } from "@/components/migration-notes"
import { ErrorNotice, Loading } from "@/components/page"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"

export function StorageSettings() {
  const { revision, agent, openSettings } = useAppContext()
  const { state } = useAsync(() => api.getOverview(), String(revision))
  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Configuration database</CardTitle>
          <CardDescription>
            Profiles, projects, actions, workflows and layouts are stored on this computer only, in
            an encrypted SQLite database. How it is unlocked is set under{" "}
            <button
              type="button"
              className="text-primary underline-offset-2 hover:underline"
              onClick={() => {
                openSettings("security")
              }}
            >
              Security
            </button>
            .
          </CardDescription>
        </CardHeader>
        <CardContent>
          {state.status === "loading" && <Loading />}
          {state.status === "error" && <ErrorNotice error={state.error} />}
          {state.status === "ready" && (
            <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-[10rem_1fr]">
              <dt className="text-muted-foreground">Location</dt>
              <dd className="font-mono text-xs break-all select-text">{state.data.databasePath}</dd>
              <dt className="text-muted-foreground">Encryption</dt>
              <dd>SQLCipher (AES-256)</dd>
              {agent?.status && (
                <>
                  <dt className="text-muted-foreground">Agent data</dt>
                  <dd className="font-mono text-xs break-all select-text">
                    {agent.status.dataDir}
                  </dd>
                </>
              )}
              <dt className="text-muted-foreground">Contents</dt>
              <dd>
                {state.data.profileCount} profile{state.data.profileCount === 1 ? "" : "s"},{" "}
                {state.data.resourceBindingCount} resource binding
                {state.data.resourceBindingCount === 1 ? "" : "s"} of this computer
              </dd>
            </dl>
          )}
        </CardContent>
      </Card>
      <MigrationNotesCard />
    </>
  )
}
