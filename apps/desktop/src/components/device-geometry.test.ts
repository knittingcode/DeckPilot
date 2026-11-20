import { describe, expect, it } from "vitest"

import { DEVICE_MODELS } from "@/test/fixtures"

import {
  KEY_SIZE,
  deviceGeometry,
  deviceSize,
  gridWidth,
  keyBorderPixels,
  keySize,
} from "./device-geometry"

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

  it("gives every control one border, in whole pixels and never under 2", () => {
    expect([24, 40, 72, 88, 102, 104].map(keyBorderPixels)).toEqual([2, 2, 4, 5, 6, 6])
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

describe("workspace tokens", () => {
  const block = (css: string, selector: string) => {
    const start = css.indexOf(`${selector} {`)
    return css.slice(start, css.indexOf("\n}", start))
  }
  const tokens = [
    "workspace",
    "workspace-foreground",
    "workspace-muted",
    "workspace-border",
    "workspace-input",
    "workspace-selection",
    "workspace-focus",
  ]

  it("are defined once for both themes and usable as colours", async () => {
    const css = await stylesheet()
    for (const token of tokens) {
      expect(block(css, ":root"), `--${token} in :root`).toMatch(new RegExp(`--${token}: `))
      expect(block(css, "@theme inline"), `--color-${token}`).toContain(
        `--color-${token}: var(--${token});`,
      )
    }
  })

  it("are the app's own dark surface, with a blue selection in the workspace", async () => {
    const css = await stylesheet()
    const root = block(css, ":root")
    // The app is permanently dark: the workspace is its base surface.
    for (const [token, value] of [
      ["workspace", "background"],
      ["workspace-foreground", "foreground"],
      ["workspace-muted", "muted-foreground"],
      ["workspace-border", "border"],
      ["workspace-input", "input"],
    ])
      expect(root).toContain(`--${token}: var(--${value});`)
    const scope = block(css, ".workspace-scope")
    expect(scope).toContain("--device-selected: var(--workspace-selection);")
    expect(scope).toContain("--device-focus: var(--workspace-focus);")
  })

  it("form one dark palette: no light theme and no theme blocks", async () => {
    const css = await stylesheet()
    expect(css).not.toMatch(/^\.dark \{/m)
    expect(css).not.toContain("prefers-color-scheme")
    expect(block(css, ":root")).toContain("--background: #111111;")
  })
})

describe("device tokens", () => {
  const block = (css: string, selector: string) => {
    const start = css.indexOf(`${selector} {`)
    return css.slice(start, css.indexOf("\n}", start))
  }
  const tokens = [
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

  it("are defined once and usable as colours; no device enclosure is drawn", async () => {
    const css = await stylesheet()
    expect(css).not.toContain("--device-body")
    for (const token of tokens) {
      expect(block(css, ":root"), `--${token} in :root`).toMatch(new RegExp(`--${token}: `))
      expect(block(css, "@theme inline"), `--color-${token}`).toContain(
        `--color-${token}: var(--${token});`,
      )
    }
  })
})

describe("inspector curtain", () => {
  it("is a masked progressive blur, hidden until scrolled, with a tint fallback", async () => {
    const css = await stylesheet()
    const rule = css.slice(css.indexOf(".inspector-curtain {"))
    expect(rule).toMatch(/pointer-events: none/)
    expect(rule).toMatch(/opacity: 0/)
    expect(css).toMatch(/\[data-scrolled\] > \.inspector-curtain \{\s*opacity: 1/)
    // Strongest at the divider, fading out: three masked blur layers.
    for (const blur of ["12px", "6px", "2px"]) {
      expect(css).toContain(`backdrop-filter: blur(${blur})`)
    }
    expect(css).toMatch(/mask-image: linear-gradient\(to bottom, #000 0%/)
    // Without backdrop blur: the tint alone.
    expect(css).toMatch(/@supports not \(\(backdrop-filter/)
    expect(css).toMatch(/prefers-reduced-motion[\s\S]*\.inspector-curtain \{\s*transition: none/)
  })
})
