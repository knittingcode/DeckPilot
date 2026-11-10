import { Page } from "@/components/page"
import { useAppContext } from "@/lib/app-context"
import { ModuleSettingsForm } from "@/settings/pages/module-settings"
import { findSettingsPage } from "@/settings/registry"

/** The settings page `id`; the navigation only opens pages that exist. */
export function SettingsView({ id }: { id: string }) {
  const { modules } = useAppContext()
  const page = findSettingsPage(modules, id) ?? findSettingsPage(modules, "appearance")
  if (!page) return null
  const View = page.view
  return (
    <Page title={page.title} description={page.description}>
      <div className="mx-auto grid w-full max-w-3xl gap-6">
        {View ? (
          <View />
        ) : (
          page.module && <ModuleSettingsForm key={page.module.id} module={page.module} />
        )}
      </div>
    </Page>
  )
}
