import { MigrationNotesCard } from "@/components/migration-notes"
import { ErrorNotice, Loading } from "@/components/page"
import { Property, PropertyList, SettingsSection } from "@/components/settings-layout"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { Trans, useTranslation } from "@/i18n"

export function StorageSettings() {
  const { t } = useTranslation()
  const { revision, agent, openSettings } = useAppContext()
  const { state } = useAsync(() => api.getOverview(), String(revision))
  return (
    <>
      <SettingsSection
        title={t("settings:storage.title")}
        description={
          <Trans
            i18nKey="settings:storage.description"
            components={{
              page: (
                <button
                  type="button"
                  className="text-primary underline-offset-2 hover:underline"
                  onClick={() => {
                    openSettings("security")
                  }}
                />
              ),
            }}
          />
        }
      >
        {state.status === "loading" && <Loading />}
        {state.status === "error" && <ErrorNotice error={state.error} />}
        {state.status === "ready" && (
          <PropertyList className="border-t pt-3">
            <Property label={t("settings:storage.location")} mono>
              {state.data.databasePath}
            </Property>
            <Property label={t("settings:storage.encryption")}>SQLCipher (AES-256)</Property>
            {agent?.status && (
              <Property label={t("settings:storage.agentData")} mono>
                {agent.status.dataDir}
              </Property>
            )}
            <Property label={t("settings:storage.contents")}>
              {t("settings:storage.contentsValue", {
                profiles: t("settings:storage.profiles", { count: state.data.profileCount }),
                bindings: t("settings:storage.bindings", {
                  count: state.data.resourceBindingCount,
                }),
              })}
            </Property>
          </PropertyList>
        )}
      </SettingsSection>
      <MigrationNotesCard />
    </>
  )
}
