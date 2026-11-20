// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest"

import { currentLanguage, i18n } from "./index"
import { effectiveLanguage, initLanguage, setLanguagePreference } from "./language"
import {
  DEFAULT_LANGUAGE_PREFERENCE,
  LANGUAGE_STORAGE_KEY,
  readLanguagePreference,
  writeLanguagePreference,
} from "./preference"

function memoryStorage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial))
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value)
    },
    removeItem: (key: string) => {
      values.delete(key)
    },
    values,
  }
}

afterEach(async () => {
  localStorage.clear()
  vi.restoreAllMocks()
  await i18n.changeLanguage("en")
})

describe("language preference", () => {
  it("defaults to the system language", () => {
    expect(DEFAULT_LANGUAGE_PREFERENCE).toBe("system")
    expect(readLanguagePreference(memoryStorage())).toBe("system")
    expect(readLanguagePreference(null)).toBe("system")
  })

  it("keeps an explicit choice and ignores unknown values", () => {
    const storage = memoryStorage()
    writeLanguagePreference("pl", storage)
    expect(storage.values.get(LANGUAGE_STORAGE_KEY)).toBe("pl")
    expect(readLanguagePreference(storage)).toBe("pl")
    expect(readLanguagePreference(memoryStorage({ [LANGUAGE_STORAGE_KEY]: "de" }))).toBe("system")
  })

  it("stores nothing for the system default", () => {
    const storage = memoryStorage({ [LANGUAGE_STORAGE_KEY]: "en" })
    writeLanguagePreference("system", storage)
    expect(storage.values.has(LANGUAGE_STORAGE_KEY)).toBe(false)
  })

  it("survives storage that throws", () => {
    const broken = {
      getItem: () => {
        throw new Error("denied")
      },
      setItem: () => {
        throw new Error("denied")
      },
      removeItem: () => {
        throw new Error("denied")
      },
    }
    expect(readLanguagePreference(broken)).toBe("system")
    expect(() => {
      writeLanguagePreference("pl", broken)
    }).not.toThrow()
  })

  it("resolves the system preference from the system's languages", async () => {
    expect(await effectiveLanguage("system", () => Promise.resolve(["de-DE", "pl-PL"]))).toBe("pl")
    expect(await effectiveLanguage("system", () => Promise.resolve(["de-DE"]))).toBe("en")
    expect(await effectiveLanguage("en", () => Promise.resolve(["pl-PL"]))).toBe("en")
  })

  it("persists the choice across sessions and shows it right away", async () => {
    await setLanguagePreference("pl")
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBe("pl")
    expect(currentLanguage()).toBe("pl")
    expect(document.documentElement.lang).toBe("pl")

    // The next start reads it back before the first render.
    await i18n.changeLanguage("en")
    await initLanguage()
    expect(currentLanguage()).toBe("pl")

    await setLanguagePreference("system")
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBeNull()
  })

  it("follows the browser's languages with the system preference", async () => {
    vi.spyOn(navigator, "languages", "get").mockReturnValue(["pl-PL", "en-US"])
    await initLanguage()
    expect(currentLanguage()).toBe("pl")
    vi.spyOn(navigator, "languages", "get").mockReturnValue(["fr-FR"])
    await initLanguage()
    expect(currentLanguage()).toBe("en")
  })
})
