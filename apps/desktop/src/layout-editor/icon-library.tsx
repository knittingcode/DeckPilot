// The Icon Library (docs/key-text.md, "Icons and backgrounds"): one shared
// library of three sources — DeckPilot's own icons (built in, offline,
// searchable, in categories), My Icons (images imported into the profile)
// and the icons installed plugins contribute. Choosing a tile puts its id
// on the selected keys (an undo step of the editor's draft, which the live
// preview shows); nothing runs. The tiles of DeckPilot's icons are drawn by
// the keys' renderer, so the key looks as its tile.

import { ImageOff, Search, Trash2, TriangleAlert, Upload } from "lucide-react"
import { useId, useMemo, useRef, useState } from "react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { type TranslationKey, useTranslation } from "@/i18n"
import type { IconFit, IconInfo, VisualLibrary } from "@/lib/types"

import { FIT_OBJECT, MAX_ICONS, type Shared, isBuiltinAsset } from "./appearance"
import { AssetGrid, type AssetTile } from "./asset-grid"
import { type IconSource, iconAvailable, iconTitle, matches, sourceOf } from "./asset-logic"

/** The profile's icon library as the editor uses it (`useIcons`). */
export interface IconState {
  icons: IconInfo[]
  usage: (icon: IconInfo) => number
  busy: boolean
  problem: string | null
  importFile: (file: File) => Promise<IconInfo | null>
  remove: (icon: IconInfo) => void
}

const FITS: { value: IconFit; label: TranslationKey; hint: TranslationKey }[] = [
  { value: "fit", label: "editor:icons.fit", hint: "editor:icons.fitHint" },
  { value: "fill", label: "editor:icons.fill", hint: "editor:icons.fillHint" },
  { value: "crop", label: "editor:icons.crop", hint: "editor:icons.cropHint" },
]

/**
 * The library: source tabs, search, DeckPilot's categories, the grid, and
 * for an image the fit. `onSelect(null)` removes the key's own icon.
 */
