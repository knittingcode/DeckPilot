import { useState } from "react"
import { toast } from "sonner"

import { ParameterField } from "@/components/parameter-field"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { initialValue } from "@/lib/parameters"
import type { ActionParameters, ModuleInfo } from "@/lib/types"

/** Settings page generated from the settings a module declares in its manifest. */
export function ModuleSettingsForm({ module }: { module: ModuleInfo }) {
  const { setModules, refresh } = useAppContext()
  const [draft, setDraft] = useState<ActionParameters>(module.configValues)
  const [saving, setSaving] = useState(false)
  const changed = JSON.stringify(draft) !== JSON.stringify(module.configValues)
  return (
    <Card>
      <CardHeader>
        <CardTitle>{module.name}</CardTitle>
        <CardDescription>{module.description}</CardDescription>
      </CardHeader>
      <CardContent className="grid max-w-xl gap-4">
        {module.config.map((spec) => (
          <ParameterField
            key={spec.key}
            spec={spec}
            value={draft[spec.key] ?? initialValue(spec)}
            onChange={(value) => {
              setDraft({ ...draft, [spec.key]: value })
            }}
          />
        ))}
        <div className="flex gap-2">
          <Button
            disabled={!changed || saving}
            onClick={() => {
              setSaving(true)
              api
                .setModuleConfig(module.id, draft)
                .then(
                  (next) => {
                    setModules(next)
                    refresh()
                    toast.success(`${module.name} settings saved`)
                  },
                  (e: unknown) => toast.error(errorMessage(e)),
                )
                .finally(() => {
                  setSaving(false)
                })
            }}
          >
            Save
          </Button>
          <Button
            variant="outline"
            disabled={!changed || saving}
            onClick={() => {
              setDraft(module.configValues)
            }}
          >
            Revert
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}
