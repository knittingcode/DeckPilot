import { Page } from "@/components/page"
import { PluginsSettings } from "@/plugins/marketplace"

/**
 * The plugin marketplace as a workspace of the main window, opened from
 * the toolbar: the catalog first. Installed plugins are also managed in
 * Settings › Extensions › Plugins. Only in builds with plugins.
 */
export function MarketplacePage() {
  return (
    <Page title="Marketplace" description="Signed community plugins from the verified registry">
      <div className="mx-auto grid w-full max-w-5xl gap-6">
        <PluginsSettings />
      </div>
    </Page>
  )
}
