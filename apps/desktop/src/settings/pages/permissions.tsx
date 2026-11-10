import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
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
    <>
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
      {capabilities.map((capability) => {
        const users = modules.filter((m) => m.capabilities.includes(capability))
        return (
          <Card key={capability}>
            <CardHeader>
              <CardTitle>{CAPABILITIES[capability].title}</CardTitle>
              <CardDescription>{CAPABILITIES[capability].description}</CardDescription>
            </CardHeader>
            <CardContent>
              {users.length === 0 ? (
                <p className="text-sm text-muted-foreground">No module uses this permission.</p>
              ) : (
                <ul className="grid gap-1 text-sm" aria-label={CAPABILITIES[capability].title}>
                  {users.map((module) => (
                    <li key={module.id} className="flex items-center gap-2">
                      <span>{module.name}</span>
                      <Badge variant={module.enabled ? "secondary" : "outline"}>
                        {module.enabled ? "Granted" : "Disabled"}
                      </Badge>
                    </li>
                  ))}
                </ul>
              )}
            </CardContent>
          </Card>
        )
      })}
    </>
  )
}
