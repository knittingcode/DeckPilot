// The settings of the folder a key opens, in that key's inspector: Auto
// Exit, and for a folder whose Back is still on its first free key
// ("legacy") the one-click "Move Back to the first key". Its name is the
// key header's title field (the folder's one title, `folderKeyTitle`).
// Only controls, no headings or explanations. The folder's own protected Back key has an
// empty inspector; the page navigator's "Close after a key runs" stays in
// its menu.

import { useContext, useId, useState } from "react"
import { ArrowLeft, ArrowRightLeft, Trash2 } from "lucide-react"

import { DialogsContext } from "@/components/dialogs/use-dialogs"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { useTranslation } from "@/i18n"
import {
  autoExitOf,
  backPosition,
  checkAutoExitSeconds,
  isFolderPage,
  moveBackToReservedKey,
  reservedBackControl,
  setAutoExit,
} from "@/lib/folder-back"
import {
  discardRecovered,
  recoveredFolderLabel,
  recoveredOf,
  resolveFolderLabel,
  restoreRecovered,
} from "@/lib/folder-control"
import { controlLabel, page as findPage } from "@/lib/layout-model"
import type { ControlId, DeviceLayout, DeviceModel } from "@/lib/types"

interface FolderSettingsProps {
  model: DeviceModel
  draft: DeviceLayout
  pageId: string
  /** Commits a change; `group` merges consecutive edits into one undo step. */
  onChange: (layout: DeviceLayout, group?: string) => void
  /** First in its panel (a folder key's Inspector): no separator above it. */
  first?: boolean
}

/** The folder `pageId`'s Auto Exit, and "Move Back to the first key" for a legacy one. */
export function FolderSettings({
  model,
  draft,
  pageId,
  onChange,
  first = false,
}: FolderSettingsProps) {
  const { t } = useTranslation()
  const id = useId()
  const page = findPage(draft, pageId)
  const stored = autoExitOf(page)
  // What is typed: committed while it is valid, kept (with why) when not.
  const [text, setText] = useState(String(stored.seconds))
  const [synced, setSynced] = useState(stored.seconds)
  if (stored.seconds !== synced) {
    // Undo, redo or revert changed it.
    setSynced(stored.seconds)
    if (Number(text) !== stored.seconds) setText(String(stored.seconds))
  }
  const [moveRefused, setMoveRefused] = useState(false)
  if (!page || !isFolderPage(draft, pageId)) return null
  const error = checkAutoExitSeconds(text)
  const shownSeconds = error ? stored.seconds : Number(text)
  const legacy = backPosition(draft, pageId) === "legacy"
  return (
    <section
      data-slot="folder-settings"
      aria-label={t("editor:folderSettings.title")}
      className={first ? "grid gap-3" : "grid gap-3 border-t pt-4"}
    >
      <label className="flex items-center gap-2 text-sm" htmlFor={`${id}-enabled`}>
        <Switch
          id={`${id}-enabled`}
          checked={stored.enabled}
          onCheckedChange={(checked) => {
            onChange(setAutoExit(draft, pageId, { ...stored, enabled: checked }))
          }}
        />
        {t("editor:folderSettings.autoExit")}
      </label>
      {stored.enabled && (
        <div className="grid gap-1.5">
          <Label htmlFor={`${id}-seconds`}>{t("editor:folderSettings.exitAfter")}</Label>
          <div className="flex items-center gap-2">
            <Input
              id={`${id}-seconds`}
              inputMode="numeric"
              className="w-24"
              value={text}
              aria-invalid={error ? true : undefined}
              aria-describedby={error ? `${id}-error` : undefined}
              onChange={(event) => {
                const value = event.target.value
                setText(value)
                if (checkAutoExitSeconds(value) === null && Number(value) !== stored.seconds) {
                  const seconds = Number(value)
                  setSynced(seconds)
                  onChange(
                    setAutoExit(draft, pageId, { ...stored, seconds }),
                    `auto-exit:${pageId}`,
                  )
                }
              }}
            />
            <span className="text-sm text-muted-foreground">
              {t("editor:folderSettings.seconds", { count: shownSeconds })}
            </span>
          </div>
          {error && (
            <p id={`${id}-error`} role="alert" className="text-xs text-destructive">
              {t(error, { min: 1, max: 3600 })}
            </p>
          )}
        </div>
      )}
      {legacy && reservedBackControl(model) && (
        <div data-slot="legacy-back" className="grid gap-2">
          <Button
            variant="outline"
            size="sm"
            className="w-fit"
            onClick={() => {
              const moved = moveBackToReservedKey(model, draft, pageId)
              if (!moved) {
                setMoveRefused(true)
                return
              }
              setMoveRefused(false)
              onChange(moved.layout)
            }}
          >
            <ArrowLeft /> {t("editor:folderSettings.moveBack")}
          </Button>
          {moveRefused && (
            <p role="alert" className="text-xs text-destructive">
              {t("editor:folderSettings.noFreeKey")}
            </p>
          )}
        </div>
      )}
    </section>
  )
}

