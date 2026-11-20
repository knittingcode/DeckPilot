import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core"
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable"
import { CSS } from "@dnd-kit/utilities"
import { ArrowLeftRight, Copy, GripVertical, Pencil, Plus, Trash2, X } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog, FormDialog } from "@/components/dialogs"
import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Button } from "@/components/ui/button"
import {
  InlineStatus,
  SettingsGroup,
  SettingsRow,
  SettingsSection,
} from "@/components/settings-layout"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useAsync } from "@/hooks/use-async"
import { useEditingProfile } from "@/hooks/use-profile"
import { Trans, useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { failureMessage, toastFailure } from "@/lib/failure"
import { useTitlebar } from "@/hooks/use-titlebar"
import { useAppContext } from "@/lib/app-context"
import type { Project, ProfileView, ResourceCheck, ResourceStatus } from "@/lib/types"

import { StatusIcon } from "@/components/resource-status-icon"
import { STATUS } from "@/lib/resource-status"

import { ProjectSwitcher } from "./project-switcher"

export function ProjectsPage() {
  const { t } = useTranslation()
  return (
    <Page
      title={t("workspace:projects.page.title")}
      description={t("workspace:projects.page.description")}
      profileScoped
    >
      <RequireProfile>{() => <Projects />}</RequireProfile>
    </Page>
  )
}

function Projects() {
  const { t } = useTranslation()
  const { state, reload, profileId } = useEditingProfile()
  const { revision, openSettings } = useAppContext()
  const checks = useAsync(
    () => (profileId ? api.checkResources(profileId) : Promise.resolve([])),
    `${profileId ?? ""}:${revision}`,
  )
  if (state.status === "loading") return <Loading />
  if (state.status === "error") return <ErrorNotice error={state.error} />
  if (!profileId) return null
  const resourceChecks = checks.state.status === "ready" ? checks.state.data : []
  const reloadAll = () => {
    reload()
    checks.reload()
  }
  return (
    <div className="mx-auto grid w-full max-w-3xl content-start gap-8">
      <ProjectCycle
        view={state.data}
        profileId={profileId}
        checks={resourceChecks}
        onChange={reloadAll}
      />
      {/* This computer's bindings are the core's (Settings › Resources). */}
      <p className="text-sm text-muted-foreground">
        {t("workspace:projects.machine.moved")}{" "}
        <Button
          variant="link"
          size="sm"
          className="h-auto p-0"
          onClick={() => {
            openSettings("resources")
          }}
        >
          {t("workspace:projects.machine.open")}
        </Button>
      </p>
    </div>
  )
}

type ProjectDialogState =
  | { mode: "create" }
  | { mode: "edit"; project: Project }
  | { mode: "copy"; project: Project }
  | null

function ProjectCycle({
  view,
  profileId,
  checks,
  onChange,
}: {
  view: ProfileView
  profileId: string
  checks: ResourceCheck[]
  onChange: () => void
}) {
  const { t } = useTranslation()
  const { refresh, profiles } = useAppContext()
  const [dialog, setDialog] = useState<ProjectDialogState>(null)
  const otherProfiles = profiles.profiles.filter((p) => p.id !== profileId)
  const projects = view.profile.projects
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )
  const changed = () => {
    onChange()
    refresh()
  }
  const onDragEnd = (event: DragEndEvent) => {
    const to = projects.findIndex((p) => p.id === event.over?.id)
    if (to < 0 || event.active.id === event.over?.id) return
    api.moveProject(profileId, String(event.active.id), to).then(changed, (e: unknown) => {
      toastFailure(t("workspace:projects.cycle.moveFailed"), e)
    })
  }
  const statusOf = (key: string) => checks.find((c) => c.key === key)?.status ?? "unbound"
  const isActiveProfile = profiles.activeProfileId === profileId
  useTitlebar({
    actions: [
      {
        id: "projects.new",
        label: t("workspace:projects.actions.new"),
        icon: Plus,
        placement: "primary",
        onSelect: () => {
          setDialog({ mode: "create" })
        },
      },
      // Quick access from anywhere on the page; the switcher card also
      // offers previous/next and explains the context.
      ...(isActiveProfile && projects.length > 1
        ? [
            {
              id: "projects.switch",
              label: t("workspace:projects.actions.switch"),
              icon: ArrowLeftRight,
              items: projects.map((project) => ({
                id: project.id,
                label: t("workspace:projects.option", { key: project.key, project: project.name }),
                checked: project.id === view.activeProjectId,
                onSelect: () => {
                  api.selectProject(profileId, project.id).then(changed, (e: unknown) => {
                    toastFailure(
                      t("workspace:projects.actions.selectFailed", { key: project.key }),
                      e,
                    )
                  })
                },
              })),
            },
          ]
        : []),
    ],
  })

  return (
    <>
      <SettingsSection
        title={t("workspace:projects.switcher.title")}
        description={t("workspace:projects.switcher.description")}
      >
        {projects.length > 0 && (
          <SettingsGroup>
            {profiles.activeProfileId === profileId ? (
              <SettingsRow label={t("workspace:projects.switcher.active")}>
                <ProjectSwitcher
                  profileId={profileId}
                  projects={projects}
                  activeProjectId={view.activeProjectId}
                />
              </SettingsRow>
            ) : (
              <SettingsRow
                label={t("workspace:projects.switcher.active")}
                description={(() => {
                  const active = projects.find((p) => p.id === view.activeProjectId)
                  return active
                    ? t("workspace:projects.switcher.inactiveWithProject", {
                        profile: view.profile.name,
                        key: active.key,
                      })
                    : t("workspace:projects.switcher.inactiveNoProject", {
                        profile: view.profile.name,
                      })
                })()}
              />
            )}
            <SettingsRow
              label={t("workspace:projects.default.label")}
              htmlFor="default-project"
              description={t("workspace:projects.default.description")}
            >
              <Select
                value={view.profile.defaultProject ?? "first"}
                onValueChange={(value) => {
                  api
                    .setDefaultProject(profileId, value === "first" ? null : value)
                    .then(() => {
                      toast.success(t("workspace:projects.default.saved"))
                      changed()
                    })
                    .catch((e: unknown) => {
                      toastFailure(t("workspace:projects.default.saveFailed"), e)
                    })
                }}
              >
                <SelectTrigger id="default-project" size="sm" className="w-56">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="first">{t("workspace:projects.default.first")}</SelectItem>
                  {projects.map((project) => (
                    <SelectItem key={project.id} value={project.id}>
                      {t("workspace:projects.option", { key: project.key, project: project.name })}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </SettingsRow>
          </SettingsGroup>
        )}
      </SettingsSection>
      <SettingsSection
        title={t("workspace:projects.cycle.title")}
        description={
          projects.length > 0
            ? t("workspace:projects.cycle.description", {
                projects: t("common:counts.projects", { count: projects.length }),
              })
            : t("workspace:projects.cycle.descriptionEmpty")
        }
      >
        {projects.length === 0 ? (
          <EmptyState variant="plain" title={t("workspace:projects.cycle.empty")}>
            {t("workspace:projects.cycle.emptyHint")}
          </EmptyState>
        ) : (
          <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
            <SortableContext
              items={projects.map((p) => p.id)}
              strategy={verticalListSortingStrategy}
            >
              <ol className="grid divide-y border-y">
                {projects.map((project, index) => (
                  <SortableProject
                    key={project.id}
                    project={project}
                    index={index}
                    active={project.id === view.activeProjectId}
                    isDefault={project.id === view.profile.defaultProject}
                    statusOf={statusOf}
                    onEdit={() => {
                      setDialog({ mode: "edit", project })
                    }}
                    onCopy={
                      otherProfiles.length > 0
                        ? () => {
                            setDialog({ mode: "copy", project })
                          }
                        : undefined
                    }
                    onDelete={async () => {
                      await api.deleteProject(profileId, project.id)
                      toast.success(t("workspace:projects.item.deleted"))
                      changed()
                    }}
                  />
                ))}
              </ol>
            </SortableContext>
          </DndContext>
        )}
      </SettingsSection>
      {dialog?.mode === "copy" && (
        <CopyProjectForm
          profileId={profileId}
          project={dialog.project}
          targets={otherProfiles}
          onClose={() => {
            setDialog(null)
          }}
          onDone={() => {
            setDialog(null)
            changed()
          }}
        />
      )}
      {dialog && dialog.mode !== "copy" && (
        <ProjectForm
          key={dialog.mode === "edit" ? dialog.project.id : "new"}
          profileId={profileId}
          project={dialog.mode === "edit" ? dialog.project : null}
          onClose={() => {
            setDialog(null)
          }}
          onDone={() => {
            setDialog(null)
            changed()
          }}
        />
      )}
    </>
  )
}

function SortableProject({
  project,
  index,
  active,
  isDefault,
  statusOf,
  onEdit,
  onCopy,
  onDelete,
}: {
  project: Project
  index: number
  active: boolean
  isDefault: boolean
  statusOf: (key: string) => ResourceStatus
  onEdit: () => void
  onCopy?: () => void
  /** Rejects with the backend's error; the confirmation then shows it. */
  onDelete: () => Promise<void>
}) {
  const { t } = useTranslation()
  const { attributes, listeners, setNodeRef, transform, transition } = useSortable({
    id: project.id,
  })
  const resources = Object.entries(project.resources)
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className="flex items-start gap-2 bg-background py-2"
      data-active={active}
    >
      <button
        type="button"
        className="cursor-grab rounded p-1 text-muted-foreground hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        aria-label={t("workspace:projects.item.reorder", { key: project.key })}
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-4" />
      </button>
      <span className="w-5 pt-1 text-right text-sm text-muted-foreground tabular-nums">
        {index + 1}.
      </span>
      <div className="grid flex-1 gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-mono text-sm font-semibold">{project.key}</span>
          <span className="text-sm">{project.name}</span>
          {active && (
            <InlineStatus tone="positive">{t("workspace:projects.item.active")}</InlineStatus>
          )}
          {isDefault && (
            <span className="text-caption text-muted-foreground">
              {t("workspace:projects.item.default")}
            </span>
          )}
        </div>
        {resources.length > 0 && (
          <ul className="grid gap-0.5 text-xs text-muted-foreground">
            {resources.map(([role, key]) => (
              <li key={role} className="flex items-center gap-1">
                <StatusIcon status={statusOf(key)} />
                {role} → <span className="font-mono">{key}</span>
                <span className="text-muted-foreground/70">
                  {t("workspace:projects.item.resourceStatus", {
                    status: t(STATUS[statusOf(key)].label),
                  })}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={t("workspace:projects.item.edit", { key: project.key })}
        onClick={onEdit}
      >
        <Pencil />
      </Button>
      {onCopy && (
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("workspace:projects.item.copy", { key: project.key })}
          title={t("workspace:projects.item.copyTooltip")}
          onClick={onCopy}
        >
          <Copy />
        </Button>
      )}
      <ConfirmDialog
        trigger={
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("workspace:projects.item.delete", { key: project.key })}
          >
            <Trash2 />
          </Button>
        }
        title={t("workspace:projects.item.deleteTitle", { key: project.key })}
        description={t("workspace:projects.item.deleteDescription")}
        onConfirm={onDelete}
        describeError={(e) =>
          failureMessage(t("workspace:projects.item.deleteFailed", { key: project.key }), e)
        }
      />
    </li>
  )
}

/** Copies a project into another profile as an independent project. */
function CopyProjectForm({
  profileId,
  project,
  targets,
  onClose,
  onDone,
}: {
  profileId: string
  project: Project
  targets: { id: string; name: string }[]
  onClose: () => void
  onDone: () => void
}) {
  const { t } = useTranslation()
  const [target, setTarget] = useState(targets[0]?.id ?? "")
  const name = targets.find((p) => p.id === target)?.name ?? ""
  return (
    <FormDialog
      open
      title={t("workspace:projects.copy.title", { key: project.key })}
      description={t("workspace:projects.copy.description")}
      submitLabel={t("workspace:projects.copy.submit")}
      canSubmit={Boolean(target)}
      onClose={onClose}
      onSubmit={async () => {
        await api.copyProject(profileId, project.id, target)
        toast.success(t("workspace:projects.copy.done", { key: project.key, profile: name }))
        onDone()
      }}
      describeError={(e) =>
        failureMessage(t("workspace:projects.copy.failed", { key: project.key }), e)
      }
    >
      <div className="grid gap-2">
        <Label htmlFor="copy-target">{t("workspace:projects.copy.target")}</Label>
        <Select value={target} onValueChange={setTarget}>
          <SelectTrigger id="copy-target" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {targets.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    </FormDialog>
  )
}

interface ResourceRow {
  role: string
  key: string
}

/** A problem with what was entered, shown as it is (not as a backend failure). */
class InvalidInput extends Error {}

function ProjectForm({
  profileId,
  project,
  onClose,
  onDone,
}: {
  profileId: string
  project: Project | null
  onClose: () => void
  onDone: () => void
}) {
  const { t } = useTranslation()
  const initialRows = Object.entries(project?.resources ?? {}).map(([role, resource]) => ({
    role,
    key: resource,
  }))
  const [key, setKey] = useState(project?.key ?? "")
  const [name, setName] = useState(project?.name ?? "")
  const [rows, setRows] = useState<ResourceRow[]>(initialRows)
  const dirty =
    key !== (project?.key ?? "") ||
    name !== (project?.name ?? "") ||
    JSON.stringify(rows) !== JSON.stringify(initialRows)
  const update = (index: number, patch: Partial<ResourceRow>) => {
    setRows((current) => current.map((row, i) => (i === index ? { ...row, ...patch } : row)))
  }
  const submit = async () => {
    const filled = rows
      .map((r) => ({ role: r.role.trim(), key: r.key.trim() }))
      .filter((r) => r.role || r.key)
    // A half-filled row or a repeated role would otherwise be saved silently
    // as an empty role or overwrite the earlier row.
    if (filled.some((r) => !r.role || !r.key)) {
      throw new InvalidInput(t("workspace:projects.form.incompleteResource"))
    }
    const roles = filled.map((r) => r.role)
    const repeated = roles.find((role, index) => roles.indexOf(role) !== index)
    if (repeated) {
      throw new InvalidInput(t("workspace:projects.form.repeatedRole", { role: repeated }))
    }
    const resources = Object.fromEntries(filled.map((r) => [r.role, r.key]))
    if (project) await api.updateProject(profileId, project.id, { key, name, resources })
    else await api.createProject(profileId, { key, name, resources })
    toast.success(t("workspace:projects.form.saved"))
    onDone()
  }
  return (
    <FormDialog
      open
      title={
        project
          ? t("workspace:projects.form.editTitle", { key: project.key })
          : t("workspace:projects.form.newTitle")
      }
      description={
        <Trans
          i18nKey="workspace:projects.form.description"
          components={{ role: <code />, key: <code className="mx-1" /> }}
        />
      }
      submitLabel={t("workspace:projects.form.save")}
      canSubmit={Boolean(key.trim())}
      dirty={dirty}
      onClose={onClose}
      onSubmit={submit}
      describeError={(e) =>
        e instanceof InvalidInput
          ? e.message
          : failureMessage(t("workspace:projects.form.saveFailed"), e)
      }
    >
      <div className="grid grid-cols-[8rem_1fr] gap-4">
        <div className="grid gap-2">
          <Label htmlFor="project-key">{t("workspace:projects.form.key")}</Label>
          <Input
            id="project-key"
            value={key}
            placeholder="SHOP"
            autoFocus
            onChange={(e) => {
              setKey(e.target.value)
            }}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="project-name">{t("workspace:projects.form.name")}</Label>
          <Input
            id="project-name"
            value={name}
            placeholder={t("workspace:projects.form.namePlaceholder")}
            onChange={(e) => {
              setName(e.target.value)
            }}
          />
        </div>
      </div>
      <div className="grid gap-2">
        <Label>{t("workspace:projects.form.resources")}</Label>
        {rows.map((row, index) => (
          <div
            key={index}
            role="group"
            aria-label={t("workspace:projects.form.resource", { number: index + 1 })}
            className="grid grid-cols-[1fr_1fr_auto] gap-2"
          >
            <Input
              aria-label={t("workspace:projects.form.role")}
              value={row.role}
              placeholder="repository"
              onChange={(e) => {
                update(index, { role: e.target.value })
              }}
            />
            <Input
              aria-label={t("workspace:projects.form.resourceKey")}
              value={row.key}
              placeholder="shop.repository"
              onChange={(e) => {
                update(index, { key: e.target.value })
              }}
            />
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={t("workspace:projects.form.removeResource")}
              onClick={() => {
                setRows((current) => current.filter((_, i) => i !== index))
              }}
            >
              <X />
            </Button>
          </div>
        ))}
        <div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              setRows((current) => [...current, { role: "", key: "" }])
            }}
          >
            <Plus /> {t("workspace:projects.form.addResource")}
          </Button>
        </div>
      </div>
    </FormDialog>
  )
}
