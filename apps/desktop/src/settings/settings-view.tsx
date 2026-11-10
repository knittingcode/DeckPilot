import { Page } from "@/components/page"
import { useAppContext } from "@/lib/app-context"
import { ModuleSettingsForm } from "@/settings/pages/module-settings"
import { CATEGORIES, findSettingsPage } from "@/settings/registry"

/** The settings page `id`; the navigation only opens pages that exist. */
export function SettingsView({ id }: { id: string }) {
  const { modules, openSettings } = useAppContext()
  const page = findSettingsPage(modules, id) ?? findSettingsPage(modules, "appearance")
  if (!page) return null
  const View = page.view
  return (
    <Page
      title={page.title}
      description={page.description}
      breadcrumbs={[
        {
          label: "Settings",
          onSelect: () => {
            openSettings("appearance")
          },
        },
        { label: CATEGORIES.find((c) => c.id === page.category)?.label ?? "" },
      ]}
    >
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
