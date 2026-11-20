// A grid of asset tiles for the Icon and Background Libraries: one tab stop
// (the selected tile, else the first), arrow keys move between tiles by
// row and column, Home and End to the ends; Enter or Space chooses (a
// button's own behaviour). Tiles have the same size everywhere, a tooltip
// and an accessible name, and the chosen one a ring.

import { type KeyboardEvent, type ReactNode, useRef, useState } from "react"

import { cn } from "@/lib/utils"

import { GRID_COLUMNS } from "./asset-logic"

export interface AssetTile {
  id: string
  /** Accessible name and tooltip. */
  label: string
  /** What the tile shows (an image, a swatch…). */
  content: ReactNode
  selected?: boolean
  disabled?: boolean
  /** Extra classes of the tile (its background, e.g. a checkerboard). */
  className?: string
  style?: React.CSSProperties
  /** Drawn over the tile's corner (e.g. a delete button). */
  overlay?: ReactNode
}

export function AssetGrid({
  label,
  tiles,
  columns = GRID_COLUMNS,
  onChoose,
}: {
  label: string
  tiles: AssetTile[]
  columns?: number
  onChoose: (id: string) => void
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([])
  const selectedIndex = tiles.findIndex((tile) => tile.selected)
  const [focus, setFocus] = useState<number | null>(null)
  const current = focus !== null && focus < tiles.length ? focus : Math.max(0, selectedIndex)
  const move = (event: KeyboardEvent, index: number) => {
    const last = tiles.length - 1
    const next = (() => {
      switch (event.key) {
        case "ArrowRight":
          return Math.min(last, index + 1)
        case "ArrowLeft":
          return Math.max(0, index - 1)
        case "ArrowDown":
          return index + columns <= last ? index + columns : index
        case "ArrowUp":
          return index - columns >= 0 ? index - columns : index
        case "Home":
          return 0
        case "End":
          return last
        default:
          return null
      }
    })()
    if (next === null) return
    event.preventDefault()
    event.stopPropagation()
    setFocus(next)
    refs.current[next]?.focus()
  }
  return (
    <div
      role="group"
      aria-label={label}
      data-slot="asset-grid"
      className="grid gap-1"
      style={{ gridTemplateColumns: `repeat(${String(columns)}, minmax(0, 1fr))` }}
    >
      {tiles.map((tile, index) => (
        <div key={tile.id} className="group relative">
          <button
            ref={(element) => {
              refs.current[index] = element
            }}
            type="button"
            data-slot="asset-tile"
            data-asset={tile.id}
            aria-label={tile.label}
            aria-pressed={tile.selected ?? false}
            title={tile.label}
            disabled={tile.disabled}
            tabIndex={index === current ? 0 : -1}
            className={cn(
              "flex aspect-square w-full items-center justify-center overflow-hidden rounded-md border border-white/10 bg-neutral-900 p-1 hover:border-white/40 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-40",
              tile.selected && "ring-2 ring-primary",
              tile.className,
            )}
            style={tile.style}
            onFocus={() => {
              setFocus(index)
            }}
            onKeyDown={(event) => {
              move(event, index)
            }}
            onClick={() => {
              onChoose(tile.id)
            }}
          >
            {tile.content}
          </button>
          {tile.overlay}
        </div>
      ))}
    </div>
  )
}
