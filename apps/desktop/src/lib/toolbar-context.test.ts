import { describe, expect, it } from "vitest"

import { modulePageContext } from "@/modules"

import {
  TOOLBAR_CONTEXTS,
  corePageContext,
  showsDevice,
  showsLayout,
  showsProfile,
  toolbarContext,
} from "./toolbar-context"

describe("toolbar context", () => {
  it("reads the four contexts and treats anything else as none", () => {
    for (const context of TOOLBAR_CONTEXTS) expect(toolbarContext(context)).toBe(context)
    for (const bad of [undefined, null, "", "Device", "devices", "device,profile", 1, ["device"]])
      expect(toolbarContext(bad)).toBe("none")
  })

  it("says which selectors each context shows", () => {
    expect(TOOLBAR_CONTEXTS.map((c) => [showsDevice(c), showsProfile(c), showsLayout(c)])).toEqual([
      [false, false, false],
      [true, false, false],
      [false, true, false],
      [true, true, false],
    ])
    // The Device Editor's own context adds the layout to both.
    const editor = "device+profile+layout"
    expect([showsDevice(editor), showsProfile(editor), showsLayout(editor)]).toEqual([
      true,
      true,
      true,
    ])
  })

  it("never lets a plugin view declare the layout selector", () => {
    expect(toolbarContext("device+profile+layout")).toBe("none")
  })

  it("gives the editor device, profile and layout, the profile's pages the profile, the rest none", () => {
    expect(corePageContext("designer")).toBe("device+profile+layout")
    expect(corePageContext("actions")).toBe("profile")
    expect(corePageContext("workflows")).toBe("profile")
    expect(corePageContext("dashboard")).toBe("none")
    expect(corePageContext("layouts")).toBe("none")
    expect(corePageContext("marketplace")).toBe("none")
    expect(corePageContext("projects/projects")).toBeUndefined()
    expect(corePageContext("toString")).toBeUndefined()
  })

  it("lets module pages say theirs, none when they do not", () => {
    expect(modulePageContext("work-session/work-session")).toBe("profile")
    expect(modulePageContext("unknown/page")).toBe("none")
    expect(modulePageContext("nonsense")).toBe("none")
  })
})
