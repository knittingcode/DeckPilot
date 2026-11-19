import { describe, expect, it } from "vitest"

import { DEVICE_MODELS } from "@/test/fixtures"

import { KEY_SIZE, deviceGeometry, deviceSize, gridWidth, keySize } from "./device-geometry"

describe("device geometry", () => {
  it("comes from the model's capabilities", () => {
    expect(deviceGeometry(DEVICE_MODELS.mk2)).toEqual({ columns: 5, rows: 3, zones: 0, dials: 0 })
    expect(deviceGeometry(DEVICE_MODELS.mini)).toEqual({ columns: 3, rows: 2, zones: 0, dials: 0 })
    expect(deviceGeometry(DEVICE_MODELS.xl)).toEqual({ columns: 8, rows: 4, zones: 0, dials: 0 })
    expect(deviceGeometry(DEVICE_MODELS.plus)).toEqual({ columns: 4, rows: 2, zones: 4, dials: 4 })
  })

  it("is wider than high as the hardware is, and taller with a strip and dials", () => {
    const mk2 = deviceSize(deviceGeometry(DEVICE_MODELS.mk2))
    const plus = deviceSize(deviceGeometry(DEVICE_MODELS.plus))
    expect(mk2.width).toBeGreaterThan(mk2.height)
    // Four keys across, but two rows of keys, the strip and the dials.
    expect(plus.width).toBeLessThan(mk2.width)
    expect(plus.height).toBeGreaterThan(mk2.height)
    expect(gridWidth(deviceGeometry(DEVICE_MODELS.xl))).toBeCloseTo(8 + 7 * 0.2)
  })

  it("scales the keys with the space, within limits", () => {
    const mk2 = deviceGeometry(DEVICE_MODELS.mk2)
    const sizes = [240, 480, 640, 800, 1200].map((width) => keySize({ width }, mk2))
    // Grows with the width, never past the maximum or under the minimum.
    expect(sizes).toEqual([...sizes].sort((a, b) => a - b))
    expect(sizes[0]).toBe(KEY_SIZE.min)
    expect(sizes.at(-1)).toBe(KEY_SIZE.max)
    expect(keySize({ width: 480 }, mk2)).toBe(Math.floor(480 / deviceSize(mk2).width))
    // The device at that size fits the width.
    for (const width of [480, 640]) {
      expect(keySize({ width }, mk2) * deviceSize(mk2).width).toBeLessThanOrEqual(width)
    }
  })

  it("fits more columns into the same width with smaller keys", () => {
    const width = 720
    const xl = keySize({ width }, deviceGeometry(DEVICE_MODELS.xl))
    const mk2 = keySize({ width }, deviceGeometry(DEVICE_MODELS.mk2))
    const mini = keySize({ width }, deviceGeometry(DEVICE_MODELS.mini))
    expect(xl).toBeLessThan(mk2)
    expect(mk2).toBeLessThanOrEqual(mini)
  })

  it("keeps to the height it is given", () => {
    const plus = deviceGeometry(DEVICE_MODELS.plus)
    const wide = keySize({ width: 1000 }, plus)
    const short = keySize({ width: 1000, height: 300 }, plus)
    expect(short).toBeLessThan(wide)
    expect(short * deviceSize(plus).height).toBeLessThanOrEqual(300)
    // No usable space: the minimum, and the container scrolls.
    expect(keySize({ width: 0 }, plus)).toBe(KEY_SIZE.min)
    expect(keySize({ width: Number.NaN }, plus)).toBe(KEY_SIZE.min)
  })
})

/**
 * The app's stylesheet as written. Vitest does not process CSS, so it is
 * read from disk (Node's fs, which the app's types do not include).
 */
async function stylesheet(): Promise<string> {
  const name = "node:fs"
  const fs = (await import(/* @vite-ignore */ name)) as {
    readFileSync: (path: URL, encoding: "utf8") => string
  }
  return fs.readFileSync(new URL("../index.css", import.meta.url), "utf8")
}

describe("device tokens", () => {
  const block = (css: string, selector: string) => {
    const start = css.indexOf(`${selector} {`)
    return css.slice(start, css.indexOf("\n}", start))
  }
  const tokens = [
    "device-body",
    "device-body-border",
    "device-strip",
    "device-key",
    "device-key-border",
    "device-key-hover",
    "device-key-foreground",
    "device-key-muted",
    "device-face",
    "device-face-folder",
    "device-selected",
    "device-focus",
    "device-drop",
  ]

  it("are defined for the light and the dark theme and usable as colours", async () => {
    const css = await stylesheet()
    for (const token of tokens) {
      expect(block(css, ":root"), `--${token} in :root`).toMatch(new RegExp(`--${token}: `))
      expect(block(css, ".dark"), `--${token} in .dark`).toMatch(new RegExp(`--${token}: `))
      expect(block(css, "@theme inline"), `--color-${token}`).toContain(
        `--color-${token}: var(--${token});`,
      )
    }
  })
})
