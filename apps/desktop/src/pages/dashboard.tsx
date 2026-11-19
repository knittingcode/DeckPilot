import { ArrowRight, ChevronRight, Download, Info } from "lucide-react"
import { Suspense } from "react"

import { AgentBadge, AgentCompatibilityNotice, AgentStopped, DeviceRow } from "@/components/agent"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { SettingsGroup, SettingsRow, SettingsSection } from "@/components/settings-layout"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
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
    api.setActiveProfile(id).then(refresh, (e: unknown) => {
      toastFailure("The profile was not activated", e)
    })
  }

  return (
    <Page title="Dashboard" description="What is running right now">
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {state.status === "ready" && (
        <div className="mx-auto grid w-full max-w-3xl gap-8">
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

          <SettingsSection title="Now">
            <SettingsGroup>
              <SettingsRow
                label="Active profile"
                description="The profile that runs on the device."
              >
                {profiles.profiles.length > 0 ? (
                  <Select value={activeId ?? undefined} onValueChange={activate}>
                    <SelectTrigger size="sm" className="w-56" aria-label="Active profile">
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
              </SettingsRow>
              {dashboardCards(modules).map(({ key, card: ModuleCard }) => (
                <Suspense key={key} fallback={<Loading />}>
                  <ModuleCard />
                </Suspense>
              ))}
            </SettingsGroup>
          </SettingsSection>

          <SettingsSection
            title={
              <>
                Background agent <AgentBadge />
              </>
            }
            description={
              agent?.status &&
              `Version ${agent.status.version} · ${agent.status.devices.length} device${
                agent.status.devices.length === 1 ? "" : "s"
              }`
            }
          >
            <SettingsGroup>
              {agent && !agent.running && <AgentStopped />}
              {agent?.status &&
                (agent.status.devices.length === 0 ? (
                  <EmptyState variant="plain" title="No Stream Deck detected">
                    Connect a supported device; it is picked up automatically.
                  </EmptyState>
                ) : (
                  agent.status.devices.map((device) => (
                    <DeviceRow key={device.id.serial} device={device} />
                  ))
                ))}
            </SettingsGroup>
          </SettingsSection>

          {active.status === "ready" && active.data && (
            <SettingsSection title={`In ${active.data.profile.name}`}>
              <SettingsGroup>
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
                    className="flex w-full items-center gap-3 rounded-sm py-2 text-left text-sm outline-none hover:bg-muted/50 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                    onClick={() => {
                      navigate(target)
                    }}
                  >
                    <span className="flex-1 font-medium">{label}</span>
                    <span className="text-muted-foreground tabular-nums">{count}</span>
                    <ChevronRight aria-hidden className="size-4 text-muted-foreground" />
                  </button>
                ))}
              </SettingsGroup>
            </SettingsSection>
          )}
        </div>
      )}
    </Page>
  )
}
