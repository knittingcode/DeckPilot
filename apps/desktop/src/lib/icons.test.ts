import { describe, expect, it } from "vitest"

import rendererList from "../../../../crates/deckpilot-render/assets/icons.txt?raw"

import { ACTION_ICONS } from "./icons"

/**
 * The action icons of the key renderer (crates/deckpilot-render/assets/icons.txt,
 * before the Icon Library's icons, which are drawn from the same font but
 * offered in the Icon Library, not as action icons).
 */
const actionBlock = rendererList.split("# The icon library")[0] ?? ""
const rendererIcons = actionBlock
  .split("\n")
  .map((line) => line.trim())
  .filter((line) => line && !line.startsWith("#"))

describe("action icons", () => {
  it("are the icons the device's keys show, so a key looks as in the app", () => {
    expect(rendererIcons.length).toBeGreaterThan(100)
    expect(Object.keys(ACTION_ICONS).sort()).toEqual([...rendererIcons].sort())
  })
})
