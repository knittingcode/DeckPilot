import { Page } from "@/components/page"
import { useTranslation } from "@/i18n"
import { PluginsSettings } from "@/plugins/marketplace"

/**
 * The plugin marketplace as a workspace of the main window, opened from
 * the toolbar: the catalog first. Installed plugins are also managed in
 * Settings › Extensions › Plugins. Only in builds with plugins.
 */
export function MarketplacePage() {
  const { t } = useTranslation()
  return (
    <Page title={t("marketplace:page.title")} description={t("marketplace:page.description")}>
      <div className="mx-auto grid w-full max-w-5xl gap-6">
        <PluginsSettings />
      </div>
    </Page>
  )
}
