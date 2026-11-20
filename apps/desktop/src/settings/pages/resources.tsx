// Settings › Resources: this computer's resource bindings (`machine_config`).
// Resources are the core's: the `system.open-*` launchers resolve them, so
// the editor works with or without the Projects plugin, which only adds the
// project roles that use each resource.

import { open as openDialog } from "@tauri-apps/plugin-dialog"
import { FolderOpen, Pencil, Plus, Trash2 } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"

import { ConfirmDialog, FieldError, FormDialog } from "@/components/dialogs"
import { EmptyState, ErrorNotice, Loading } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { InlineStatus, SettingsSection } from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
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
import { useTranslation } from "@/i18n"
import { useFormat } from "@/i18n/format"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { failureMessage } from "@/lib/failure"
import { PROJECTS_PLUGIN, useIsPluginActive } from "@/lib/plugin-features"
import type { ResourceCheck, ResourceKind } from "@/lib/types"
import { KIND_LABELS, STATUS } from "@/lib/resource-status"
import { StatusIcon } from "@/components/resource-status-icon"

export function ResourcesSettings() {
  return <RequireProfile>{(profileId) => <Resources profileId={profileId} />}</RequireProfile>
}

function Resources({ profileId }: { profileId: string }) {
  const { revision, refresh } = useAppContext()
  const projects = useIsPluginActive(PROJECTS_PLUGIN)
  const checks = useAsync(() => api.checkResources(profileId), `${profileId}:${revision}`)
  if (checks.state.status === "loading") return <Loading />
  if (checks.state.status === "error") return <ErrorNotice error={checks.state.error} />
  // Unbound keys are those only projects ask for: not shown without them.
  const shown = projects ? checks.state.data : checks.state.data.filter((c) => c.value !== null)
  return (
    <MachineResources
      checks={shown}
      projects={projects}
      onChange={() => {
        checks.reload()
        refresh()
      }}
    />
  )
}

const KINDS = Object.keys(KIND_LABELS) as ResourceKind[]
const PICKABLE: ResourceKind[] = ["directory", "application", "terminal"]

/**
 * This computer's resource bindings: what each resource key (`shop.repository`)
 * stands for here. The core's `system.open-*` launchers use them, and so do
 * project roles while the Projects plugin is active.
 */
function MachineResources({
  checks,
  onChange,
  projects = true,
}: {
  checks: ResourceCheck[]
  onChange: () => void
  /** Whether project usage is shown (the Projects plugin is active). */
  projects?: boolean
}) {
  const { t } = useTranslation()
  const format = useFormat()
  const [editing, setEditing] = useState<ResourceCheck | "new" | null>(null)
  return (
    <SettingsSection
      title={t("workspace:projects.machine.title")}
      description={t("workspace:projects.machine.description")}
      actions={
        <Button
          size="sm"
          onClick={() => {
            setEditing("new")
          }}
        >
          <Plus /> {t("workspace:projects.machine.add")}
        </Button>
      }
    >
      {checks.length === 0 && (
        <EmptyState variant="plain" title={t("workspace:projects.machine.empty")} />
      )}
      {checks.length > 0 && (
        <div className="grid divide-y border-y">
          {checks.map((check) => (
            <div key={check.key} className="flex flex-wrap items-center gap-2 py-2">
              <StatusIcon status={check.status} />
              <div className="grid min-w-0 flex-1 gap-0.5">
                <span className="font-mono text-sm">{check.key}</span>
                <span className="truncate text-xs text-muted-foreground select-text">
                  {check.value
                    ? check.kind
                      ? t("workspace:projects.machine.value", {
                          kind: t(KIND_LABELS[check.kind]),
                          value: check.value,
                        })
                      : check.value
                    : t("workspace:projects.status.unbound")}
                </span>
                {projects && check.usedBy.length > 0 && (
                  <span className="text-xs text-muted-foreground">
                    {t("workspace:projects.machine.usedBy", {
                      count: check.usedBy.length,
                      projects: format.list(check.usedBy),
                    })}
                  </span>
                )}
              </div>
              <InlineStatus
                tone={
                  check.status === "ok"
                    ? "positive"
                    : check.status === "missing"
                      ? "warning"
                      : "neutral"
                }
              >
                {t(STATUS[check.status].label)}
              </InlineStatus>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("workspace:projects.machine.edit", { key: check.key })}
                onClick={() => {
                  setEditing(check)
                }}
              >
                <Pencil />
              </Button>
              {check.value && (
                <ConfirmDialog
                  trigger={
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("workspace:projects.machine.remove", { key: check.key })}
                    >
                      <Trash2 />
                    </Button>
                  }
                  title={t("workspace:projects.machine.removeTitle", { key: check.key })}
                  description={t("workspace:projects.machine.removeDescription")}
                  confirmLabel={t("workspace:projects.machine.removeConfirm")}
                  // Stays open, busy, until removed; a failure is shown inline.
                  onConfirm={async () => {
                    await api.removeResourceBinding(check.key)
                    onChange()
                  }}
                  describeError={(e) =>
                    failureMessage(
                      t("workspace:projects.machine.removeFailed", { key: check.key }),
                      e,
                    )
                  }
                />
              )}
            </div>
          ))}
        </div>
      )}
      {editing && (
        <BindingForm
          key={editing === "new" ? "new" : editing.key}
          check={editing === "new" ? null : editing}
          onClose={() => {
            setEditing(null)
          }}
          onDone={() => {
            setEditing(null)
            onChange()
          }}
        />
      )}
    </SettingsSection>
  )
}

