import { useEffect, useState } from "react"

import { EmptyState, Page } from "@/components/page"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
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
  const { navigate } = useAppContext()
  return (
    <Page title="Plugin view">
      <EmptyState title="This plugin view is not available">
        Its plugin was disabled or uninstalled, or no longer offers it.
        <Button
          variant="outline"
          size="sm"
          className="mt-3"
          onClick={() => {
            navigate("designer")
          }}
        >
          Open the Device Editor
        </Button>
      </EmptyState>
    </Page>
  )
}

function VariablesView({ item }: { item: PluginNavigationItem }) {
  const namespace = pluginVariableNamespace(item.plugin)
  return (
    <Page title={item.label} description={`Variables of ${item.pluginName}`}>
      <SettingsSection
        data-plugin-view="variables"
        title={`Variables of ${item.pluginName}`}
        description={
          <>
            Live values the plugin publishes under <code>{namespace}</code>. Insert one in an action
            or a key label with {"{{ reference }}"}.
          </>
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
  return (
    <Page title={item.label} description={`Actions of ${item.pluginName}`} profileScoped>
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
  const client = usePluginClient()
  // "Cancel" puts the form back to what is stored.
  const [generation, setGeneration] = useState(0)
  return (
    <Page title={item.label} description={`Settings of ${item.pluginName}`}>
      <SettingsSection
        data-plugin-view="settings"
        className="max-w-2xl"
        title={`Settings of ${item.pluginName}`}
        description="Stored by the agent for this plugin only. Also in Settings › Extensions › Plugins."
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
