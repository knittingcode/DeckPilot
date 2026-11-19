import { Check, ChevronsUpDown, Monitor } from "lucide-react"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import type { DeviceLayout, SharedLayoutSummary } from "@/lib/types"
import { cn } from "@/lib/utils"

function profilesLabel(count: number): string {
  return `shared · ${count} profiles`
}

/**
 * The Device Editor's choice of layout: the profile's layouts for the
 * model (its default marked, and those other profiles use too), then the
 * model's layouts this profile does not use yet, to use them here.
 */
export function LayoutChooser({
  profileId,
  modelName,
  layouts,
  current,
  defaultLayout,
  shared,
  onSelect,
  onUse,
}: {
  profileId: string
  modelName: string
  /** The profile's layouts for the model. */
  layouts: DeviceLayout[]
  current: DeviceLayout | null
  defaultLayout: string | null
  /** Every shared layout of the model; `null` while unknown. */
  shared: SharedLayoutSummary[] | null
  onSelect: (id: string) => void
  /** Assigns a layout of the model to the profile and opens it. */
  onUse: (layout: SharedLayoutSummary) => void
}) {
  const summary = (id: string) => shared?.find((l) => l.id === id)
  const own = new Set(layouts.map((l) => l.id))
  const others = (shared ?? [])
    .filter((l) => !own.has(l.id))
    .sort((a, b) => a.name.localeCompare(b.name))
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          aria-label={`Layout: ${current?.name ?? "none"}`}
          className="w-56 justify-between font-normal"
        >
          <span className="truncate">{current?.name ?? "No layouts"}</span>
          <ChevronsUpDown className="opacity-50" aria-hidden />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-72">
        <DropdownMenuGroup aria-label="Layouts of this profile">
          <DropdownMenuLabel className="text-xs text-muted-foreground">
            {layouts.length === 0 ? `No layouts for ${modelName} yet` : "In this profile"}
          </DropdownMenuLabel>
          {layouts.map((layout) => {
            const users = summary(layout.id)?.profiles.length ?? 1
            const marks = [
              layout.id === defaultLayout && "default",
              users > 1 && profilesLabel(users),
            ].filter(Boolean)
            return (
              <DropdownMenuItem
                key={layout.id}
                aria-current={layout.id === current?.id ? "true" : undefined}
                onSelect={() => {
                  onSelect(layout.id)
                }}
              >
                <Check
                  className={cn(layout.id === current?.id ? "opacity-100" : "opacity-0")}
                  aria-hidden
                />
                <span className="min-w-0 flex-1 truncate">{layout.name}</span>
                {marks.length > 0 && (
                  <span className="text-xs text-muted-foreground">{marks.join(" · ")}</span>
                )}
              </DropdownMenuItem>
            )
          })}
        </DropdownMenuGroup>
        {others.length > 0 && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuGroup aria-label={`Other layouts for ${modelName}`}>
              <DropdownMenuLabel className="text-xs text-muted-foreground">
                Other layouts for {modelName}
              </DropdownMenuLabel>
              {others.map((layout) => (
                <DropdownMenuItem
                  key={layout.id}
                  aria-label={`Use ${layout.name} in this profile`}
                  onSelect={() => {
                    onUse(layout)
                  }}
                >
                  <span className="grid min-w-0 flex-1">
                    <span className="truncate">{layout.name}</span>
                    <span className="truncate text-xs text-muted-foreground">
                      {layout.profiles.length === 0
                        ? "Not used by any profile"
                        : `Used by ${layout.profiles
                            .filter((p) => p.profileId !== profileId)
                            .map((p) => p.profileName)
                            .join(", ")}`}
                    </span>
                  </span>
                  <span className="text-xs">Use in this profile</span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** A quiet note: the open layout is shared, so edits apply to every profile using it. */
export function SharedLayoutNote({ layout }: { layout: SharedLayoutSummary | undefined }) {
  if (!layout || layout.profiles.length < 2) return null
  return (
    <p className="text-xs text-muted-foreground" role="note">
      Shared: edits apply to every profile using this layout (
      {layout.profiles.map((p) => p.profileName).join(", ")}).
    </p>
  )
}

/** The device shows another layout than the one open: says which, and opens it. */
export function DeviceLayoutHint({ name, onOpen }: { name: string; onOpen: () => void }) {
  return (
    <p className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground" role="status">
      <Monitor className="size-3.5" aria-hidden />
      The device shows {name}.
      <Button variant="link" size="xs" className="h-auto p-0 text-xs" onClick={onOpen}>
        Open {name}
      </Button>
    </p>
  )
}
