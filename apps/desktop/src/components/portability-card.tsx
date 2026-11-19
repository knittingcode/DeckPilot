import { Download, Upload } from "lucide-react"

import { SettingsGroup, SettingsRow } from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"

/**
 * Encrypted export and import of the portable configuration. Both run in
 * native windows of their own (`portability.export`, `portability.import`).
 */
export function PortabilityCard() {
  const open = (id: string) => {
    api.openWindow(id).catch((e: unknown) => {
      toastFailure("The window did not open", e)
    })
  }
  return (
    <div className="grid gap-3">
      <p className="text-sm text-muted-foreground">
        Move profiles, projects, actions, workflows, layouts, module and application settings to
        another computer in an encrypted file — and keep one as your backup. Paths and other
        settings of this computer, keys and passwords are not included.
      </p>
      <SettingsGroup>
        <SettingsRow
          label="Export"
          description="Saves the configuration to an encrypted file protected by a password you choose."
        >
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              open("portability.export")
            }}
          >
            <Download /> Export…
          </Button>
        </SettingsRow>
        <SettingsRow
          label="Import"
          description="Reads an encrypted export; you review what changes before anything is applied."
        >
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              open("portability.import")
            }}
          >
            <Upload /> Import…
          </Button>
        </SettingsRow>
      </SettingsGroup>
    </div>
  )
}
