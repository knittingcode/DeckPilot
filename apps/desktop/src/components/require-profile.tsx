import type { ReactNode } from "react"

import { EmptyState } from "@/components/page"
import { Button } from "@/components/ui/button"
import { useAppContext } from "@/lib/app-context"

/** Renders `children` with the profile being edited, or a prompt to create one. */
export function RequireProfile({ children }: { children: (profileId: string) => ReactNode }) {
  const { editingProfileId, navigate } = useAppContext()
  if (!editingProfileId) {
    return (
      <EmptyState title="No profile yet">
        <p>Profiles hold your actions, workflows, projects and layouts.</p>
        <Button
          className="mt-3"
          size="sm"
          onClick={() => {
            navigate("profiles")
          }}
        >
          Create a profile
        </Button>
      </EmptyState>
    )
  }
  return children(editingProfileId)
}
