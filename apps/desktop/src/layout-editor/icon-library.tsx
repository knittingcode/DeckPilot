import { ImageOff, Trash2, Upload } from "lucide-react"
import { useId, useRef } from "react"

import { Button } from "@/components/ui/button"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import type { IconFit, IconInfo } from "@/lib/types"
import { cn } from "@/lib/utils"

import { FIT_OBJECT, MAX_ICONS, type Shared } from "./appearance"

const FITS: { value: IconFit; label: string; hint: string }[] = [
  { value: "fit", label: "Fit", hint: "Whole icon, aspect kept" },
  { value: "fill", label: "Fill", hint: "Stretched to the key" },
  { value: "crop", label: "Crop", hint: "Covers the key, edges cut off" },
]

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`

/**
 * The profile's icons: choose one for the selected keys, import PNG or JPEG
 * files, delete unused ones, and choose how the icon fills the key.
 */
export function IconLibrary({
  icons,
  usage,
  selected,
  fit,
  busy,
  problem,
  onSelect,
  onFit,
  onImport,
  onDelete,
}: {
  icons: IconInfo[]
  /** Keys showing each icon, counting the unsaved layout. */
  usage: (icon: IconInfo) => number
  selected: Shared<string>
  fit: Shared<IconFit>
  busy: boolean
  problem: string | null
  onSelect: (id: string | null) => void
  onFit: (fit: IconFit) => void
  onImport: (file: File) => void
  onDelete: (icon: IconInfo) => void
}) {
  const input = useRef<HTMLInputElement>(null)
  const problemId = useId()
  const chosen = selected === "mixed" ? undefined : icons.find((i) => i.id === selected)
  const shownFit = fit === "mixed" || fit === undefined ? "fit" : fit

  return (
    <fieldset className="grid gap-2">
      <legend className="mb-1 text-sm font-medium">Icon</legend>
      <div className="flex flex-wrap items-center gap-2">
        <input
          ref={input}
          type="file"
          accept="image/png,image/jpeg"
          className="sr-only"
          aria-label="Icon file"
          aria-describedby={problem ? problemId : undefined}
          onChange={(e) => {
            const file = e.target.files?.[0]
            if (file) onImport(file)
            e.target.value = ""
          }}
        />
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={busy || icons.length >= MAX_ICONS}
          onClick={() => input.current?.click()}
        >
          <Upload /> Import icon
        </Button>
        <span className="text-xs text-muted-foreground">
          {icons.length} of {MAX_ICONS} · PNG or JPEG, up to 2 MiB
        </span>
      </div>
      {problem && (
        <p id={problemId} role="alert" className="text-xs text-destructive">
          {problem}
        </p>
      )}
      <ul className="grid grid-cols-5 gap-1" aria-label="Icons">
        <li>
          <button
            type="button"
            aria-pressed={selected === undefined}
            aria-label="No icon"
            title="No icon"
            className={cn(
              "flex aspect-square w-full items-center justify-center rounded-md border text-muted-foreground hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
              selected === undefined && "ring-2 ring-primary",
            )}
            onClick={() => {
              onSelect(null)
            }}
          >
            <ImageOff className="size-4" />
          </button>
        </li>
        {icons.map((icon) => {
          const used = usage(icon)
          return (
            <li key={icon.id} className="group relative">
              <button
                type="button"
                aria-pressed={selected === icon.id}
                aria-label={`${icon.name}, used by ${plural(used, "key")}`}
                title={`${icon.name} · ${icon.width}×${icon.height} · used by ${plural(used, "key")}`}
                className={cn(
                  "block aspect-square w-full overflow-hidden rounded-md border bg-neutral-900 p-0.5 hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                  selected === icon.id && "ring-2 ring-primary",
                )}
                onClick={() => {
                  onSelect(icon.id)
                }}
              >
                <img src={icon.dataUrl} alt="" className="size-full object-contain" />
              </button>
              {used > 0 && (
                <span
                  className="pointer-events-none absolute right-0.5 bottom-0.5 rounded bg-background/90 px-1 text-[10px] tabular-nums"
                  aria-hidden
                >
                  {used}
                </span>
              )}
              <Button
                type="button"
                variant="secondary"
                size="icon"
                className="absolute -top-1 -right-1 size-5 opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 focus-visible:opacity-100"
                aria-label={`Delete ${icon.name}`}
                title={used > 0 ? `Used by ${plural(used, "key")}; cannot be deleted` : "Delete"}
                disabled={busy || used > 0}
                onClick={() => {
                  onDelete(icon)
                }}
              >
                <Trash2 className="size-3" />
              </Button>
            </li>
          )
        })}
      </ul>
      {icons.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No icons in this profile yet. Imported images are scaled to at most 256 pixels and stored
          in the profile.
        </p>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-label="Icon fit"
          value={fit === "mixed" ? "" : shownFit}
          disabled={selected === undefined}
          onValueChange={(value) => {
            if (value) onFit(value as IconFit)
          }}
        >
          {FITS.map((option) => (
            <ToggleGroupItem
              key={option.value}
              value={option.value}
              title={option.hint}
              className="px-2.5"
            >
              {option.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        {chosen && (
          <div
            className="size-14 overflow-hidden rounded-lg bg-neutral-800"
            role="img"
            aria-label={`${chosen.name} with ${shownFit}`}
          >
            <img
              src={chosen.dataUrl}
              alt=""
              className="size-full"
              style={{ objectFit: FIT_OBJECT[shownFit] }}
            />
          </div>
        )}
      </div>
    </fieldset>
  )
}
