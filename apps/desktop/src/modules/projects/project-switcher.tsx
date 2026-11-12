import { ChevronLeft, ChevronRight } from "lucide-react"

import { Button } from "@/components/ui/button"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import type { Project } from "@/lib/types"

/**
 * Cycles or selects the active project of the active profile. Changing the
 * project only changes context; nothing is launched or restarted.
 */
export function ProjectSwitcher({
  profileId,
  projects,
  activeProjectId,
}: {
  profileId: string
  projects: Project[]
  activeProjectId: string | null
}) {
  const { profiles, refresh } = useAppContext()
  const isActiveProfile = profiles.activeProfileId === profileId
  const run = (operation: () => Promise<unknown>) => {
    operation().then(refresh, (error: unknown) => {
      toastFailure("The project was not switched", error)
    })
  }
  if (projects.length === 0) {
    return <p className="text-sm text-muted-foreground">This profile has no projects.</p>
  }
  if (!isActiveProfile) {
    return (
      <p className="text-sm text-muted-foreground">
        Projects can be switched for the profile that is active on the device.
      </p>
    )
  }
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button
        variant="outline"
        size="icon"
        aria-label="Previous project"
        onClick={() => {
          run(() => api.cycleProject(profileId, "previous"))
        }}
      >
        <ChevronLeft />
      </Button>
      <ToggleGroup
        type="single"
        variant="outline"
        value={activeProjectId ?? ""}
        onValueChange={(id) => {
          if (id) run(() => api.selectProject(profileId, id))
        }}
        aria-label="Active project"
      >
        {projects.map((project) => (
          <ToggleGroupItem
            key={project.id}
            value={project.id}
            className="px-3 font-mono text-xs"
            title={project.name}
          >
            {project.key}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
      <Button
        variant="outline"
        size="icon"
        aria-label="Next project"
        onClick={() => {
          run(() => api.cycleProject(profileId, "next"))
        }}
      >
        <ChevronRight />
      </Button>
    </div>
  )
}
