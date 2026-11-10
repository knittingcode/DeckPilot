import { ErrorNotice, Loading } from "@/components/page"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { osLabel } from "@/lib/format"

export function AboutSettings() {
  const { state } = useAsync(() => Promise.all([api.getOverview(), api.launchInfo()]), "about")
  return (
    <Card>
      <CardHeader>
        <CardTitle>DeckPilot</CardTitle>
        <CardDescription>
          Stream Deck management for macOS, Windows and Linux. Open source under the Apache License
          2.0.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {state.status === "loading" && <Loading />}
        {state.status === "error" && <ErrorNotice error={state.error} />}
        {state.status === "ready" && (
          <dl className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-[10rem_1fr]">
            <dt className="text-muted-foreground">Version</dt>
            <dd>
              {state.data[0].appVersion}
              {state.data[0].development && " (development session)"}
            </dd>
            <dt className="text-muted-foreground">Previous version</dt>
            <dd>{state.data[1].launch?.previousVersion ?? "—"}</dd>
            <dt className="text-muted-foreground">Operating system</dt>
            <dd>{osLabel(state.data[0].os)}</dd>
            <dt className="text-muted-foreground">Source code</dt>
            <dd className="select-text">github.com/knittingcode/DeckPilot</dd>
          </dl>
        )}
      </CardContent>
    </Card>
  )
}
