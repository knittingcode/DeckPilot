import {
  Check,
  ChevronDown,
  Copy,
  LayoutGrid,
  Library,
  Monitor,
  Pencil,
  Plus,
  Star,
  Trash2,
} from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { DestructiveDialog, FormDialog } from "@/components/dialogs"
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
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { useAsync } from "@/hooks/use-async"
import { useFormat } from "@/i18n/format"
import { i18n, useTranslation } from "@/i18n"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { failureMessage, toastFailure } from "@/lib/failure"
import { shownLayout } from "@/lib/layout-navigation"
import type { DeviceLayout, DeviceModel, SharedLayoutSummary } from "@/lib/types"
import { cn } from "@/lib/utils"
import { selectedDevice } from "@/shell/device-options"
import { useEditorSelection } from "@/shell/editor-selection-context"

import { defaultLayoutOf, resolveLayout, useConfirmDiscard } from "./layout-selection"

/**
 * Layout selection and management of the Device Editor, shown in the main
 * window's toolbar (`LayoutSelector` in shell/toolbar.tsx): the profile's
 * layouts for the device's model, the model's other layouts to use in the
 * profile, and new, duplicate, rename, default and delete. Layouts are
 * shared between profiles; everything here goes by the layout's id and
 * never copies a layout behind the user's back.
 */

const profilesLabel = (count: number) => i18n.t("layouts:menu.sharedProfiles", { count })

type Dialog =
  | { kind: "new" }
  | { kind: "rename"; layout: DeviceLayout }
  | { kind: "duplicate"; layout: DeviceLayout }
  | { kind: "delete"; layout: DeviceLayout }

/**
 * The toolbar's layout selector: a compact dropdown with the layouts of
 * the profile being edited for the device's model, then a separate section
 * that manages them. Layouts of other models are listed, but cannot be
 * chosen.
 */
