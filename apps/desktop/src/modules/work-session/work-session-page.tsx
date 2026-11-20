import { Keyboard } from "lucide-react"
import { useState } from "react"

import { ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
import { useAsync } from "@/hooks/use-async"
import { useEditingProfile } from "@/hooks/use-profile"
import { Trans, useTranslation } from "@/i18n"
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
  const { t } = useTranslation()
  return (
    <Page
      title={t("workspace:workSession.page.title")}
      description={t("workspace:workSession.page.description")}
      profileScoped
    >
      <WorkSession />
    </Page>
  )
}

function WorkSession() {
  const { t } = useTranslation()
  const { view, replace } = useWorkSession()
  if (view.status === "loading") return <Loading />
  if (view.status === "unsupported") {
    return (
      <Alert>
        <AlertTitle>{t("workspace:workSession.page.unsupportedTitle")}</AlertTitle>
        <AlertDescription>
          {t("workspace:workSession.page.unsupportedDescription")}
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
  const { t } = useTranslation()
  const { state: profile } = useEditingProfile()
  const settings = useAsync(() => api.getWorkSessionSettings(profileId), profileId)
  const [saved, setSaved] = useState<WorkSessionSettings | null>(null)
  const current: WorkSessionSettings =
    saved ?? (settings.state.status === "ready" ? settings.state.data : DEFAULT_SETTINGS)
  const projects = profile.status === "ready" ? profile.data.profile.projects : []
  const projectName = (id: string) => {
    const project = projects.find((p) => p.id === id)
    return project
      ? t("workspace:projects.option", { key: project.key, project: project.name })
      : t("workspace:workSession.page.otherProject")
  }
  return (
    <div className="mx-auto grid w-full max-w-3xl gap-8">
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
    </div>
  )
}

/** How a key gets the work session; the assignment itself is the gesture editor's. */
function AssignCard() {
  const { t } = useTranslation()
  const { navigate } = useAppContext()
  return (
    <SettingsSection
      title={t("workspace:workSession.page.key.title")}
      description={t("workspace:workSession.page.key.description")}
    >
      <div className="grid gap-3 text-sm">
        <ol className="grid list-decimal gap-1 pl-5">
          <li>{t("workspace:workSession.page.key.step1")}</li>
          <li>
            <Trans
              i18nKey="workspace:workSession.page.key.step2"
              components={{ em: <span className="font-medium" /> }}
            />
          </li>
          <li>{t("workspace:workSession.page.key.step3")}</li>
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
            {t("workspace:workSession.page.key.open")}
          </Button>
        </div>
      </div>
    </SettingsSection>
  )
}
