import { ErrorNotice, Loading } from "@/components/page"
import { Property, PropertyList, SettingsSection } from "@/components/settings-layout"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { osLabel } from "@/lib/format"
import { useTranslation } from "@/i18n"

export function AboutSettings() {
  const { t } = useTranslation()
  const { state } = useAsync(() => Promise.all([api.getOverview(), api.launchInfo()]), "about")
  return (
    <SettingsSection title="DeckPilot" description={t("settings:about.description")}>
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {state.status === "ready" && (
        <PropertyList className="border-t pt-3">
          <Property label={t("settings:about.version")}>
            {state.data[0].appVersion}
            {state.data[0].development && ` ${t("settings:about.development")}`}
          </Property>
          <Property label={t("settings:about.previousVersion")}>
            {state.data[1].launch?.previousVersion ?? "—"}
          </Property>
          <Property label={t("settings:about.os")}>{osLabel(state.data[0].os)}</Property>
          <Property label={t("settings:about.source")}>github.com/knittingcode/deckpilot</Property>
        </PropertyList>
      )}
    </SettingsSection>
  )
}
