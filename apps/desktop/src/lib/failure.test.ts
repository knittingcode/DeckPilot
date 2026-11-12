import { describe, expect, it, vi } from "vitest"

import { failureDetail, failureMessage, sentence, toastFailure } from "./failure"

const toast = vi.hoisted(() => ({ error: vi.fn() }))
vi.mock("sonner", () => ({ toast }))

describe("failure messages", () => {
  it("turns backend messages into sentences", () => {
    expect(sentence("profile not found")).toBe("Profile not found.")
    expect(sentence("  Done!  ")).toBe("Done!")
    expect(sentence("")).toBe("")
  })

  it("says what failed, why and what to do", () => {
    expect(failureDetail("no device is connected")).toBe("No device is connected.")
    expect(failureDetail(new Error("timed out"), "try again")).toBe("Timed out. Try again.")
    expect(failureDetail({ odd: true })).toBe("Unexpected error.")
    expect(failureMessage("The brightness was not changed", "no device")).toBe(
      "The brightness was not changed: No device.",
    )
  })

  it("shows what failed as the toast title", () => {
    toastFailure("The layout was not deleted", "it is in use", "Activate another layout first")
    expect(toast.error).toHaveBeenCalledWith("The layout was not deleted", {
      description: "It is in use. Activate another layout first.",
    })
  })
})
