import { describe, expect, it } from "vitest"

import { deviceModel } from "@/test/fixtures"

import { currentPlatform, isSupported, verificationOn, verifiedPlatforms } from "./verification"

describe("verification", () => {
  it("supports a model only where it was physically verified", () => {
    const mk2 = deviceModel()
    expect(isSupported(mk2)).toBe(true)
    expect(verifiedPlatforms(mk2)).toEqual(["macos"])
    expect(verificationOn(mk2, "windows").level).toBe("automatedTests")

    const xl = deviceModel({ id: "elgato.streamdeck.xl", verification: [] })
    expect(isSupported(xl)).toBe(false)
    // A platform without an entry reads as defined.
    expect(verificationOn(xl, "linux").level).toBe("defined")

    // Automated tests on every platform are not physical verification.
    const tested = deviceModel({
      verification: ["macos", "windows", "linux"].map((platform) => ({
        platform: platform as "macos",
        level: "automatedTests" as const,
        evidenceDate: null,
        evidence: "",
      })),
    })
    expect(isSupported(tested)).toBe(false)
  })

  it("detects the platform from the user agent", () => {
    expect(currentPlatform("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe("macos")
    expect(currentPlatform("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe("windows")
    expect(currentPlatform("Mozilla/5.0 (X11; Linux x86_64)")).toBe("linux")
  })
})
