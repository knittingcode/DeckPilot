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
import { type TranslationKey, useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { type PageId, useAppContext } from "@/lib/app-context"
import { usePluginFeatures } from "@/lib/plugin-features"
import { dashboardCards } from "@/modules"
import { hostDashboardCards } from "@/modules/host-views"

export function DashboardPage() {
  const { t } = useTranslation()
  const format = useFormat()
  const { revision, profiles, refresh, agent, updates, navigate, openSettings, modules } =
    useAppContext()
  const features = usePluginFeatures()
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
      toastFailure(t("workspace:dashboard.activateFailed"), e)
    })
  }

  return (
    <Page title={t("workspace:dashboard.title")} description={t("workspace:dashboard.description")}>
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {state.status === "ready" && (
        <div className="mx-auto grid w-full max-w-3xl gap-8">
          {state.data[1].launch?.firstAfterUpdate && (
            <Alert>
              <Info />
              <AlertTitle>{t("workspace:dashboard.updated.title")}</AlertTitle>
              <AlertDescription>
                {t("workspace:dashboard.updated.description", {
                  from: state.data[1].launch.previousVersion,
                  to: state.data[1].launch.currentVersion,
                })}
              </AlertDescription>
            </Alert>
          )}
          {updates?.available && (
            <Alert>
              <Download />
              <AlertTitle>
                {t("workspace:dashboard.updateAvailable", { version: updates.available.version })}
              </AlertTitle>
              <AlertDescription>
                <Button
                  size="sm"
                  variant="outline"
                  className="mt-2"
                  onClick={() => {
                    openSettings("updates")
                  }}
                >
                  {t("workspace:dashboard.viewUpdate")} <ArrowRight />
                </Button>
              </AlertDescription>
            </Alert>
          )}
          <AgentCompatibilityNotice />

          <SettingsSection title={t("workspace:dashboard.now")}>
            <SettingsGroup>
              <SettingsRow
                label={t("workspace:dashboard.activeProfile")}
                description={t("workspace:dashboard.activeProfileHint")}
              >
                {profiles.profiles.length > 0 ? (
                  <Select value={activeId ?? undefined} onValueChange={activate}>
                    <SelectTrigger
                      size="sm"
                      className="w-56"
                      aria-label={t("workspace:dashboard.activeProfile")}
                    >
                      <SelectValue placeholder={t("workspace:dashboard.chooseProfile")} />
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
                    {t("workspace:dashboard.createProfile")}
                  </Button>
                )}
              </SettingsRow>
              {[...hostDashboardCards(features), ...dashboardCards(modules)].map(
                ({ key, card: ModuleCard }) => (
                  <Suspense key={key} fallback={<Loading />}>
                    <ModuleCard />
                  </Suspense>
                ),
              )}
            </SettingsGroup>
          </SettingsSection>

          <SettingsSection
            title={
              <>
                {t("workspace:dashboard.agent")} <AgentBadge />
              </>
            }
            description={
              agent?.status &&
              t("workspace:dashboard.agentSummary", {
                version: agent.status.version,
                count: agent.status.devices.length,
              })
            }
          >
            <SettingsGroup>
              {agent && !agent.running && <AgentStopped />}
              {agent?.status &&
                (agent.status.devices.length === 0 ? (
                  <EmptyState variant="plain" title={t("workspace:dashboard.noDevice.title")}>
                    {t("workspace:dashboard.noDevice.description")}
                  </EmptyState>
                ) : (
                  agent.status.devices.map((device) => (
                    <DeviceRow key={device.id.serial} device={device} />
                  ))
                ))}
            </SettingsGroup>
          </SettingsSection>

          {active.status === "ready" && active.data && (
            <SettingsSection
              title={t("workspace:dashboard.inProfile", { profile: active.data.profile.name })}
            >
              <SettingsGroup>
                {(
                  [
                    [
                      "workspace:dashboard.counts.actions",
                      active.data.profile.actions.length,
                      "actions",
                    ],
                    // The Workflows screen only while its plugin shows it.
                    ...(features.hostViews.has("workflows")
                      ? [
                          [
                            "workspace:dashboard.counts.workflows",
                            active.data.profile.workflows.length,
                            "workflows",
                          ],
                        ]
                      : []),
                    [
                      "workspace:dashboard.counts.layouts",
                      active.data.profile.layouts.length,
                      "designer",
                    ],
                  ] as [TranslationKey, number, PageId][]
                ).map(([label, count, target]) => (
                  <button
                    key={label}
                    type="button"
                    className="flex w-full items-center gap-3 rounded-sm py-2 text-left text-sm outline-none hover:bg-muted/50 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                    onClick={() => {
                      navigate(target)
                    }}
                  >
                    <span className="flex-1 font-medium">{t(label)}</span>
                    <span className="text-muted-foreground tabular-nums">
                      {format.number(count)}
                    </span>
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