export function LayoutMenu({ className }: { className?: string }) {
  const { t } = useTranslation()
  const format = useFormat()
  const { editingProfileId, revision, refresh, agent, navigate } = useAppContext()
  const selection = useEditorSelection()
  const confirmDiscard = useConfirmDiscard()
  const profileId = editingProfileId
  const view = useAsync(
    () => (profileId ? api.getProfile(profileId) : Promise.resolve(null)),
    `layout-menu:${profileId ?? ""}:${revision}`,
  )
  const sharedState = useAsync(() => api.listSharedLayouts(), `layout-menu-shared:${revision}`)
  const [dialog, setDialog] = useState<Dialog | null>(null)

  const models = selection.models.status === "ready" ? selection.models.data : []
  const model = models.find((m) => m.id === selection.modelId)
  const profile = view.state.status === "ready" ? view.state.data?.profile : undefined
  const all = profile?.layouts ?? []
  const layouts = model ? all.filter((l) => l.model === model.id) : []
  const otherModels = model ? all.filter((l) => l.model !== model.id) : []
  const device = model
    ? selectedDevice(agent?.status?.devices ?? [], model.id, selection.device)
    : null
  const onDevice = shownLayout(device)
  const defaultLayout = model
    ? defaultLayoutOf(
        layouts,
        profile?.activeLayouts?.[model.id],
        onDevice,
        agent?.status?.activeProfile?.id === profileId,
      )
    : null
  const current = resolveLayout(layouts, selection.layoutId, defaultLayout)
  const shared = sharedState.state.status === "ready" ? sharedState.state.data : null
  const summary = (id: string) => shared?.find((l) => l.id === id)
  const own = new Set(all.map((l) => l.id))
  const others = (shared ?? [])
    .filter((l) => l.model === model?.id && !own.has(l.id))
    .sort((a, b) => a.name.localeCompare(b.name))
  const modelName = (id: string): string =>
    models.find((m) => m.id === id)?.name ??
    shared?.find((l) => l.model === id)?.modelName ??
    t("layouts:menu.anotherModel")
  const ready = Boolean(profileId && model && profile)

  const changed = (message: string, id?: string) => {
    toast.success(message)
    if (id && model) selection.showLayout(id, model.id)
    refresh()
  }
  const choose = async (id: string) => {
    if (id === current?.id) return
    if (!(await confirmDiscard())) return
    selection.selectLayout(id)
  }
  const use = async (layout: SharedLayoutSummary) => {
    if (!profileId || !model || !(await confirmDiscard())) return
    api.assignLayout(profileId, layout.id).then(
      () => {
        changed(t("layouts:menu.toast.assigned", { layout: layout.name }), layout.id)
      },
      (e: unknown) => {
        toastFailure(t("layouts:menu.toast.assignFailed", { layout: layout.name }), e)
      },
    )
  }
  const makeDefault = (layout: DeviceLayout) => {
    if (!profileId || !model) return
    api.activateLayout(profileId, layout.id).then(
      () => {
        changed(
          t("layouts:menu.toast.madeDefault", { layout: layout.name, model: model.name }),
          layout.id,
        )
      },
      (e: unknown) => {
        toastFailure(t("layouts:menu.toast.defaultFailed", { layout: layout.name }), e)
      },
    )
  }

  const label = current
    ? current.id === defaultLayout
      ? t("layouts:menu.selectorDefault", { layout: current.name })
      : t("layouts:menu.selector", { layout: current.name })
    : t("layouts:menu.choose")
  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            className={className}
            aria-label={label}
            title={label}
            disabled={!ready}
          >
            <LayoutGrid />
            <span className="truncate">{current?.name ?? t("layouts:menu.placeholder")}</span>
            <ChevronDown className="opacity-60" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-80">
          <DropdownMenuGroup aria-label={t("layouts:menu.profileLayouts")}>
            <DropdownMenuLabel className="text-xs text-muted-foreground">
              {layouts.length === 0
                ? model
                  ? t("layouts:menu.noneForModel", { model: model.name })
                  : t("layouts:menu.noneForDevice")
                : model
                  ? t("layouts:menu.forModel", { model: model.name })
                  : t("layouts:menu.forDevice")}
            </DropdownMenuLabel>
            {layouts.map((layout) => {
              const users = summary(layout.id)?.profiles.length ?? 1
              const marks = [
                layout.id === defaultLayout && t("layouts:menu.defaultMark"),
                users > 1 && profilesLabel(users),
              ].filter(Boolean)
              return (
                <DropdownMenuItem
                  key={layout.id}
                  aria-current={layout.id === current?.id ? "true" : undefined}
                  onSelect={() => {
                    void choose(layout.id)
                  }}
                >
                  <Check
                    className={cn(layout.id === current?.id ? "opacity-100" : "opacity-0")}
                    aria-hidden
                  />
                  <span className="min-w-0 flex-1 truncate">{layout.name}</span>
                  {layout.id === onDevice && (
                    <Monitor
                      className="text-emerald-500"
                      aria-label={t("layouts:menu.onDevice")}
                      role="img"
                    />
                  )}
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
              <DropdownMenuGroup
                aria-label={t("layouts:menu.otherLayouts", { model: model?.name ?? "" })}
              >
                <DropdownMenuLabel className="text-xs text-muted-foreground">
                  {t("layouts:menu.otherLayouts", { model: model?.name ?? "" })}
                </DropdownMenuLabel>
                {others.map((layout) => (
                  <DropdownMenuItem
                    key={layout.id}
                    aria-label={t("layouts:menu.useLayout", { layout: layout.name })}
                    onSelect={() => {
                      void use(layout)
                    }}
                  >
                    <span className="grid min-w-0 flex-1">
                      <span className="truncate">{layout.name}</span>
                      <span className="truncate text-xs text-muted-foreground">
                        {layout.profiles.length === 0
                          ? t("layouts:menu.unused")
                          : t("layouts:menu.usedBy", {
                              profiles: format.list(
                                layout.profiles
                                  .filter((p) => p.profileId !== profileId)
                                  .map((p) => p.profileName),
                              ),
                            })}
                      </span>
                    </span>
                    <span className="text-xs">{t("layouts:menu.use")}</span>
                  </DropdownMenuItem>
                ))}
              </DropdownMenuGroup>
            </>
          )}
          {otherModels.length > 0 && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuGroup aria-label={t("layouts:menu.otherModels")}>
                <DropdownMenuLabel className="text-xs text-muted-foreground">
                  {t("layouts:menu.notForDevice")}
                </DropdownMenuLabel>
                {otherModels.map((layout) => (
                  <DropdownMenuItem
                    key={layout.id}
                    disabled
                    title={t("layouts:menu.madeFor", { model: modelName(layout.model) })}
                  >
                    <span className="min-w-0 flex-1 truncate">{layout.name}</span>
                    <span className="text-xs text-muted-foreground">{modelName(layout.model)}</span>
                  </DropdownMenuItem>
                ))}
              </DropdownMenuGroup>
            </>
          )}
          <DropdownMenuSeparator />
          <DropdownMenuGroup aria-label={t("layouts:menu.manageGroup")}>
            <DropdownMenuItem
              onSelect={() => {
                setDialog({ kind: "new" })
              }}
            >
              <Plus /> {t("layouts:menu.new")}
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={!current}
              onSelect={() => {
                if (current) setDialog({ kind: "duplicate", layout: current })
              }}
            >
              <Copy /> {t("layouts:menu.duplicate")}
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={!current}
              onSelect={() => {
                if (!current) return
                const layout = current
                void confirmDiscard().then((ok) => {
                  if (ok) setDialog({ kind: "rename", layout })
                })
              }}
            >
              <Pencil /> {t("layouts:menu.rename")}
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={!current || current.id === defaultLayout}
              title={model ? t("layouts:menu.defaultHint", { model: model.name }) : undefined}
              onSelect={() => {
                if (current) makeDefault(current)
              }}
            >
              <Star />{" "}
              {current && current.id === defaultLayout
                ? t("layouts:menu.isDefault")
                : t("layouts:menu.setDefault")}
            </DropdownMenuItem>
            <DropdownMenuItem
              variant="destructive"
              disabled={!current}
              onSelect={() => {
                if (current) setDialog({ kind: "delete", layout: current })
              }}
            >
              <Trash2 /> {t("layouts:menu.delete")}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={() => {
                navigate("layouts")
              }}
            >
              <Library /> {t("layouts:menu.manage")}
            </DropdownMenuItem>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      {profileId && model && dialog?.kind === "new" && (
        <NewLayoutDialog
          model={model}
          onClose={() => {
            setDialog(null)
          }}
          onCreate={async (name, generate) => {
            if (!(await confirmDiscard())) return
            const { id } = await api.createLayout(profileId, model.id, name, generate)
            setDialog(null)
            changed(t("layouts:page.toast.created"), id)
          }}
        />
      )}
      {profileId && dialog?.kind === "duplicate" && (
        <NameDialog
          title={t("layouts:management.duplicateTitle", { layout: dialog.layout.name })}
          description={t("layouts:management.duplicateDescription")}
          confirmLabel={t("layouts:page.duplicate.confirm")}
          initial={t("layouts:management.copyName", { layout: dialog.layout.name })}
          onClose={() => {
            setDialog(null)
          }}
          onSubmit={async (name) => {
            if (!(await confirmDiscard())) return
            const id = await api.duplicateLayout(profileId, dialog.layout.id, name)
            setDialog(null)
            changed(t("layouts:page.toast.duplicated"), id)
          }}
          describeError={(e) =>
            failureMessage(
              t("layouts:management.duplicateFailed", { layout: dialog.layout.name }),
              e,
            )
          }
        />
      )}
      {dialog?.kind === "rename" && (
        <NameDialog
          title={t("layouts:management.renameTitle", { layout: dialog.layout.name })}
          description={
            (summary(dialog.layout.id)?.profiles.length ?? 1) > 1
              ? t("layouts:management.renameShared")
              : undefined
          }
          confirmLabel={t("layouts:page.rename.confirm")}
          initial={dialog.layout.name}
          onClose={() => {
            setDialog(null)
          }}
          onSubmit={async (name) => {
            await api.renameSharedLayout(dialog.layout.id, name)
            setDialog(null)
            changed(t("layouts:page.toast.renamed"))
          }}
          describeError={(e) =>
            failureMessage(t("layouts:management.renameFailed", { layout: dialog.layout.name }), e)
          }
        />
      )}
      {profileId && dialog?.kind === "delete" && (
        <DestructiveDialog
          open
          onOpenChange={(open) => {
            if (!open) setDialog(null)
          }}
          title={t("layouts:page.delete.title", { layout: dialog.layout.name })}
          description={
            (summary(dialog.layout.id)?.profiles.length ?? 1) > 1
              ? t("layouts:management.deleteShared")
              : t("layouts:management.deleteOwn")
          }
          busyLabel={t("layouts:management.deleting")}
          onConfirm={async () => {
            await api.deleteLayout(profileId, dialog.layout.id)
            selection.setDirty(false)
            setDialog(null)
            changed(t("layouts:page.toast.deleted"))
          }}
          describeError={(e) =>
            failureMessage(
              t("layouts:management.deleteFailed", { layout: dialog.layout.name }),
              e,
              t("layouts:management.deleteFailedHint"),
            )
          }
        />
      )}
    </>
  )
}

