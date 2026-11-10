import { Puzzle } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ParameterField } from "@/components/parameter-field"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Switch } from "@/components/ui/switch"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { osLabel } from "@/lib/format"
import { initialValue } from "@/lib/parameters"
import type { Capability, ModuleInfo, ParameterValue } from "@/lib/types"

const CAPABILITIES: Record<Capability, string> = {
  "profile-read": "Reads the active profile",
  "project-context": "Changes the active project",
  "machine-resources": "Reads this computer's resource bindings",
}

/** Built-in feature modules: enable or disable them and edit their settings. */
export function ModulesCard() {
  const { modules, setModules, refresh } = useAppContext()
  const apply = (change: Promise<ModuleInfo[]>, message: string) => {
    change.then(
      (next) => {
        setModules(next)
        refresh()
        toast.success(message)
      },
      (e: unknown) => toast.error(errorMessage(e)),
    )
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Puzzle className="size-4" /> Modules
        </CardTitle>
        <CardDescription>
          Features are built in as modules. Disabling a module hides its pages and action types;
          actions you created with it are kept and shown as unavailable.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3">
        {modules.map((module) => (
          <ModuleRow
            key={module.id}
            module={module}
            dependents={modules.filter((m) => m.enabled && m.dependencies.includes(module.id))}
            onToggle={(enabled) => {
              apply(
                api.setModuleEnabled(module.id, enabled),
                `${module.name} ${enabled ? "enabled" : "disabled"}`,
              )
            }}
            onConfigure={(key, value) => {
              apply(
                api.setModuleConfig(module.id, { ...module.configValues, [key]: value }),
                `${module.name} settings saved`,
              )
            }}
          />
        ))}
      </CardContent>
    </Card>
  )
}

function ModuleRow({
  module,
  dependents,
  onToggle,
  onConfigure,
}: {
  module: ModuleInfo
  dependents: ModuleInfo[]
  onToggle: (enabled: boolean) => void
  onConfigure: (key: string, value: ParameterValue) => void
}) {
  const [showDetails, setShowDetails] = useState(false)
  const switchId = `module-${module.id}`
  const blocked = module.enabled && dependents.length > 0
  return (
    <section className="grid gap-2 rounded-lg border p-3" aria-labelledby={`${switchId}-name`}>
      <div className="flex items-start gap-3">
        <div className="grid flex-1 gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <span id={`${switchId}-name`} className="font-medium">
              {module.name}
            </span>
            <span className="text-xs text-muted-foreground">v{module.version}</span>
            {!module.supported && <Badge variant="outline">Not available on this system</Badge>}
          </div>
          <p className="text-sm text-muted-foreground">{module.description}</p>
          <p className="text-xs text-muted-foreground">
            {module.actionTypes.length} action type{module.actionTypes.length === 1 ? "" : "s"}
            {module.pages.length > 0 && ` · pages: ${module.pages.map((p) => p.title).join(", ")}`}
            {` · ${module.platforms.length === 0 ? "all platforms" : module.platforms.map(osLabel).join(", ")}`}
          </p>
        </div>
        <Switch
          id={switchId}
          checked={module.enabled}
          disabled={!module.supported || blocked}
          aria-label={`${module.enabled ? "Disable" : "Enable"} ${module.name}`}
          title={blocked ? `Required by ${dependents.map((d) => d.name).join(", ")}` : undefined}
          onCheckedChange={onToggle}
        />
      </div>
      <button
        type="button"
        className="justify-self-start text-xs text-primary underline-offset-2 hover:underline"
        aria-expanded={showDetails}
        onClick={() => {
          setShowDetails(!showDetails)
        }}
      >
        {showDetails ? "Hide details" : "Details and settings"}
      </button>
      {showDetails && (
        <div className="grid gap-3 border-t pt-3 text-sm">
          <div>
            <h4 className="text-xs font-medium text-muted-foreground uppercase">Permissions</h4>
            <ul className="mt-1 list-disc pl-5">
              {module.capabilities.map((capability) => (
                <li key={capability}>{CAPABILITIES[capability]}</li>
              ))}
            </ul>
          </div>
          <div>
            <h4 className="text-xs font-medium text-muted-foreground uppercase">Action types</h4>
            <ul className="mt-1 grid gap-0.5">
              {module.actionTypes.map((type) => (
                <li key={type.id}>
                  {type.name}{" "}
                  <span className="font-mono text-xs text-muted-foreground">{type.id}</span>
                </li>
              ))}
            </ul>
          </div>
          {module.dependencies.length > 0 && (
            <p className="text-xs text-muted-foreground">
              Requires: {module.dependencies.join(", ")}
            </p>
          )}
          {module.config.map((spec) => (
            <ParameterField
              key={spec.key}
              spec={spec}
              value={module.configValues[spec.key] ?? initialValue(spec)}
              onChange={(value) => {
                onConfigure(spec.key, value)
              }}
            />
          ))}
        </div>
      )}
    </section>
  )
}
