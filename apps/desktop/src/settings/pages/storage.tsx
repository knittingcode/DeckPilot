import { MigrationNotesCard } from "@/components/migration-notes"
import { ErrorNotice, Loading } from "@/components/page"
import { Property, PropertyList, SettingsSection } from "@/components/settings-layout"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"

export function StorageSettings() {
  const { revision, agent, openSettings } = useAppContext()
  const { state } = useAsync(() => api.getOverview(), String(revision))
  return (
    <>
      <SettingsSection
        title="Configuration database"
        description={
          <>
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
          </>
        }
      >
        {state.status === "loading" && <Loading />}
        {state.status === "error" && <ErrorNotice error={state.error} />}
        {state.status === "ready" && (
          <PropertyList className="border-t pt-3">
            <Property label="Location" mono>
              {state.data.databasePath}
            </Property>
            <Property label="Encryption">SQLCipher (AES-256)</Property>
            {agent?.status && (
              <Property label="Agent data" mono>
                {agent.status.dataDir}
              </Property>
            )}
            <Property label="Contents">
              {state.data.profileCount} profile{state.data.profileCount === 1 ? "" : "s"},{" "}
              {state.data.resourceBindingCount} resource binding
              {state.data.resourceBindingCount === 1 ? "" : "s"} of this computer
            </Property>
          </PropertyList>
        )}
      </SettingsSection>
      <MigrationNotesCard />
    </>
  )
}