function BindingForm({
  check,
  onClose,
  onDone,
}: {
  check: ResourceCheck | null
  onClose: () => void
  onDone: () => void
}) {
  const { t } = useTranslation()
  const [key, setKey] = useState(check?.key ?? "")
  const [kind, setKind] = useState<ResourceKind>(check?.kind ?? "directory")
  const [value, setValue] = useState(check?.value ?? "")
  const [browseError, setBrowseError] = useState<string | null>(null)
  const dirty =
    key !== (check?.key ?? "") ||
    kind !== (check?.kind ?? "directory") ||
    value !== (check?.value ?? "")
  const browse = async () => {
    setBrowseError(null)
    try {
      const picked = await openDialog({ directory: kind === "directory", multiple: false })
      if (typeof picked === "string") setValue(picked)
    } catch (e) {
      setBrowseError(
        failureMessage(
          t("workspace:projects.binding.browseFailed"),
          e,
          t("workspace:projects.binding.browseNext"),
        ),
      )
    }
  }
  return (
    <FormDialog
      open
      title={
        check
          ? t("workspace:projects.binding.editTitle", { key: check.key })
          : t("workspace:projects.binding.newTitle")
      }
      description={t("workspace:projects.binding.description")}
      submitLabel={t("workspace:projects.form.save")}
      canSubmit={Boolean(key.trim() && value.trim())}
      dirty={dirty}
      onClose={onClose}
      onSubmit={async () => {
        await api.setResourceBinding(key, kind, value)
        toast.success(t("workspace:projects.binding.saved"))
        onDone()
      }}
      describeError={(e) => failureMessage(t("workspace:projects.binding.saveFailed", { key }), e)}
    >
      <div className="grid grid-cols-[1fr_12rem] gap-4">
        <div className="grid gap-2">
          <Label htmlFor="binding-key">{t("workspace:projects.form.resourceKey")}</Label>
          <Input
            id="binding-key"
            value={key}
            disabled={Boolean(check)}
            placeholder="shop.repository"
            onChange={(e) => {
              setKey(e.target.value)
            }}
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="binding-kind">{t("workspace:projects.binding.kind")}</Label>
          <Select
            value={kind}
            onValueChange={(next) => {
              setKind(next as ResourceKind)
            }}
          >
            <SelectTrigger id="binding-kind" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {KINDS.map((option) => (
                <SelectItem key={option} value={option}>
                  {t(KIND_LABELS[option])}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>
      <div className="grid gap-2">
        <Label htmlFor="binding-value">{t("workspace:projects.binding.value")}</Label>
        <div className="flex gap-2">
          <Input
            id="binding-value"
            value={value}
            aria-invalid={browseError ? true : undefined}
            aria-describedby={browseError ? "binding-value-error" : undefined}
            placeholder={
              kind === "url" ? "https://…" : t("workspace:projects.binding.valuePlaceholder")
            }
            onChange={(e) => {
              setValue(e.target.value)
            }}
          />
          {PICKABLE.includes(kind) && (
            <Button type="button" variant="outline" onClick={() => void browse()}>
              <FolderOpen /> {t("workspace:projects.binding.browse")}
            </Button>
          )}
        </div>
        <FieldError id="binding-value-error" message={browseError} />
      </div>
    </FormDialog>
  )
}
