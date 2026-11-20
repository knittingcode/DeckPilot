import { Monitor } from "lucide-react"

import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import type { SharedLayoutSummary } from "@/lib/types"

/** A quiet note: the open layout is shared, so edits apply to every profile using it. */
export function SharedLayoutNote({ layout }: { layout: SharedLayoutSummary | undefined }) {
  const { t } = useTranslation()
  if (!layout || layout.profiles.length < 2) return null
  return (
    <p className="text-xs text-muted-foreground" role="note">
      {t("editor:chooser.shared", {
        profiles: layout.profiles.map((p) => p.profileName).join(", "),
      })}
    </p>
  )
}

/** The device shows another layout than the one open: says which, and opens it. */
export function DeviceLayoutHint({ name, onOpen }: { name: string; onOpen: () => void }) {
  const { t } = useTranslation()
  return (
    <p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground" role="status">
      <Monitor className="size-3.5" aria-hidden />
      {t("editor:chooser.deviceShows", { layout: name })}
      <Button variant="link" size="xs" className="h-auto p-0 text-xs" onClick={onOpen}>
        {t("editor:chooser.open", { layout: name })}
      </Button>
    </p>
  )
}
