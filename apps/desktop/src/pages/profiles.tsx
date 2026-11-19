import { Copy, Ellipsis, Pencil, Plus, Trash2 } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState, ErrorNotice, Loading } from "@/components/page"
import {
  InlineStatus,
  Property,
  PropertyList,
  SettingsGroup,
  SettingsSection,
} from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Textarea } from "@/components/ui/textarea"
import { useEditingProfile } from "@/hooks/use-profile"
import { useTitlebar } from "@/hooks/use-titlebar"
import { api } from "@/lib/api"
import { failureMessage, toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { formatTimestamp } from "@/lib/format"
import type { ProfileSummary } from "@/lib/types"

type DialogState =
  | { mode: "create" }
  | { mode: "rename"; profile: ProfileSummary; description: string }
  | { mode: "duplicate"; profile: ProfileSummary }
  | null

/**
 * Settings › Profiles: every profile, the one active on the device, and
 * the one the editors work on (chosen in the main window's toolbar).
 */
export function ProfilesSettings() {
  const { profiles, refresh, editingProfileId, setEditingProfileId } = useAppContext()
  const [dialog, setDialog] = useState<DialogState>(null)
  const { state } = useEditingProfile()

  const run = (operation: () => Promise<unknown>, message: string, failed: string) => {
    operation().then(
      () => {
        toast.success(message)
        refresh()
      },
      (e: unknown) => {
        toastFailure(failed, e)
      },
    )
  }

  useTitlebar({
    actions: [
      {
        id: "profiles.new",
        label: "New profile",
        icon: Plus,
        placement: "primary",
        onSelect: () => {
          setDialog({ mode: "create" })
        },
      },
    ],
  })

  const openDetails = (profile: ProfileSummary) => {
    api
      .getProfile(profile.id)
      .then((view) => {
        setDialog({ mode: "rename", profile, description: view.profile.description })
      })
      .catch((e: unknown) => {
        toastFailure(`${profile.name} could not be opened for editing`, e)
      })
  }

  return (
    <>
      <SettingsSection
        title="All profiles"
        description="The active profile runs on the device. Any profile can be edited: select it here or in the toolbar."
      >
        {profiles.profiles.length === 0 ? (
          <EmptyState variant="plain" title="No profiles yet">
            Create your first profile with New profile in the toolbar.
          </EmptyState>
        ) : (
          <Table aria-label="Profiles" className="table-fixed">
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead>Name</TableHead>
                <TableHead className="hidden w-48 sm:table-cell">Last modified</TableHead>
                <TableHead className="w-24">Status</TableHead>
                <TableHead className="w-36">
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {profiles.profiles.map((profile) => {
                const active = profile.id === profiles.activeProfileId
                const editing = profile.id === editingProfileId
                return (
                  <TableRow key={profile.id} data-state={editing ? "selected" : undefined}>
                    <TableCell className="truncate">
                      <button
                        type="button"
                        aria-current={editing ? "true" : undefined}
                        title={editing ? "Being edited" : `Edit ${profile.name}`}
                        className="max-w-full truncate rounded-sm font-medium outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
                        onClick={() => {
                          setEditingProfileId(profile.id)
                        }}
                      >
                        {profile.name}
                      </button>
                    </TableCell>
                    <TableCell className="hidden text-caption text-muted-foreground tabular-nums sm:table-cell">
                      {formatTimestamp(profile.updatedAt)}
                    </TableCell>
                    <TableCell>
                      {active ? (
                        <InlineStatus tone="positive" title="Runs on the device">
                          Active
                        </InlineStatus>
                      ) : (
                        <span className="sr-only">Inactive</span>
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center justify-end gap-1">
                        {!active && (
                          <Button
                            variant="outline"
                            size="xs"
                            className="h-7 px-2.5 text-caption"
                            aria-label={`Make ${profile.name} active`}
                            onClick={() => {
                              run(
                                () => api.setActiveProfile(profile.id),
                                `${profile.name} is active`,
                                `${profile.name} was not activated`,
                              )
                            }}
                          >
                            Make active
                          </Button>
                        )}
                        <ProfileMenu
                          profile={profile}
                          active={active}
                          onRename={() => {
                            openDetails(profile)
                          }}
                          onDuplicate={() => {
                            setDialog({ mode: "duplicate", profile })
                          }}
                          onDelete={() => {
                            run(
                              () => api.deleteProfile(profile.id),
                              "Profile deleted",
                              `${profile.name} was not deleted`,
                            )
                          }}
                        />
                      </div>
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
        )}
      </SettingsSection>

      {state.status === "loading" && editingProfileId && <Loading />}
      {state.status === "error" && editingProfileId && <ErrorNotice error={state.error} />}
      {state.status === "ready" && (
        <SettingsSection
          title={state.data.profile.name}
          description="The profile being edited."
          actions={
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                const summary = profiles.profiles.find((p) => p.id === state.data.profile.id)
                if (summary) openDetails(summary)
              }}
            >
              <Pencil /> Edit details…
            </Button>
          }
        >
          <SettingsGroup className="py-3">
            <PropertyList aria-label={`Details of ${state.data.profile.name}`}>
              <Property label="Description">
                {state.data.profile.description || (
                  <span className="text-muted-foreground">No description</span>
                )}
              </Property>
              <Property label="Actions">{state.data.profile.actions.length}</Property>
              <Property label="Workflows">{state.data.profile.workflows.length}</Property>
              <Property label="Projects">{state.data.profile.projects.length}</Property>
              <Property label="Layouts">{state.data.profile.layouts.length}</Property>
            </PropertyList>
          </SettingsGroup>
          {state.data.issues.length > 0 && <ErrorNotice error={state.data.issues.join(" ")} />}
        </SettingsSection>
      )}

      <ProfileDialog
        state={dialog}
        onClose={() => {
          setDialog(null)
        }}
        onDone={(id) => {
          setDialog(null)
          if (id) setEditingProfileId(id)
          refresh()
        }}
      />
    </>
  )
}

/** Rename, duplicate and delete: the less frequent commands of a profile. */
function ProfileMenu({
  profile,
  active,
  onRename,
  onDuplicate,
  onDelete,
}: {
  profile: ProfileSummary
  active: boolean
  onRename: () => void
  onDuplicate: () => void
  onDelete: () => void
}) {
  const [confirming, setConfirming] = useState(false)
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7"
            aria-label={`More for ${profile.name}`}
          >
            <Ellipsis />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={onRename}>
            <Pencil /> Rename…
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={onDuplicate}>
            <Copy /> Duplicate…
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            variant="destructive"
            onSelect={() => {
              setConfirming(true)
            }}
          >
            <Trash2 /> Delete…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={`Delete “${profile.name}”?`}
        description={
          active
            ? "This profile is active on the device. Its actions, workflows, projects and layouts are deleted and the device shows no layout until another profile is activated."
            : "Its actions, workflows, projects and layouts are deleted. This cannot be undone."
        }
        onConfirm={onDelete}
      />
    </>
  )
}

function ProfileDialog({
  state,
  onClose,
  onDone,
}: {
  state: DialogState
  onClose: () => void
  onDone: (id?: string) => void
}) {
  return (
    <Dialog
      open={state !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent>
        {state && (
          <ProfileForm
            key={state.mode === "create" ? "create" : `${state.mode}:${state.profile.id}`}
            state={state}
            onDone={onDone}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function ProfileForm({
  state,
  onDone,
}: {
  state: NonNullable<DialogState>
  onDone: (id?: string) => void
}) {
  const [name, setName] = useState(
    state.mode === "create"
      ? ""
      : state.mode === "duplicate"
        ? `${state.profile.name} (copy)`
        : state.profile.name,
  )
  const [description, setDescription] = useState(state.mode === "rename" ? state.description : "")
  const [error, setError] = useState<string | null>(null)
  const titles = { create: "New profile", rename: "Edit profile", duplicate: "Duplicate profile" }

  const submit = async (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    try {
      if (state.mode === "create") {
        onDone((await api.createProfile(name, description)).id)
      } else if (state.mode === "rename") {
        await api.updateProfileDetails(state.profile.id, name, description)
        onDone()
      } else {
        onDone((await api.duplicateProfile(state.profile.id, name)).id)
      }
      toast.success("Saved")
    } catch (e) {
      const failed = {
        create: "The profile was not created",
        rename: "The profile was not saved",
        duplicate: "The profile was not duplicated",
      }
      setError(failureMessage(failed[state.mode], e))
    }
  }

  return (
    <form className="grid gap-4" onSubmit={(e) => void submit(e)}>
      <DialogHeader>
        <DialogTitle>{titles[state.mode]}</DialogTitle>
        {state.mode === "duplicate" && (
          <DialogDescription>
            The copy gets its own actions, workflows, projects and layouts. Changing one profile
            never changes the other.
          </DialogDescription>
        )}
      </DialogHeader>
      {error && <ErrorNotice error={error} />}
      <div className="grid gap-2">
        <Label htmlFor="profile-name">Name</Label>
        <Input
          id="profile-name"
          value={name}
          autoFocus
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
      {state.mode !== "duplicate" && (
        <div className="grid gap-2">
          <Label htmlFor="profile-description">Description</Label>
          <Textarea
            id="profile-description"
            value={description}
            onChange={(e) => {
              setDescription(e.target.value)
            }}
          />
        </div>
      )}
      <DialogFooter>
        <Button type="submit" disabled={!name.trim()}>
          Save
        </Button>
      </DialogFooter>
    </form>
  )
}