export function IconLibrary({
  library,
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
  library: VisualLibrary
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
  const { t } = useTranslation()
  const input = useRef<HTMLInputElement>(null)
  const problemId = useId()
  const current = selected === "mixed" ? undefined : selected
  const [source, setSource] = useState<IconSource>(() => sourceOf(current, icons))
  const [query, setQuery] = useState("")
  const [category, setCategory] = useState<string>("")
  const chosenImage =
    current && !isBuiltinAsset(current)
      ? (icons.find((i) => i.id === current)?.dataUrl ??
        library.plugins.find((a) => a.id === current)?.thumbnail ??
        undefined)
      : undefined
  const shownFit = fit === "mixed" || fit === undefined ? "fit" : fit
  const status = current ? iconAvailable(current, library, icons) : "available"
  const pluginIcons = library.plugins.filter((asset) => asset.kind === "icon")

  const tiles: AssetTile[] = useMemo(() => {
    const none: AssetTile = {
      id: "",
      label: t("editor:icons.none"),
      content: <ImageOff className="size-4 text-muted-foreground" />,
      selected: selected === undefined,
    }
    if (source === "deckpilot") {
      const shown = library.icons.filter(
        (icon) =>
          (!category || icon.categories.includes(category)) &&
          (!query || matches(query, icon.name, ...icon.tags)),
      )
      return [
        ...(query || category ? [] : [none]),
        ...shown.map((icon) => ({
          id: icon.id,
          label: iconTitle(icon.name),
          content: <img src={icon.thumbnail} alt="" className="size-[70%]" />,
          selected: current === icon.id,
        })),
      ]
    }
    if (source === "mine") {
      return [
        ...(query ? [] : [none]),
        ...icons
          .filter((icon) => !query || matches(query, icon.name))
          .map((icon) => {
            const used = usage(icon)
            return {
              id: icon.id,
              label: t("editor:icons.entry", { name: icon.name, count: used }),
              content: <img src={icon.dataUrl} alt="" className="size-full object-contain" />,
              selected: current === icon.id,
              overlay: (
                <>
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
                    aria-label={t("editor:icons.delete", { name: icon.name })}
                    title={
                      used > 0
                        ? t("editor:icons.inUse", { count: used })
                        : t("editor:icons.deleteTitle")
                    }
                    disabled={busy || used > 0}
                    onClick={() => {
                      onDelete(icon)
                    }}
                  >
                    <Trash2 className="size-3" />
                  </Button>
                </>
              ),
            }
          }),
      ]
    }
    return pluginIcons
      .filter((asset) => !query || matches(query, asset.name, asset.pluginName, ...asset.keywords))
      .map((asset) => ({
        id: asset.id,
        label: asset.available
          ? t("editor:assets.pluginEntry", { name: asset.name, plugin: asset.pluginName })
          : t("editor:assets.pluginDisabledEntry", { name: asset.name, plugin: asset.pluginName }),
        content: asset.thumbnail ? (
          <img src={asset.thumbnail} alt="" className="size-full object-contain" />
        ) : (
          <TriangleAlert className="size-4 text-muted-foreground" />
        ),
        selected: current === asset.id,
        disabled: !asset.available,
      }))
  }, [
    source,
    library,
    icons,
    query,
    category,
    current,
    selected,
    usage,
    busy,
    onDelete,
    pluginIcons,
    t,
  ])

  return (
    <div className="grid gap-2" data-slot="icon-library">
      <div className="flex items-center gap-2">
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-label={t("editor:assets.sources")}
          value={source}
          onValueChange={(value) => {
            if (value) setSource(value as IconSource)
          }}
        >
          <ToggleGroupItem value="deckpilot" className="px-2">
            {t("editor:assets.deckpilot")}
          </ToggleGroupItem>
          <ToggleGroupItem value="mine" className="px-2">
            {t("editor:assets.mine")}
          </ToggleGroupItem>
          <ToggleGroupItem value="plugins" className="px-2">
            {t("editor:assets.plugins")}
          </ToggleGroupItem>
        </ToggleGroup>
      </div>
      <div className="relative">
        <Search className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          aria-label={t("editor:assets.search")}
          placeholder={t("editor:assets.search")}
          className="h-8 pl-7"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value)
          }}
        />
      </div>
      {source === "deckpilot" && (
        <select
          aria-label={t("editor:assets.category")}
          className="h-8 rounded-md border border-input bg-background px-2 text-sm"
          value={category}
          onChange={(event) => {
            setCategory(event.target.value)
          }}
        >
          <option value="">{t("editor:assets.allCategories")}</option>
          {library.categories.map((name) => (
            <option key={name} value={name}>
              {t(`editor:assets.categories.${name}` as TranslationKey, { defaultValue: name })}
            </option>
          ))}
        </select>
      )}
      {source === "mine" && (
        <div className="flex flex-wrap items-center gap-2">
          <input
            ref={input}
            type="file"
            accept="image/png,image/jpeg"
            className="sr-only"
            tabIndex={-1}
            aria-label={t("editor:icons.file")}
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
            <Upload /> {t("editor:icons.import")}
          </Button>
          <span className="text-xs text-muted-foreground">
            {t("editor:icons.limits", { used: icons.length, max: MAX_ICONS })}
          </span>
        </div>
      )}
      {problem && (
        <p id={problemId} role="alert" className="text-xs text-destructive">
          {problem}
        </p>
      )}
      {status !== "available" && (
        <p role="status" data-slot="asset-unavailable" className="text-xs text-amber-400">
          {t(status === "disabled" ? "editor:assets.iconDisabled" : "editor:assets.iconMissing")}
        </p>
      )}
      <div className="relative max-h-64 overflow-y-auto pr-0.5" data-slot="asset-scroll">
        {tiles.length > 0 ? (
          <AssetGrid
            label={
              source === "mine"
                ? t("editor:icons.list")
                : source === "plugins"
                  ? t("editor:assets.pluginIcons")
                  : t("editor:assets.deckpilotIcons")
            }
            tiles={tiles}
            onChoose={(id) => {
              onSelect(id === "" ? null : id)
            }}
          />
        ) : (
          <p className="py-4 text-center text-xs text-muted-foreground">
            {source === "plugins" && !query
              ? t("editor:assets.noPluginIcons")
              : source === "mine" && !query
                ? t("editor:icons.empty")
                : t("editor:assets.noMatch")}
          </p>
        )}
      </div>
      {chosenImage && (
        <div className="flex flex-wrap items-center gap-3">
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            aria-label={t("editor:icons.fitLabel")}
            value={fit === "mixed" ? "" : shownFit}
            onValueChange={(value) => {
              if (value) onFit(value as IconFit)
            }}
          >
            {FITS.map((option) => (
              <ToggleGroupItem
                key={option.value}
                value={option.value}
                title={t(option.hint)}
                className="px-2.5"
              >
                {t(option.label)}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          <div
            className="size-12 overflow-hidden rounded-lg bg-device-face"
            role="img"
            aria-label={t("editor:icons.withFit", {
              name: icons.find((i) => i.id === current)?.name ?? current ?? "",
              fit: t(FITS.find((option) => option.value === shownFit)?.label ?? "editor:icons.fit"),
            })}
          >
            <img
              src={chosenImage}
              alt=""
              className="size-full"
              style={{ objectFit: FIT_OBJECT[shownFit] }}
            />
          </div>
        </div>
      )}
    </div>
  )
}
