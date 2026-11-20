import { afterEach, describe, expect, it } from "vitest"

import { i18n } from "@/i18n"

import { isAuthenticationRequired } from "./api"
import { errorCode, errorMessage, rawErrorMessage, translateErrorCode } from "./backend-error"
import { credentialFailure } from "./credential-errors"
import { failureDetail, failureMessage } from "./failure"

afterEach(async () => {
  await i18n.changeLanguage("en")
})

const locked = { code: "locked", message: "the configuration is locked" }
const inUse = {
  code: "invalid.layoutUsedElsewhere",
  message:
    "the layout is also used by “Home”; delete it in Layouts, or remove it from this profile",
  params: { profiles: "“Home”" },
}

describe("backend errors", () => {
  it("translates known codes in English, equal to the backend's text", () => {
    expect(errorMessage(locked)).toBe("the configuration is locked")
    expect(errorMessage(inUse)).toBe(inUse.message)
    expect(
      errorMessage({
        code: "agent.notRunning",
        message: "the DeckPilot agent is not running",
      }),
    ).toBe("the DeckPilot agent is not running")
  })

  it("translates known codes in Polish, with params and plurals", async () => {
    await i18n.changeLanguage("pl")
    expect(errorMessage(locked)).toBe("konfiguracja jest zablokowana")
    expect(errorMessage(inUse)).toBe(
      "układ jest używany także przez profile “Home”; usuń go w sekcji Układy albo odłącz go od tego profilu",
    )
    expect(
      errorMessage({
        code: "invalid.passwordIncorrect",
        message: "the password is not correct",
      }),
    ).toBe("hasło jest nieprawidłowe")
    const running = (count: number) =>
      errorMessage({ code: "invalid.workflowsRunning", message: "…", params: { count } })
    expect(running(1)).toMatch(/^w tle działa 1 przepływ pracy;/)
    expect(running(3)).toMatch(/^w tle działają 3 przepływy pracy;/)
    expect(running(5)).toMatch(/^w tle działa 5 przepływów pracy;/)
    expect(
      errorMessage({
        code: "storage.unsupportedSchema",
        message: "database schema version 9 is newer than supported version 7",
        params: { found: 9, supported: 7 },
      }),
    ).toBe("wersja schematu bazy danych 9 jest nowsza niż obsługiwana wersja 7")
  })

  it("falls back to the message for unknown codes and groups, never a raw key", async () => {
    await i18n.changeLanguage("pl")
    expect(errorMessage({ code: "invalid", message: "something internal" })).toBe(
      "something internal",
    )
    expect(errorMessage({ code: "invalid.somethingNew", message: "a new refusal" })).toBe(
      "a new refusal",
    )
    // `notFound` and `storage` are groups of codes, not texts.
    expect(errorMessage({ code: "notFound", message: "thing not found" })).toBe("thing not found")
    expect(errorMessage({ code: "storage", message: "storage failed" })).toBe("storage failed")
    expect(
      errorMessage({ code: "agent.failed", message: "the plugin host crashed", params: {} }),
    ).toBe("the plugin host crashed")
    expect(translateErrorCode("credential")).toBeUndefined()
  })

  it("keeps accepting plain strings and Error objects", async () => {
    await i18n.changeLanguage("pl")
    expect(errorMessage("the configuration is locked")).toBe("the configuration is locked")
    expect(errorMessage(new Error("boom"))).toBe("boom")
    expect(errorMessage(42)).toBe("Nieoczekiwany błąd")
    expect(errorCode("locked")).toBeUndefined()
    expect(rawErrorMessage(locked)).toBe("the configuration is locked")
  })

  it("reaches failure messages and toasts translated", async () => {
    await i18n.changeLanguage("pl")
    expect(failureDetail(locked)).toBe("Konfiguracja jest zablokowana.")
    expect(failureMessage("Nie zapisano układu", locked)).toBe(
      "Nie zapisano układu: Konfiguracja jest zablokowana.",
    )
  })

  it("recognizes a request for authentication by code and by text", () => {
    expect(
      isAuthenticationRequired({
        code: "authenticationRequired",
        message: "authentication required",
      }),
    ).toBe(true)
    expect(isAuthenticationRequired("authentication required")).toBe(true)
    expect(isAuthenticationRequired(new Error("authentication required"))).toBe(true)
    expect(isAuthenticationRequired(locked)).toBe(false)
    // The code decides, not a message that happens to match.
    expect(isAuthenticationRequired({ code: "invalid", message: "authentication required" })).toBe(
      false,
    )
  })

  it("maps credential failures by code, and by text for older builds", async () => {
    await i18n.changeLanguage("pl")
    expect(
      credentialFailure({
        code: "invalid.passwordIncorrect",
        message: "the password is not correct",
      }),
    ).toBe("incorrect")
    expect(credentialFailure({ code: "storage.invalidKey", message: "…" })).toBe("incorrect")
    expect(credentialFailure({ code: "agent.closed", message: "…" })).toBe("agent")
    expect(credentialFailure({ code: "storage.corrupt", message: "…" })).toBe("storage")
    expect(credentialFailure({ code: "invalid.notPasswordProtected", message: "…" })).toBe(
      "notPassword",
    )
    // An agent's own message, matched in English whatever the window's language.
    expect(
      credentialFailure({ code: "agent.failed", message: "database error: disk I/O error" }),
    ).toBe("storage")
    expect(credentialFailure({ code: "locked", message: "the configuration is locked" })).toBe(
      "unknown",
    )
  })
})
