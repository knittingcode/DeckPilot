import type { ReactNode } from "react"

import { EmptyState } from "@/components/page"
import { useAppContext } from "@/lib/app-context"

/** Renders `children` with the active profile id, or an explanation when there is none. */
export function RequireProfile({ children }: { children: (profileId: string) => ReactNode }) {
  const { profiles } = useAppContext()
  if (!profiles.activeProfileId) {
    return (
      <EmptyState title="No active profile">
        Create a profile on the Profiles page, or mark an existing one as active.
      </EmptyState>
    )
  }
  return children(profiles.activeProfileId)
}
