import type { ReactNode } from "react"

import { EmptyState } from "@/components/page"
import { Button } from "@/components/ui/button"
import { useTranslation } from "@/i18n"
import { useAppContext } from "@/lib/app-context"

/** Renders `children` with the profile being edited, or a prompt to create one. */
export function RequireProfile({ children }: { children: (profileId: string) => ReactNode }) {
  const { t } = useTranslation()
  const { editingProfileId, navigate } = useAppContext()
  if (!editingProfileId) {
    return (
      <EmptyState title={t("actions:requireProfile.title")}>
        <p>{t("actions:requireProfile.body")}</p>
        <Button
          className="mt-3"
          size="sm"
          onClick={() => {
            navigate("profiles")
          }}
        >
          {t("actions:requireProfile.create")}
        </Button>
      </EmptyState>
    )
  }
  return children(editingProfileId)
}
