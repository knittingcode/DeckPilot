import { Copy, Pencil, Plus, Trash2 } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
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

export function ProfilesPage() {
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

  return (
    <Page
      title="Profiles"
      description="Independent sets of actions, workflows, projects and layouts"
      actions={[
        {
          id: "profiles.new",
          label: "New profile",
          icon: Plus,
          placement: "primary",
          onSelect: () => {
            setDialog({ mode: "create" })
          },
        },
      ]}
    >
      <Card>
        <CardHeader>
          <CardTitle>All profiles</CardTitle>
          <CardDescription>
            The active profile runs on the device. Any profile can be edited; pick it in the
            sidebar.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {profiles.profiles.length === 0 ? (
            <EmptyState title="No profiles yet">Create your first profile.</EmptyState>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead className="hidden md:table-cell">Last modified</TableHead>
                  <TableHead className="w-0" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {profiles.profiles.map((profile) => {
                  const active = profile.id === profiles.activeProfileId
                  return (
                    <TableRow
                      key={profile.id}
                      data-state={profile.id === editingProfileId ? "selected" : undefined}
                    >
                      <TableCell className="font-medium">
                        <button
                          type="button"
                          className="hover:underline"
                          onClick={() => {
                            setEditingProfileId(profile.id)
                          }}
                        >
                          {profile.name}
                        </button>
                        {active && <Badge className="ml-2">On device</Badge>}
                      </TableCell>
                      <TableCell className="hidden text-muted-foreground md:table-cell">
                        {formatTimestamp(profile.updatedAt)}
                      </TableCell>
                      <TableCell>
                        <div className="flex justify-end gap-1">
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={active}
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
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label={`Rename ${profile.name}`}
                            onClick={() => {
                              api
                                .getProfile(profile.id)
                                .then((view) => {
                                  setDialog({
                                    mode: "rename",
                                    profile,
                                    description: view.profile.description,
                                  })
                                })
                                .catch((e: unknown) => {
                                  toastFailure(`${profile.name} could not be opened for editing`, e)
                                })
                            }}
                          >
                            <Pencil />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            aria-label={`Duplicate ${profile.name}`}
                            onClick={() => {
                              setDialog({ mode: "duplicate", profile })
                            }}
                          >
                            <Copy />
                          </Button>
                          <ConfirmDialog
                            trigger={
                              <Button
                                variant="ghost"
                                size="icon"
                                aria-label={`Delete ${profile.name}`}
                              >
                                <Trash2 />
                              </Button>
                            }
                            title={`Delete “${profile.name}”?`}
                            description={
                              active
                                ? "This profile is active on the device. Its actions, workflows, projects and layouts are deleted and the device shows no layout until another profile is activated."
                                : "Its actions, workflows, projects and layouts are deleted. This cannot be undone."
                            }
                            onConfirm={() => {
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
        </CardContent>
      </Card>

      {state.status === "loading" && editingProfileId && <Loading />}
      {state.status === "error" && editingProfileId && <ErrorNotice error={state.error} />}
      {state.status === "ready" && (
        <Card>
          <CardHeader>
            <CardTitle>{state.data.profile.name}</CardTitle>
            <CardDescription>{state.data.profile.description || "No description"}</CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid grid-cols-2 gap-2 text-sm sm:grid-cols-4">
              <span>{state.data.profile.actions.length} actions</span>
              <span>{state.data.profile.workflows.length} workflows</span>
              <span>{state.data.profile.projects.length} projects</span>
              <span>{state.data.profile.layouts.length} layouts</span>
            </div>
            {state.data.issues.length > 0 && <ErrorNotice error={state.data.issues.join(" ")} />}
          </CardContent>
        </Card>
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
    </Page>
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
