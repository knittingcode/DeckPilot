import { ArrowRight, Download, Info } from "lucide-react"
import { Suspense } from "react"
import { toast } from "sonner"

import { AgentBadge, AgentCompatibilityNotice, AgentStopped, DeviceRow } from "@/components/agent"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useAsync } from "@/hooks/use-async"
import { api, errorMessage } from "@/lib/api"
import { type PageId, useAppContext } from "@/lib/app-context"
import { dashboardCards } from "@/modules"

export function DashboardPage() {
  const { revision, profiles, refresh, agent, updates, navigate, openSettings, modules } =
    useAppContext()
  const { state } = useAsync(
    () => Promise.all([api.getOverview(), api.launchInfo()]),
    String(revision),
  )
  const activeId = profiles.activeProfileId
  const active = useAsync(
    () => (activeId ? api.getProfile(activeId) : Promise.resolve(null)),
    `${activeId ?? ""}:${revision}`,
  ).state

  const activate = (id: string) => {
    api.setActiveProfile(id).then(refresh, (e: unknown) => toast.error(errorMessage(e)))
  }

  return (
    <Page title="Dashboard" description="What is running right now">
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {state.status === "ready" && (
        <>
          {state.data[1].launch?.firstAfterUpdate && (
            <Alert>
              <Info />
              <AlertTitle>DeckPilot was updated</AlertTitle>
              <AlertDescription>
                Updated from {state.data[1].launch.previousVersion} to{" "}
                {state.data[1].launch.currentVersion}. Your configuration was kept.
              </AlertDescription>
            </Alert>
          )}
          {updates?.available && (
            <Alert>
              <Download />
              <AlertTitle>Version {updates.available.version} is available</AlertTitle>
              <AlertDescription>
                <Button
                  size="sm"
                  variant="outline"
                  className="mt-2"
                  onClick={() => {
                    openSettings("updates")
                  }}
                >
                  View update <ArrowRight />
                </Button>
              </AlertDescription>
            </Alert>
          )}
          <AgentCompatibilityNotice />

          <div className="grid gap-4 lg:grid-cols-2">
            <Card>
              <CardHeader>
                <CardDescription>Active profile</CardDescription>
                <CardTitle className="text-xl">
                  {state.data[0].activeProfile?.name ?? "None"}
                </CardTitle>
              </CardHeader>
              <CardContent>
                {profiles.profiles.length > 0 ? (
                  <Select value={activeId ?? undefined} onValueChange={activate}>
                    <SelectTrigger className="w-60" aria-label="Active profile">
                      <SelectValue placeholder="Choose a profile" />
                    </SelectTrigger>
                    <SelectContent>
                      {profiles.profiles.map((profile) => (
                        <SelectItem key={profile.id} value={profile.id}>
                          {profile.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ) : (
                  <Button
                    size="sm"
                    onClick={() => {
                      navigate("profiles")
                    }}
                  >
                    Create a profile
                  </Button>
                )}
              </CardContent>
            </Card>
            {dashboardCards(modules).map(({ key, card: ModuleCard }) => (
              <Suspense key={key} fallback={<Loading />}>
                <ModuleCard />
              </Suspense>
            ))}
          </div>

          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                Background agent <AgentBadge />
              </CardTitle>
              {agent?.status && (
                <CardDescription>
                  Version {agent.status.version} · {agent.status.devices.length} device
                  {agent.status.devices.length === 1 ? "" : "s"}
                </CardDescription>
              )}
            </CardHeader>
            <CardContent className="grid gap-2">
              {agent && !agent.running && <AgentStopped />}
              {agent?.status &&
                (agent.status.devices.length === 0 ? (
                  <EmptyState title="No Stream Deck detected">
                    Connect a supported device; it is picked up automatically.
                  </EmptyState>
                ) : (
                  agent.status.devices.map((device) => (
                    <DeviceRow key={device.id.serial} device={device} />
                  ))
                ))}
            </CardContent>
          </Card>

          {active.status === "ready" && active.data && (
            <div className="grid gap-4 sm:grid-cols-3">
              {(
                [
                  ["Actions", active.data.profile.actions.length, "actions"],
                  ["Workflows", active.data.profile.workflows.length, "workflows"],
                  ["Layouts", active.data.profile.layouts.length, "designer"],
                ] as [string, number, PageId][]
              ).map(([label, count, target]) => (
                <button
                  key={label}
                  type="button"
                  className="rounded-xl text-left focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  onClick={() => {
                    navigate(target)
                  }}
                >
                  <Card className="transition-colors hover:bg-accent/50">
                    <CardHeader>
                      <CardDescription>{label} in the active profile</CardDescription>
                      <CardTitle className="text-2xl tabular-nums">{count}</CardTitle>
                    </CardHeader>
                  </Card>
                </button>
              ))}
            </div>
          )}
        </>
      )}
    </Page>
  )
}
