import { describe, expect, it } from "vitest"

import rendererList from "../../../../crates/deckpilot-render/assets/icons.txt?raw"

import { ACTION_ICONS } from "./icons"

/** The icons the key renderer draws (crates/deckpilot-render/assets/icons.txt). */
const rendererIcons = rendererList
  .split("\n")
  .map((line) => line.trim())
  .filter((line) => line && !line.startsWith("#"))

describe("action icons", () => {
  it("are the icons the device's keys show, so a key looks as in the app", () => {
    expect(rendererIcons.length).toBeGreaterThan(100)
    expect(Object.keys(ACTION_ICONS).sort()).toEqual([...rendererIcons].sort())
  })
})
