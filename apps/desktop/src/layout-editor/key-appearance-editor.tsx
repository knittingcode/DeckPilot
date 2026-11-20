// The Inspector's key header (docs/key-text.md, "Title appearance"): for every
// key with a display that does something — actions, Multi Actions, plugin
// actions, layout navigation, folder keys — the live key preview on the left
// (the Canvas's image of the key; it opens the key menu and never runs the
// key), and on the right the key's visible title with the title appearance
// button, and below it "Show title". Icon and background are the key menu's
// (icon-menu.tsx). For a folder key the title is the folder's name, its one
// title (`folderKeyTitle`). The folder's protected Back key and dials have
// none. Gestures, folder settings and the rest follow below it.

import { ImageIcon } from "lucide-react"
import { useId, useState } from "react"

import { KeyPreview } from "@/components/key-preview"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { useTranslation } from "@/i18n"
import { folderControlOf, folderKeyTitle } from "@/lib/folder-control"
import { controlKey, controlLabel, renamePage } from "@/lib/layout-model"
import type { ControlId, DeviceLayout } from "@/lib/types"

import {
  type SharedAppearance,
  setShownLabel,
  sharedAppearance,
  shownBinding,
  stylable,
} from "./appearance"
import { KeyMenu } from "./icon-menu"
import { type KeyLook, useKeyLook } from "./key-look-context"
import { TitleFormatButton } from "./title-appearance-editor"
import { suggestionsOf } from "./title-look"

/**
 * "Show title": whether the title is drawn on the key. Hiding keeps the
 * title, its look and a folder's name; showing restores them.
 */
function ShowTitle({
  look,
  shared,
  controls,
}: {
  look: KeyLook
  shared: SharedAppearance
  controls: ControlId[]
}) {
  const { t } = useTranslation()
  const id = useId()
  const suggestions = suggestionsOf(look, controls)
  const offered = !suggestions?.supports || suggestions.supports.includes("visibility")
  return (
    <div data-slot="show-title" className="flex h-6 min-w-0 items-center gap-2">
      <Switch
        id={id}
        size="sm"
        checked={shared.showTitle !== false}
        disabled={!offered}
        onCheckedChange={(checked) => {
          look.restyle({ showTitle: checked ? null : false })
        }}
      />
      <label htmlFor={id} className="truncate text-sm">
        {t("editor:title.show")}
      </label>
    </div>
  )
}

/** A message under the header: an icon that was not imported or pasted. */
function Problem({ message }: { message: string | null }) {
  if (!message) return null
  return (
    <p role="alert" data-slot="key-appearance-problem" className="text-xs text-destructive">
      {message}
    </p>
  )
}

/**
 * The header of one key: preview with its key menu, visible title with its
 * appearance popover, "Show title". Nothing when the key has no display or
 * nothing assigned (or outside the editor).
 */
export function KeyAppearanceEditor({
  control,
  defaultTitle,
  onChange,
}: {
  control: ControlId
  /** What the key shows without a title of its own, when the Inspector knows. */
  defaultTitle?: string
  /** Commits a layout change (the title), as the Inspector's other edits. */
  onChange: (layout: DeviceLayout, group?: string) => void
}) {
  const { t } = useTranslation()
  const look = useKeyLook()
  const [pasteProblem, setPasteProblem] = useState<string | null>(null)
  if (!look || !stylable(control)) return null
  const binding = shownBinding(look.draft, look.pageId, control)
  if (!binding) return null
  const key = controlKey(control)
  const shared = sharedAppearance(look.draft, look.pageId, [control])
  const face = look.faces.get(key)
  // A folder key's title is its folder's name: one name, edited here.
  const folder = folderControlOf(look.draft, look.pageId, control)
  const folderName = folderKeyTitle(look.draft, look.pageId, control)
  // Without a title of its own the key shows its default one (the action's,
  // or layout's name, a plugin's suggestion or live title): the agent's
  // face says which.
  const placeholder =
    folder !== undefined || binding.label
      ? undefined
      : defaultTitle || face?.title || t("editor:keyHeader.noTitle")
  const name = controlLabel(control)
  return (
    <div data-slot="key-appearance" className="@container grid gap-1.5">
      <div className="flex min-w-0 items-start gap-3">
        <KeyMenu
          look={look}
          shared={shared}
          onProblem={setPasteProblem}
          trigger={
            <button
              type="button"
              data-slot="key-preview"
              aria-label={t("editor:keyHeader.preview", { control: name })}
              title={t("editor:keyHeader.previewTitle")}
              className="relative size-14 shrink-0 overflow-hidden rounded-lg border border-device-key-border bg-black focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none @2xs:size-16"
            >
              <KeyPreview
                face={face}
                image={look.images.get(key)}
                animation={look.animations?.get(key)}
                icon={look.symbols?.get(key)}
              />
            </button>
          }
        />
        <div className="grid min-w-0 flex-1 gap-1.5">
          <div className="flex min-w-0 items-center gap-1.5">
            <Input
              data-slot="key-title"
              aria-label={
                folder !== undefined
                  ? t("editor:keyHeader.folderName")
                  : t("editor:keyHeader.title")
              }
              placeholder={placeholder}
              maxLength={folder !== undefined ? 64 : 32}
              className="h-8 min-w-0 flex-1"
              value={folderName ?? binding.label ?? ""}
              onChange={(event) => {
                const value = event.target.value
                if (folder !== undefined) {
                  // The folder's one name: its key, the navigator and the
                  // breadcrumbs follow it (one undo step per burst).
                  onChange(renamePage(look.draft, folder, value), `page-name:${folder}`)
                  return
                }
                // The visible title, exactly as typed (only the label of
                // what the key shows changes).
                onChange(setShownLabel(look.draft, look.pageId, control, value), `key-title:${key}`)
              }}
            />
            <TitleFormatButton controls={[control]} />
          </div>
          <ShowTitle look={look} shared={shared} controls={[control]} />
        </div>
      </div>
      <Problem message={pasteProblem ?? look.icons.problem} />
    </div>
  )
}

/**
 * The header of a multi-selection: how many keys, the key menu, the title
 * appearance popover and "Show title", for every selected key that has
 * something assigned.
 */
export function KeysAppearanceEditor({ controls }: { controls: ControlId[] }) {
  const { t } = useTranslation()
  const look = useKeyLook()
  const [pasteProblem, setPasteProblem] = useState<string | null>(null)
  if (!look) return null
  const styled = controls.filter(
    (c) => stylable(c) && shownBinding(look.draft, look.pageId, c) !== undefined,
  )
  if (styled.length === 0) return null
  const shared = sharedAppearance(look.draft, look.pageId, styled)
  return (
    <div data-slot="key-appearance" className="grid gap-1.5">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-sm">
          {t("editor:appearance.appliesTo", { count: styled.length })}
        </span>
        <KeyMenu
          look={look}
          shared={shared}
          onProblem={setPasteProblem}
          trigger={
            <Button
              type="button"
              variant="outline"
              size="icon-sm"
              aria-label={t("editor:keyHeader.icons")}
              title={t("editor:keyHeader.icons")}
            >
              <ImageIcon />
            </Button>
          }
        />
        <TitleFormatButton controls={styled} />
      </div>
      <ShowTitle look={look} shared={shared} controls={styled} />
      <Problem message={pasteProblem ?? look.icons.problem} />
    </div>
  )
}
