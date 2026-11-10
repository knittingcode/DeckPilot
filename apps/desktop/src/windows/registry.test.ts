import { describe, expect, it } from "vitest"

import { windowView } from "./registry"

describe("window registry", () => {
  it("knows the application's windows and nothing else", () => {
    for (const id of [
      "security.setup",
      "security.unlock",
      "security.migrate",
      "security.confirm",
      "workflows.execution-summary",
    ]) {
      expect(windowView(id)).not.toBeNull()
    }
    for (const id of ["logs.viewer", "toString", "__proto__", "projects.unknown"]) {
      expect(windowView(id)).toBeNull()
    }
  })
})
