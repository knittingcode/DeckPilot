import type { ReactNode } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { useTitlebar } from "@/hooks/use-titlebar"
import { useAppContext } from "@/lib/app-context"
import type { TitlebarAction, TitlebarBreadcrumb } from "@/lib/titlebar"

/** The profile a profile-scoped page edits, shown in the titlebar. */
function useProfileScope(enabled: boolean) {
  const { profiles, editingProfileId } = useAppContext()
  const profile = enabled ? profiles.profiles.find((p) => p.id === editingProfileId) : undefined
  if (!profile) return undefined
  const onDevice = profile.id === profiles.activeProfileId
  return {
    caption: "Profile",
    label: profile.name,
    tone: onDevice ? ("positive" as const) : ("neutral" as const),
    hint: onDevice ? "Active on the device" : "Not active on the device",
  }
}

/**
 * A page of the main window. Its title, description and actions appear in
 * the shared titlebar; the page renders only its content.
 */
export function Page({
  title,
  description,
  breadcrumbs,
  actions,
  profileScoped = false,
  children,
}: {
  title: string
  description?: string
  breadcrumbs?: TitlebarBreadcrumb[]
  /** Commands of the page in the titlebar. Components inside may add more (`useTitlebar`). */
  actions?: TitlebarAction[]
  /** The page edits the profile chosen in the sidebar. */
  profileScoped?: boolean
  children: ReactNode
}) {
  const status = useProfileScope(profileScoped)
  useTitlebar({ title, subtitle: description, breadcrumbs, actions, status })
  return <div className="flex flex-1 flex-col gap-6 p-6">{children}</div>
}

export function ErrorNotice({ error }: { error: string }) {
  return (
    <Alert variant="destructive">
      <AlertTitle>Something went wrong</AlertTitle>
      <AlertDescription>{error}</AlertDescription>
    </Alert>
  )
}

export function Loading() {
  return <p className="text-sm text-muted-foreground">Loading…</p>
}

export function EmptyState({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed p-8 text-center">
      <p className="text-sm font-medium">{title}</p>
      {children && <div className="text-sm text-muted-foreground">{children}</div>}
    </div>
  )
}
