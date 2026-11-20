import { useTranslation } from "@/i18n"

/**
 * A binding kept although its action cannot run from its control
 * (docs/device-controls.md): it stays in the profile, the device does not
 * run it, and it works again once a compatible action or device is there.
 */
export function IncompatibleNote({ reason }: { reason: string | undefined }) {
  const { t } = useTranslation()
  if (!reason) return null
  return (
    <p className="text-xs text-destructive" role="note" data-slot="incompatible-note">
      {t("settings:incompatible.note", { reason })}
    </p>
  )
}
