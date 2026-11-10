import type { ReactNode } from "react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { Separator } from "@/components/ui/separator"
import { SidebarTrigger } from "@/components/ui/sidebar"
import { useAppContext } from "@/lib/app-context"

/** The profile a profile-scoped page edits, shown in its header. */
function ProfileScope() {
  const { profiles, editingProfileId } = useAppContext()
  const profile = profiles.profiles.find((p) => p.id === editingProfileId)
  if (!profile) return null
  const onDevice = profile.id === profiles.activeProfileId
  return (
    <span
      className="flex max-w-48 shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs"
      title={onDevice ? "Active on the device" : "Not active on the device"}
    >
      <span
        className={
          onDevice
            ? "size-1.5 rounded-full bg-emerald-500"
            : "size-1.5 rounded-full bg-muted-foreground/50"
        }
        aria-hidden
      />
      <span className="text-muted-foreground">Profile</span>
      <span className="truncate font-medium">{profile.name}</span>
    </span>
  )
}

export function Page({
  title,
  description,
  actions,
  profileScoped = false,
  children,
}: {
  title: string
  description?: string
  actions?: ReactNode
  /** The page edits the profile chosen in the sidebar. */
  profileScoped?: boolean
  children: ReactNode
}) {
  return (
    <div className="flex min-h-full flex-col">
      <header className="flex h-14 shrink-0 items-center gap-2 border-b px-4">
        <SidebarTrigger className="-ml-1" />
        <Separator orientation="vertical" className="mr-2 data-[orientation=vertical]:h-4" />
        <div className="min-w-0 flex-1">
          <h1 className="truncate text-sm font-semibold">{title}</h1>
          {description && <p className="truncate text-xs text-muted-foreground">{description}</p>}
        </div>
        {profileScoped && <ProfileScope />}
        {actions}
      </header>
      <div className="flex flex-1 flex-col gap-6 p-6">{children}</div>
    </div>
  )
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
