/**
 * How far a device model has been verified, per platform (see
 * docs/devices.md). The data is static in the model registry; the app only
 * presents it.
 */

import type { TranslationKey } from "@/i18n"

import type { DeviceModel, Platform, PlatformVerification, VerificationLevel } from "./types"

export const PLATFORMS: Platform[] = ["macos", "windows", "linux"]

/** Product names, the same in every language. */
export const PLATFORM_LABELS: Record<Platform, string> = {
  macos: "macOS",
  windows: "Windows",
  linux: "Linux",
}

/** Keys of `devices.json` (`verification.level.*`), resolved with `t` where they are shown. */
export const LEVELS: Record<VerificationLevel, { label: TranslationKey; detail: TranslationKey }> =
  {
    unsupported: {
      label: "devices:verification.level.unsupported.label",
      detail: "devices:verification.level.unsupported.detail",
    },
    experimental: {
      label: "devices:verification.level.experimental.label",
      detail: "devices:verification.level.experimental.detail",
    },
    defined: {
      label: "devices:verification.level.defined.label",
      detail: "devices:verification.level.defined.detail",
    },
    driverImplemented: {
      label: "devices:verification.level.driverImplemented.label",
      detail: "devices:verification.level.driverImplemented.detail",
    },
    automatedTests: {
      label: "devices:verification.level.automatedTests.label",
      detail: "devices:verification.level.automatedTests.detail",
    },
    physicallyVerified: {
      label: "devices:verification.level.physicallyVerified.label",
      detail: "devices:verification.level.physicallyVerified.detail",
    },
  }

/** The entry for `platform`; a platform without one is `defined`. */
export function verificationOn(model: DeviceModel, platform: Platform): PlatformVerification {
  return (
    model.verification.find((entry) => entry.platform === platform) ?? {
      platform,
      level: "defined",
      evidenceDate: null,
      evidence: "",
    }
  )
}

/** Platforms on which the model was tested on a physical device. */
export function verifiedPlatforms(model: DeviceModel): Platform[] {
  return PLATFORMS.filter(
    (platform) => verificationOn(model, platform).level === "physicallyVerified",
  )
}

/** Listed as supported: physically verified on at least one platform. */
export function isSupported(model: DeviceModel): boolean {
  return verifiedPlatforms(model).length > 0
}

/** The platform the app runs on, from the user agent. */
export function currentPlatform(
  userAgent: string = typeof navigator === "undefined" ? "" : navigator.userAgent,
): Platform {
  if (/Mac/i.test(userAgent)) return "macos"
  if (/Win/i.test(userAgent)) return "windows"
  return "linux"
}
