import { ErrorNotice, Loading } from "@/components/page"
import { Property, PropertyList, SettingsSection } from "@/components/settings-layout"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { osLabel } from "@/lib/format"

export function AboutSettings() {
  const { state } = useAsync(() => Promise.all([api.getOverview(), api.launchInfo()]), "about")
  return (
    <SettingsSection
      title="DeckPilot"
      description="Stream Deck management for macOS, Windows and Linux. Open source under the Apache License 2.0."
    >
      {state.status === "loading" && <Loading />}
      {state.status === "error" && <ErrorNotice error={state.error} />}
      {state.status === "ready" && (
        <PropertyList className="border-t pt-3">
          <Property label="Version">
            {state.data[0].appVersion}
            {state.data[0].development && " (development session)"}
          </Property>
          <Property label="Previous version">
            {state.data[1].launch?.previousVersion ?? "—"}
          </Property>
          <Property label="Operating system">{osLabel(state.data[0].os)}</Property>
          <Property label="Source code">github.com/knittingcode/deckpilot</Property>
        </PropertyList>
      )}
    </SettingsSection>
  )
}
