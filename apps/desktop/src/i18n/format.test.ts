import { describe, expect, it } from "vitest"

import {
  collator,
  formatDate,
  formatDateTime,
  formatFileSize,
  formatList,
  formatNumber,
  formatPercent,
  formatRelativeTime,
  formatTime,
  formatter,
} from "./format"

// 18 November 2026, 13:05 local time.
const date = new Date(2026, 10, 18, 13, 5)

describe("formatting per locale", () => {
  it("formats dates the Polish way, not the US way", () => {
    expect(formatDate("pl", date, { dateStyle: "short" })).toBe("18.11.2026")
    expect(formatDate("en", date, { dateStyle: "short" })).toBe("11/18/26")
    expect(formatDate("pl", date)).toBe("18 lis 2026")
    expect(formatTime("pl", date)).toBe("13:05")
    expect(formatTime("en", date)).toMatch(/^1:05\sPM$/)
    expect(formatDateTime("pl", date)).toContain("18 lis 2026")
    expect(formatDateTime("pl", date)).toContain("13:05")
  })

  it("formats numbers, percentages and sizes", () => {
    expect(formatNumber("pl", 1234.5)).toBe("1234,5")
    expect(formatNumber("pl", 12345.5)).toMatch(/^12\s345,5$/)
    expect(formatNumber("en", 12345.5)).toBe("12,345.5")
    expect(formatPercent("pl", 0.425, 1)).toBe("42,5%")
    expect(formatPercent("en", 0.42)).toBe("42%")
    expect(formatFileSize("pl", 1536 * 1024)).toBe("1,5 MB")
    expect(formatFileSize("en", 1536 * 1024)).toBe("1.5 MB")
    expect(formatFileSize("en", 512)).toBe("512 byte")
    expect(formatFileSize("pl", 2048)).toBe("2 kB")
  })

  it("formats relative times and lists", () => {
    const now = date.getTime()
    expect(formatRelativeTime("pl", now - 5 * 60_000, now)).toBe("5 minut temu")
    expect(formatRelativeTime("en", now - 5 * 60_000, now)).toBe("5 minutes ago")
    expect(formatRelativeTime("pl", now + 86_400_000, now)).toBe("jutro")
    expect(formatList("pl", ["Praca", "Dom", "Gry"])).toBe("Praca, Dom i Gry")
    expect(formatList("en", ["Work", "Home", "Games"])).toBe("Work, Home, and Games")
  })

  it("sorts by the language's alphabet", () => {
    const words = ["zebra", "łódź", "lato", "ćma", "cel"]
    expect([...words].sort(collator("pl").compare)).toEqual(["cel", "ćma", "lato", "łódź", "zebra"])
    expect(formatter("pl").compare("a", "ą")).toBeLessThan(0)
  })
})