interface FolderControlPanelProps {
  model: DeviceModel
  draft: DeviceLayout
  /** The page the folder key is on. */
  pageId: string
  control: ControlId
  /** The folder the key opens. */
  folder: string
  onChange: (layout: DeviceLayout, group?: string) => void
}

/**
 * The Inspector of a folder key (`lib/folder-control.ts`): it only opens
 * its folder, so it shows the folder's settings and nothing else — Auto
 * Exit, the title and the gestures a layout from before kept on the key
 * when there are any. The folder's name is the key header's title. Its look is the appearance panel below it
 * (inspector.tsx). The folder opens with a double click on its key (or
 * the key's menu), so there is no button for it here.
 */
export function FolderControlPanel({
  model,
  draft,
  pageId,
  control,
  folder,
  onChange,
}: FolderControlPanelProps) {
  return (
    <>
      <FolderSettings
        key={folder}
        model={model}
        draft={draft}
        pageId={folder}
        onChange={onChange}
        first
      />
      <RecoveredTitle draft={draft} pageId={pageId} control={control} onChange={onChange} />
      <RecoveredGestures
        model={model}
        draft={draft}
        pageId={pageId}
        control={control}
        onChange={onChange}
      />
    </>
  )
}

/**
 * The title a folder key had of its own before the folder's name became
 * its one title (migration 13, an import): offered as the folder's name or
 * discarded. Nothing when the key kept none.
 */
function RecoveredTitle({
  draft,
  pageId,
  control,
  onChange,
}: {
  draft: DeviceLayout
  pageId: string
  control: ControlId
  onChange: (layout: DeviceLayout, group?: string) => void
}) {
  const { t } = useTranslation()
  const label = recoveredFolderLabel(draft, pageId, control)
  if (label === undefined) return null
  return (
    <section
      data-slot="recovered-title"
      aria-label={t("editor:folderControl.recoveredTitle.title")}
      className="grid gap-2 border-t pt-4"
    >
      <p className="text-sm">{t("editor:folderControl.recoveredTitle.text", { label })}</p>
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            onChange(resolveFolderLabel(draft, pageId, control, true))
          }}
        >
          {t("editor:folderControl.recoveredTitle.use")}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            onChange(resolveFolderLabel(draft, pageId, control, false))
          }}
        >
          <Trash2 /> {t("editor:folderControl.recoveredTitle.discard")}
        </Button>
      </div>
    </section>
  )
}

/**
 * "Recovered gestures (n)": what a folder key ran besides opening its
 * folder before folder keys were core controls (migration 12, an import).
 * Never run; moved to the page's first free key, or discarded after the
 * shared destructive confirmation. Nothing when the key has none.
 */
function RecoveredGestures({
  model,
  draft,
  pageId,
  control,
  onChange,
}: {
  model: DeviceModel
  draft: DeviceLayout
  pageId: string
  control: ControlId
  onChange: (layout: DeviceLayout, group?: string) => void
}) {
  const { t } = useTranslation()
  const dialogs = useContext(DialogsContext)
  const [refused, setRefused] = useState(false)
  const count = recoveredOf(draft, pageId, control).length
  if (count === 0) return null
  return (
    <section
      data-slot="recovered-gestures"
      aria-label={t("editor:folderControl.recovered.title", { count })}
      className="grid gap-2 border-t pt-4"
    >
      <p className="text-sm font-medium">{t("editor:folderControl.recovered.title", { count })}</p>
      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            const moved = restoreRecovered(model, draft, pageId, control)
            if (!moved) {
              setRefused(true)
              return
            }
            setRefused(false)
            onChange(moved.layout)
          }}
        >
          <ArrowRightLeft /> {t("editor:folderControl.recovered.move")}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            void (async () => {
              const ok = dialogs
                ? await dialogs.destructive({
                    title: t("editor:folderControl.recovered.discardTitle", { count }),
                    description: t("editor:folderControl.recovered.discardDescription", {
                      control: controlLabel(control),
                    }),
                    confirmLabel: t("editor:folderControl.recovered.discard"),
                  })
                : false
              if (ok) onChange(discardRecovered(draft, pageId, control))
            })()
          }}
        >
          <Trash2 /> {t("editor:folderControl.recovered.discard")}
        </Button>
      </div>
      {refused && (
        <p role="alert" className="text-xs text-destructive">
          {t("editor:folderControl.recovered.noFreeKey")}
        </p>
      )}
    </section>
  )
}
