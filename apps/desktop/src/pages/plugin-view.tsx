import { useEffect, useState } from "react"

import { EmptyState, Page } from "@/components/page"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
import { Trans, useTranslation } from "@/i18n"
import { useAppContext } from "@/lib/app-context"
import { ActionLibraryView } from "@/pages/actions"
import { VariablesTable } from "@/pages/variables-table"
import { usePluginClientOverride, type PluginClient } from "@/plugins/api"
import { PluginConfiguration } from "@/plugins/configure"
import {
  type PluginNavigationItem,
  pluginVariableNamespace,
  useCurrentPluginView,
} from "@/shell/plugin-navigation"

/**
 * The view a plugin's toolbar entry opens, drawn by the app from the
 * entry's kind (docs/device-controls.md section 4): the plugin's
 * variables, its actions or its settings. Nothing of the plugin is
 * rendered but its declared data.
 */
export function PluginViewPage() {
  const { item } = useCurrentPluginView()
  if (!item) return <GoneView />
  switch (item.view.kind) {
    case "variables":
      return <VariablesView item={item} />
    case "actions":
      return <ActionsView item={item} />
    case "settings":
      return <SettingsView item={item} />
    default:
      // Kinds this app does not draw have no entry; an old link lands here.
      return <GoneView />
  }
}

/** The entry is gone: its plugin was disabled or uninstalled. */
function GoneView() {
  const { t } = useTranslation()
  const { navigate } = useAppContext()
  return (
    <Page title={t("workspace:pluginView.gone.page")}>
      <EmptyState title={t("workspace:pluginView.gone.title")}>
        {t("workspace:pluginView.gone.body")}
        <Button
          variant="outline"
          size="sm"
          className="mt-3"
          onClick={() => {
            navigate("designer")
          }}
        >
          {t("workspace:pluginView.gone.openEditor")}
        </Button>
      </EmptyState>
    </Page>
  )
}

function VariablesView({ item }: { item: PluginNavigationItem }) {
  const { t } = useTranslation()
  const namespace = pluginVariableNamespace(item.plugin)
  const title = t("workspace:pluginView.variables.title", { plugin: item.pluginName })
  return (
    <Page title={item.label} description={title}>
      <SettingsSection
        data-plugin-view="variables"
        title={title}
        description={
          <Trans
            i18nKey="workspace:pluginView.variables.description"
            values={{ namespace, reference: "{{ reference }}" }}
            components={{ code: <code /> }}
          />
        }
      >
        <VariablesTable
          shared={[]}
          selected={null}
          onEdit={() => undefined}
          namespace={namespace}
        />
      </SettingsSection>
    </Page>
  )
}

function ActionsView({ item }: { item: PluginNavigationItem }) {
  const { t } = useTranslation()
  return (
    <Page
      title={item.label}
      description={t("workspace:pluginView.actions.title", { plugin: item.pluginName })}
      profileScoped
    >
      <div data-plugin-view="actions" className="contents">
        <ActionLibraryView plugin={{ id: item.plugin, name: item.pluginName }} />
      </div>
    </Page>
  )
}

function usePluginClient(): PluginClient | null {
  const override = usePluginClientOverride()
  const [loaded, setLoaded] = useState<PluginClient | null>(null)
  useEffect(() => {
    if (override) return
    let active = true
    void import("@/plugins/client").then((m) => {
      if (active) setLoaded(m.createTauriPluginClient())
    })
    return () => {
      active = false
    }
  }, [override])
  return override ?? loaded
}

function SettingsView({ item }: { item: PluginNavigationItem }) {
  const { t } = useTranslation()
  const client = usePluginClient()
  const title = t("workspace:pluginView.settings.title", { plugin: item.pluginName })
  // "Cancel" puts the form back to what is stored.
  const [generation, setGeneration] = useState(0)
  return (
    <Page title={item.label} description={title}>
      <SettingsSection
        data-plugin-view="settings"
        className="max-w-2xl"
        title={title}
        description={t("workspace:pluginView.settings.description")}
      >
        <div className="grid gap-4 border-t pt-4">
          {client && (
            <PluginConfiguration
              key={generation}
              id={item.plugin}
              name={item.pluginName}
              client={client}
              onClose={() => {
                setGeneration((g) => g + 1)
              }}
              onChanged={() => undefined}
            />
          )}
        </div>
      </SettingsSection>
    </Page>
  )
}
