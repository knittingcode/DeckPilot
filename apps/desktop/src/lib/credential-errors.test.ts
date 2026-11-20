import { describe, expect, it } from "vitest"

import { credentialFailure, credentialFailureMessage } from "./credential-errors"

describe("credential failures", () => {
  it.each([
    ["the password is not correct", "incorrect"],
    ["the database could not be opened with the provided key", "incorrect"],
    ["this configuration does not use a password; it opens with the system key", "notPassword"],
    ["the DeckPilot agent is not running", "agent"],
    ["connection to the agent was closed", "agent"],
    ["communication with the agent failed: Broken pipe (os error 32)", "agent"],
    ["unexpected message from the agent: hello", "agent"],
    [
      "the running agent (0.8.1) is too old for `unlock` (it needs protocol 11, the agent speaks 10); restart the agent",
      "agent",
    ],
    ["database error: file is not a database", "storage"],
    ["the database failed its integrity check: page 3", "storage"],
    ["database schema version 12 is newer than supported version 11", "storage"],
    ["file system error: Permission denied", "storage"],
    ["something else", "unknown"],
  ] as const)("%s → %s", (error, kind) => {
    expect(credentialFailure(error)).toBe(kind)
  })

  it("never returns the backend text", () => {
    const raw = "database error: /Users/someone/Library/deckpilot.db is locked"
    expect(credentialFailureMessage(raw)).not.toContain("/Users")
    expect(credentialFailureMessage(new Error("x"))).toBe(
      "DeckPilot couldn’t be unlocked. Try again.",
    )
  })
})
