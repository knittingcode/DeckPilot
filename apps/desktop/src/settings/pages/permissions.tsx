import { InlineStatus, SettingsGroup, SettingsRow } from "@/components/settings-layout"
import { useAppContext } from "@/lib/app-context"
import { CAPABILITIES } from "@/lib/capabilities"
import type { Capability } from "@/lib/types"

/**
 * Capabilities declared in module manifests. They are fixed by each
 * module; disabling a module withdraws all of them.
 */
export function PermissionsSettings() {
  const { modules, openSettings } = useAppContext()
  const capabilities = Object.keys(CAPABILITIES) as Capability[]
  return (
    <div className="grid gap-3">
      <p className="text-sm text-muted-foreground">
        Every module declares what it may access. A disabled module has no access. To withdraw a
        permission, disable the module under{" "}
        <button
          type="button"
          className="text-primary underline-offset-2 hover:underline"
          onClick={() => {
            openSettings("modules")
          }}
        >
          Modules
        </button>
        .
      </p>
      <SettingsGroup>
        {capabilities.map((capability) => {
          const users = modules.filter((m) => m.capabilities.includes(capability))
          return (
            <SettingsRow
              key={capability}
              align="start"
              label={CAPABILITIES[capability].title}
              description={CAPABILITIES[capability].description}
            >
              {users.length === 0 ? (
                <p className="w-44 text-caption text-muted-foreground">
                  No module uses this permission.
                </p>
              ) : (
                <ul className="grid w-44 gap-1 text-sm" aria-label={CAPABILITIES[capability].title}>
                  {users.map((module) => (
                    <li key={module.id} className="flex items-center justify-between gap-3">
                      <span className="truncate">{module.name}</span>
                      <InlineStatus tone={module.enabled ? "positive" : "neutral"}>
                        {module.enabled ? "Granted" : "Disabled"}
                      </InlineStatus>
                    </li>
                  ))}
                </ul>
              )}
            </SettingsRow>
          )
        })}
      </SettingsGroup>
    </div>
  )
}
