import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { controlKey, controlLabel } from "@/lib/layout-model"
import type { ControlId, DeviceLayout, FontSize, IconInfo, TitleAlign } from "@/lib/types"
import { cn } from "@/lib/utils"

import {
  type AppearancePatch,
  type KeyLookChange,
  contrastHint,
  sharedAppearance,
  shownBinding,
  stylable,
  withIconFit,
} from "./appearance"
import { ColorField } from "./color-field"
import { IconLibrary } from "./icon-library"

const FONT_SIZES: { value: FontSize; label: string }[] = [
  { value: "small", label: "Small" },
  { value: "medium", label: "Medium" },
  { value: "large", label: "Large" },
]

const ALIGNS: { value: TitleAlign | "auto"; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "top", label: "Top" },
  { value: "middle", label: "Middle" },
  { value: "bottom", label: "Bottom" },
]

export interface IconState {
  icons: IconInfo[]
  usage: (icon: IconInfo) => number
  busy: boolean
  problem: string | null
  importFile: (file: File) => Promise<IconInfo | null>
  remove: (icon: IconInfo) => void
}

/**
 * How the selected keys look: colours, title, icon. Changes apply to every
 * selected key that has something assigned, as one undo step.
 */
export function AppearancePanel({
  draft,
  pageId,
  controls,
  images,
  icons,
  onChange,
}: {
  draft: DeviceLayout
  pageId: string
  controls: ControlId[]
  /** Rendered key images of the page, by control key. */
  images: Map<string, string>
  icons: IconState
  onChange: (change: KeyLookChange, group?: string) => void
}) {
  const styled = controls.filter((c) => stylable(c) && shownBinding(draft, pageId, c))
  if (controls.every((c) => !stylable(c))) {
    return <p className="text-xs text-muted-foreground">Dials have no display to style.</p>
  }
  if (styled.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        Assign an action, a workflow or a page first; the look belongs to what the key does.
      </p>
    )
  }
  const shared = sharedAppearance(draft, pageId, styled)
  const set = (patch: AppearancePatch, group?: string) => {
    onChange(patch, group)
  }
  const hint = contrastHint(
    shared.background === "mixed" ? undefined : shared.background,
    shared.textColor === "mixed" ? undefined : shared.textColor,
  )
  const skipped = controls.length - styled.length
  const preview = styled.length === 1 && styled[0] ? images.get(controlKey(styled[0])) : undefined

  return (
    <section className="grid gap-4" aria-label="Appearance">
      <div className="flex items-start gap-3">
        <h4 className="flex-1 text-sm font-medium">
          Appearance
          {styled.length > 1 && (
            <span className="block text-xs font-normal text-muted-foreground">
              Applies to {styled.length} keys
            </span>
          )}
        </h4>
        {preview && (
          <img
            src={preview}
            alt={`Rendered preview of ${controlLabel(styled[0] as ControlId)}`}
            className="size-16 rounded-lg border bg-black"
          />
        )}
      </div>
      {skipped > 0 && (
        <p className="text-xs text-muted-foreground">
          {skipped} selected {skipped === 1 ? "control has" : "controls have"} nothing assigned or
          no display and {skipped === 1 ? "keeps its" : "keep their"} look.
        </p>
      )}

      <ColorField
        label="Background"
        value={shared.background}
        onChange={(value, group) => {
          set({ background: value }, group)
        }}
      />
      <ColorField
        label="Text colour"
        value={shared.textColor}
        onChange={(value, group) => {
          set({ textColor: value }, group)
        }}
      />
      <p
        className={cn(
          "text-xs",
          hint.level === "good" ? "text-muted-foreground" : "text-amber-700 dark:text-amber-400",
        )}
        aria-live="polite"
      >
        {hint.message}
      </p>

      <div className="grid gap-2">
        <Label id="appearance-font-size">Text size</Label>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-labelledby="appearance-font-size"
          value={shared.fontSize === "mixed" ? "" : (shared.fontSize ?? "medium")}
          onValueChange={(value) => {
            // Medium is the default and not stored.
            if (value) set({ fontSize: value === "medium" ? null : (value as FontSize) })
          }}
        >
          {FONT_SIZES.map((size) => (
            <ToggleGroupItem key={size.value} value={size.value} className="px-2.5">
              {size.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>

      <div className="grid gap-2">
        <Label id="appearance-title-align">Title position</Label>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-labelledby="appearance-title-align"
          value={shared.titleAlign === "mixed" ? "" : (shared.titleAlign ?? "auto")}
          onValueChange={(value) => {
            if (value) set({ titleAlign: value === "auto" ? null : (value as TitleAlign) })
          }}
        >
          {ALIGNS.map((align) => (
            <ToggleGroupItem
              key={align.value}
              value={align.value}
              className="px-2.5"
              title={align.value === "auto" ? "Middle, or bottom with an icon" : undefined}
            >
              {align.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>

      <label className="flex items-center gap-3 text-sm">
        <Switch
          checked={shared.showTitle !== false}
          onCheckedChange={(checked) => {
            set({ showTitle: checked ? null : false })
          }}
        />
        Show title
      </label>

      <IconLibrary
        icons={icons.icons}
        usage={icons.usage}
        selected={shared.iconId}
        fit={shared.iconFit}
        busy={icons.busy}
        problem={icons.problem}
        onSelect={(id) => {
          if (!id) {
            set({ icon: null })
            return
          }
          const fit = shared.iconFit === "mixed" ? undefined : shared.iconFit
          set({ icon: fit && fit !== "fit" ? { id, fit } : { id } })
        }}
        onFit={(fit) => {
          onChange(withIconFit(fit))
        }}
        onImport={(file) => {
          void icons.importFile(file).then((icon) => {
            // A new icon is put on the selected keys right away.
            if (icon) set({ icon: { id: icon.id } })
          })
        }}
        onDelete={icons.remove}
      />
    </section>
  )
}
