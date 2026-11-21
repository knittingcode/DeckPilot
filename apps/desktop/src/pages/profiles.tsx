import { Copy, Ellipsis, Pencil, Plus, Trash2 } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { DestructiveDialog, FormDialog } from "@/components/dialogs"
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
import { useTranslation } from "@/i18n"
import { useTitlebar } from "@/hooks/use-titlebar"
import { api } from "@/lib/api"
import { failureMessage, toastFailure } from "@/lib/failure"
import { useAppContext } from "@/lib/app-context"
import { PROJECTS_PLUGIN, useIsPluginActive } from "@/lib/plugin-features"
import { formatShortTimestamp, formatTimestamp } from "@/lib/format"
import type { ProfileSummary } from "@/lib/types"
import { useOpenLayout } from "@/shell/open-layout"
import { ProfileLayouts } from "@/shell/profile-layouts"

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
  const { t } = useTranslation()
  const { profiles, refresh, editingProfileId, setEditingProfileId } = useAppContext()
  const [dialog, setDialog] = useState<DialogState>(null)
  const { state } = useEditingProfile()
  const { open: openLayout, dialog: openDialog } = useOpenLayout()
  const projectsActive = useIsPluginActive(PROJECTS_PLUGIN)

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
        label: t("workspace:profiles.new"),
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
        toastFailure(t("workspace:profiles.toast.openFailed", { profile: profile.name }), e)
      })
  }

  return (
    <>
      <SettingsSection
        title={t("workspace:profiles.all.title")}
        description={t("workspace:profiles.all.description")}
      >
        {profiles.profiles.length === 0 ? (
          <EmptyState variant="plain" title={t("workspace:profiles.empty.title")}>
            {t("workspace:profiles.empty.body")}
          </EmptyState>
        ) : (
          <Table aria-label={t("workspace:profiles.table.label")} className="table-fixed">
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead>{t("workspace:profiles.table.name")}</TableHead>
                <TableHead className="hidden w-44 sm:table-cell">
                  {t("workspace:profiles.table.modified")}
                </TableHead>
                <TableHead className="w-20">{t("workspace:profiles.table.status")}</TableHead>
                <TableHead className="w-32">
                  <span className="sr-only">{t("workspace:profiles.table.actions")}</span>
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
                        title={
                          editing
                            ? t("workspace:profiles.beingEdited")
                            : t("workspace:profiles.editProfile", { profile: profile.name })
                        }
                        className="max-w-full truncate rounded-sm font-medium outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
                        onClick={() => {
                          setEditingProfileId(profile.id)
                        }}
                      >
                        {profile.name}
                      </button>
                    </TableCell>
                    <TableCell className="hidden text-caption text-muted-foreground tabular-nums sm:table-cell">
                      <time
                        dateTime={new Date(profile.updatedAt * 1000).toISOString()}
                        title={formatTimestamp(profile.updatedAt)}
                      >
                        {formatShortTimestamp(profile.updatedAt)}
                      </time>
                    </TableCell>
                    <TableCell>
                      {active ? (
                        <InlineStatus tone="positive" title={t("workspace:profiles.runsOnDevice")}>
                          {t("workspace:profiles.active")}
                        </InlineStatus>
                      ) : (
                        <span className="sr-only">{t("workspace:profiles.inactive")}</span>
                      )}
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center justify-end gap-1">
                        {!active && (
                          <Button
                            variant="outline"
                            size="xs"
                            className="h-7 px-2.5 text-caption"
                            aria-label={t("workspace:profiles.makeActiveLabel", {
                              profile: profile.name,
                            })}
                            onClick={() => {
                              run(
                                () => api.setActiveProfile(profile.id),
                                t("workspace:profiles.toast.activated", { profile: profile.name }),
                                t("workspace:profiles.toast.activateFailed", {
                                  profile: profile.name,
                                }),
                              )
                            }}
                          >
                            {t("workspace:profiles.makeActive")}
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
                          onDelete={async () => {
                            await api.deleteProfile(profile.id)
                            toast.success(t("workspace:profiles.toast.deleted"))
                            refresh()
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
          description={t("workspace:profiles.editing.description")}
          actions={
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                const summary = profiles.profiles.find((p) => p.id === state.data.profile.id)
                if (summary) openDetails(summary)
              }}
            >
              <Pencil /> {t("workspace:profiles.editing.editDetails")}
            </Button>
          }
        >
          <SettingsGroup className="py-3">
            <PropertyList
              aria-label={t("workspace:profiles.editing.details", {
                profile: state.data.profile.name,
              })}
            >
              <Property label={t("workspace:profiles.editing.description_label")}>
                {state.data.profile.description || (
                  <span className="text-muted-foreground">
                    {t("workspace:profiles.editing.noDescription")}
                  </span>
                )}
              </Property>
              <Property label={t("workspace:profiles.editing.actions")}>
                {state.data.profile.actions.length}
              </Property>
              <Property label={t("workspace:profiles.editing.workflows")}>
                {state.data.profile.workflows.length}
              </Property>
              {projectsActive && (
                <Property label={t("workspace:profiles.editing.projects")}>
                  {state.data.profile.projects.length}
                </Property>
              )}
            </PropertyList>
          </SettingsGroup>
          {state.data.issues.length > 0 && <ErrorNotice error={state.data.issues.join(" ")} />}
        </SettingsSection>
      )}
      {state.status === "ready" && (
        <ProfileLayouts profileId={state.data.profile.id} onOpenLayout={openLayout} />
      )}
      {openDialog}

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
  /** Rejects with the backend's error; the confirmation then shows it. */
  onDelete: () => Promise<void>
}) {
  const { t } = useTranslation()
  const [confirming, setConfirming] = useState(false)
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-7"
            aria-label={t("workspace:profiles.menu.more", { profile: profile.name })}
          >
            <Ellipsis />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={onRename}>
            <Pencil /> {t("workspace:profiles.menu.rename")}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={onDuplicate}>
            <Copy /> {t("workspace:profiles.menu.duplicate")}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            variant="destructive"
            onSelect={() => {
              setConfirming(true)
            }}
          >
            <Trash2 /> {t("workspace:profiles.menu.delete")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <DestructiveDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("workspace:profiles.delete.title", { profile: profile.name })}
        description={
          active ? t("workspace:profiles.delete.active") : t("workspace:profiles.delete.inactive")
        }
        onConfirm={onDelete}
        describeError={(e) =>
          failureMessage(t("workspace:profiles.delete.failed", { profile: profile.name }), e)
        }
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
  if (!state) return null
  return (
    <ProfileForm
      key={state.mode === "create" ? "create" : `${state.mode}:${state.profile.id}`}
      state={state}
      onClose={onClose}
      onDone={onDone}
    />
  )
}

function ProfileForm({
  state,
  onClose,
  onDone,
}: {
  state: NonNullable<DialogState>
  onClose: () => void
  onDone: (id?: string) => void
}) {
  const { t } = useTranslation()
  const initialName =
    state.mode === "create"
      ? ""
      : state.mode === "duplicate"
        ? t("workspace:profiles.form.copyName", { profile: state.profile.name })
        : state.profile.name
  const initialDescription = state.mode === "rename" ? state.description : ""
  const [name, setName] = useState(initialName)
  const [description, setDescription] = useState(initialDescription)
  const titles = {
    create: t("workspace:profiles.form.createTitle"),
    rename: t("workspace:profiles.form.renameTitle"),
    duplicate: t("workspace:profiles.form.duplicateTitle"),
  }
  const failed = {
    create: t("workspace:profiles.form.createFailed"),
    rename: t("workspace:profiles.form.renameFailed"),
    duplicate: t("workspace:profiles.form.duplicateFailed"),
  }

  const submit = async () => {
    if (state.mode === "create") {
      onDone((await api.createProfile(name, description)).id)
    } else if (state.mode === "rename") {
      await api.updateProfileDetails(state.profile.id, name, description)
      onDone()
    } else {
      onDone((await api.duplicateProfile(state.profile.id, name)).id)
    }
    toast.success(t("workspace:profiles.form.saved"))
  }

  return (
    <FormDialog
      open
      title={titles[state.mode]}
      description={
        state.mode === "duplicate" ? t("workspace:profiles.form.duplicateDescription") : undefined
      }
      submitLabel={t("workspace:profiles.form.save")}
      canSubmit={Boolean(name.trim())}
      dirty={name !== initialName || description !== initialDescription}
      onClose={onClose}
      onSubmit={submit}
      describeError={(e) => failureMessage(failed[state.mode], e)}
    >
      <div className="grid gap-2">
        <Label htmlFor="profile-name">{t("workspace:profiles.form.name")}</Label>
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
          <Label htmlFor="profile-description">{t("workspace:profiles.form.description")}</Label>
          <Textarea
            id="profile-description"
            value={description}
            onChange={(e) => {
              setDescription(e.target.value)
            }}
          />
        </div>
      )}
    </FormDialog>
  )
}