/** Asks for a layout's name: rename and duplicate. */
function NameDialog({
  title,
  description,
  confirmLabel,
  initial,
  onClose,
  onSubmit,
  describeError,
}: {
  title: string
  description?: string
  confirmLabel: string
  initial: string
  onClose: () => void
  /** Rejects with the backend's error; the dialog then stays open and shows it. */
  onSubmit: (name: string) => Promise<void>
  describeError: (error: unknown) => string
}) {
  const { t } = useTranslation()
  const [name, setName] = useState(initial)
  const trimmed = name.trim()
  return (
    <FormDialog
      open
      size="compact"
      title={title}
      description={description}
      submitLabel={confirmLabel}
      canSubmit={Boolean(trimmed)}
      dirty={name !== initial}
      onClose={onClose}
      onSubmit={() => onSubmit(trimmed)}
      describeError={describeError}
    >
      <div className="grid gap-1.5">
        <Label htmlFor="layout-name">{t("layouts:page.create.name")}</Label>
        <Input
          id="layout-name"
          autoFocus
          value={name}
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
    </FormDialog>
  )
}

/** A new layout of the profile for one device model. */
function NewLayoutDialog({
  model,
  onClose,
  onCreate,
}: {
  model: DeviceModel
  onClose: () => void
  /** Rejects with the backend's error; the dialog then stays open and shows it. */
  onCreate: (name: string, generate: boolean) => Promise<void>
}) {
  const { t } = useTranslation()
  const [name, setName] = useState("")
  const [generate, setGenerate] = useState(true)
  return (
    <FormDialog
      open
      title={t("layouts:management.newTitle", { model: model.name })}
      submitLabel={t("layouts:page.create.submit")}
      dirty={name.trim() !== ""}
      onClose={onClose}
      onSubmit={() => onCreate(name, generate)}
      describeError={(e) =>
        failureMessage(t("layouts:management.createFailed", { layout: name }), e)
      }
    >
      <div className="grid gap-2">
        <Label htmlFor="layout-name">{t("layouts:page.create.name")}</Label>
        <Input
          id="layout-name"
          value={name}
          autoFocus
          placeholder={t("layouts:management.newPlaceholder", { model: model.name })}
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
      </div>
      <label className="flex items-center gap-3 text-sm">
        <Switch checked={generate} onCheckedChange={setGenerate} />
        {t("layouts:management.generate")}
      </label>
    </FormDialog>
  )
}
