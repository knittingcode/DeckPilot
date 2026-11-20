import { Download, Upload } from "lucide-react"

import { SettingsGroup, SettingsRow } from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useTranslation } from "@/i18n"

/**
 * Encrypted export and import of the portable configuration. Both run in
 * native windows of their own (`portability.export`, `portability.import`).
 */
export function PortabilityCard() {
  const { t } = useTranslation()
  const open = (id: string) => {
    api.openWindow(id).catch((e: unknown) => {
      toastFailure(t("settings:backup.windowFailed"), e)
    })
  }
  return (
    <div className="grid gap-3">
      <p className="text-sm text-muted-foreground">{t("settings:backup.intro")}</p>
      <SettingsGroup>
        <SettingsRow
          label={t("settings:backup.export")}
          description={t("settings:backup.exportDescription")}
        >
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              open("portability.export")
            }}
          >
            <Download /> {t("settings:backup.exportEllipsis")}
          </Button>
        </SettingsRow>
        <SettingsRow
          label={t("settings:backup.import")}
          description={t("settings:backup.importDescription")}
        >
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              open("portability.import")
            }}
          >
            <Upload /> {t("settings:backup.importEllipsis")}
          </Button>
        </SettingsRow>
      </SettingsGroup>
    </div>
  )
}
