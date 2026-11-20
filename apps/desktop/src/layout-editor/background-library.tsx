// The Background Library (docs/key-text.md, "Icons and backgrounds"): the
// same for every kind of key. Transparent, Black and White first (always
// there, whatever plugins are installed), then solid colours (the shared
// palette and a custom colour), DeckPilot's gradients, images (My Icons)
// and the backgrounds installed plugins contribute. Choosing a tile stores
// the background on the selected keys (an undo step of the draft, which
// the live preview shows); the device's default is a key that stores none.

import { Palette, RotateCcw, TriangleAlert } from "lucide-react"

import { Button } from "@/components/ui/button"
import { type TranslationKey, useTranslation } from "@/i18n"
import type { IconInfo, VisualLibrary } from "@/lib/types"

import { SWATCHES, type Shared, assetBackground } from "./appearance"
import { AssetGrid, type AssetTile } from "./asset-grid"
import { BASIC_BACKGROUNDS, CHECKERBOARD, backgroundStatus } from "./asset-logic"

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="grid gap-1" aria-label={title}>
      <h3 className="text-xs font-medium text-muted-foreground">{title}</h3>
      {children}
    </section>
  )
}

export function BackgroundLibrary({
  library,
  icons,
  value,
  onChoose,
  onCustom,
  onReset,
}: {
  library: VisualLibrary
  /** The profile's images (My Icons), usable as backgrounds. */
  icons: IconInfo[]
  value: Shared<string>
  /** Stores `background` on the selected keys. */
  onChoose: (background: string) => void
  /** Opens the custom colour field. */
  onCustom: () => void
  /** Back to the device's default (stores none). */
  onReset: () => void
}) {
  const { t } = useTranslation()
  const current = value === "mixed" ? undefined : value
  const status = backgroundStatus(current, library, icons)
  const swatch = (colour: string): React.CSSProperties => ({ backgroundColor: colour })
  const basic: AssetTile[] = BASIC_BACKGROUNDS.map((entry) => ({
    id: entry.value,
    label: t(entry.name),
    content: null,
    selected: current === entry.value,
    style: entry.value === "transparent" ? CHECKERBOARD : swatch(entry.value),
  }))
  const palette = SWATCHES.filter(
    (entry) => !BASIC_BACKGROUNDS.some((basic) => basic.value === entry.value),
  )
  const isPalette = palette.some((entry) => entry.value === current)
  const custom =
    current && !current.startsWith("asset:") && current !== "transparent" && !isPalette
      ? current
      : undefined
  const solids: AssetTile[] = [
    ...palette.map((entry) => ({
      id: entry.value,
      label: t("editor:color.swatchTitle", { name: t(entry.name), value: entry.value }),
      content: null,
      selected: current === entry.value,
      style: swatch(entry.value),
    })),
    ...(custom && !BASIC_BACKGROUNDS.some((b) => b.value === custom)
      ? [
          {
            id: custom,
            label: t("editor:assets.customColour", { value: custom }),
            content: null,
            selected: true,
            style: swatch(custom),
          },
        ]
      : []),
    {
      id: "custom",
      label: t("editor:assets.chooseCustom"),
      content: <Palette className="size-4 text-muted-foreground" />,
    },
  ]
  const gradients: AssetTile[] = library.gradients.map((gradient) => {
    const background = assetBackground(gradient.id)
    const short = gradient.name.replace(/^gradient-/, "")
    return {
      id: background,
      label: t(`editor:assets.gradients.${short}` as TranslationKey, { defaultValue: short }),
      content: <img src={gradient.thumbnail} alt="" className="size-full rounded-sm" />,
      selected: current === background,
      className: "p-0",
    }
  })
  const images: AssetTile[] = icons.map((icon) => {
    const background = assetBackground(icon.id)
    return {
      id: background,
      label: icon.name,
      content: <img src={icon.dataUrl} alt="" className="size-full object-cover" />,
      selected: current === background,
      className: "p-0",
    }
  })
  const plugins: AssetTile[] = library.plugins
    .filter((asset) => asset.kind !== "icon")
    .map((asset) => {
      const background = assetBackground(asset.id)
      return {
        id: background,
        label: asset.available
          ? t("editor:assets.pluginEntry", { name: asset.name, plugin: asset.pluginName })
          : t("editor:assets.pluginDisabledEntry", { name: asset.name, plugin: asset.pluginName }),
        content: asset.thumbnail ? (
          <img src={asset.thumbnail} alt="" className="size-full object-cover" />
        ) : (
          <TriangleAlert className="size-4 text-muted-foreground" />
        ),
        selected: current === background,
        disabled: !asset.available,
        className: "p-0",
      }
    })
  const choose = (id: string) => {
    if (id === "custom") onCustom()
    else onChoose(id)
  }
  return (
    <div className="grid gap-2.5" data-slot="background-library">
      {status !== "available" && (
        <p role="status" data-slot="asset-unavailable" className="text-xs text-amber-400">
          {t(
            status === "disabled"
              ? "editor:assets.backgroundDisabled"
              : "editor:assets.backgroundMissing",
          )}
        </p>
      )}
      <div
        className="relative grid max-h-80 gap-2.5 overflow-y-auto pr-0.5"
        data-slot="asset-scroll"
      >
        <Section title={t("editor:assets.basic")}>
          <AssetGrid label={t("editor:assets.basic")} tiles={basic} onChoose={choose} />
        </Section>
        <Section title={t("editor:assets.solid")}>
          <AssetGrid label={t("editor:assets.solid")} tiles={solids} onChoose={choose} />
        </Section>
        {gradients.length > 0 && (
          <Section title={t("editor:assets.gradientsTitle")}>
            <AssetGrid
              label={t("editor:assets.gradientsTitle")}
              tiles={gradients}
              onChoose={choose}
            />
          </Section>
        )}
        {images.length > 0 && (
          <Section title={t("editor:assets.images")}>
            <AssetGrid label={t("editor:assets.images")} tiles={images} onChoose={choose} />
          </Section>
        )}
        <Section title={t("editor:assets.pluginBackgrounds")}>
          {plugins.length > 0 ? (
            <AssetGrid
              label={t("editor:assets.pluginBackgrounds")}
              tiles={plugins}
              onChoose={choose}
            />
          ) : (
            <p className="text-xs text-muted-foreground">
              {t("editor:assets.noPluginBackgrounds")}
            </p>
          )}
        </Section>
      </div>
      {value !== undefined && (
        <Button type="button" variant="ghost" size="sm" className="w-fit" onClick={onReset}>
          <RotateCcw /> {t("editor:keyMenu.resetBackground")}
        </Button>
      )}
    </div>
  )
}
