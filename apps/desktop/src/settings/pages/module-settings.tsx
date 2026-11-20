import { useState } from "react"
import { toast } from "sonner"

import { ParameterField } from "@/components/parameter-field"
import { VariableScopeProvider } from "@/components/variable-picker/scope-provider"
import { Button } from "@/components/ui/button"
import { SettingsSection } from "@/components/settings-layout"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { initialValue } from "@/lib/parameters"
import type { ActionParameters, ModuleInfo } from "@/lib/types"
import { useTranslation } from "@/i18n"

/** Settings page generated from the settings a module declares in its manifest. */
export function ModuleSettingsForm({ module }: { module: ModuleInfo }) {
  const { t } = useTranslation()
  const { setModules, refresh } = useAppContext()
  const [draft, setDraft] = useState<ActionParameters>(module.configValues)
  const [saving, setSaving] = useState(false)
  const changed = JSON.stringify(draft) !== JSON.stringify(module.configValues)
  return (
    <SettingsSection title={t("settings:title")} description={module.description}>
      <div className="grid max-w-xl gap-4 border-t pt-4">
        {/* Module settings are read as saved; no resolver turns references
            in them into values, so the picker is off here. */}
        <VariableScopeProvider disabled>
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
        </VariableScopeProvider>
      </div>
      <div className="flex justify-end gap-2 border-t pt-3">
        <Button
          variant="ghost"
          size="sm"
          disabled={!changed || saving}
          onClick={() => {
            setDraft(module.configValues)
          }}
        >
          {t("settings:shared.revert")}
        </Button>
        <Button
          size="sm"
          disabled={!changed || saving}
          onClick={() => {
            setSaving(true)
            api
              .setModuleConfig(module.id, draft)
              .then(
                (next) => {
                  setModules(next)
                  refresh()
                  toast.success(t("settings:moduleSettings.saved", { module: module.name }))
                },
                (e: unknown) => {
                  toastFailure(t("settings:moduleSettings.notSaved", { module: module.name }), e)
                },
              )
              .finally(() => {
                setSaving(false)
              })
          }}
        >
          {t("settings:shared.save")}
        </Button>
      </div>
    </SettingsSection>
  )
}
