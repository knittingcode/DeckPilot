/**
 * How far a device model has been verified, per platform (see
 * docs/devices.md). The data is static in the model registry; the app only
 * presents it.
 */

import type { DeviceModel, Platform, PlatformVerification, VerificationLevel } from "./types"

export const PLATFORMS: Platform[] = ["macos", "windows", "linux"]

export const PLATFORM_LABELS: Record<Platform, string> = {
  macos: "macOS",
  windows: "Windows",
  linux: "Linux",
}

export const LEVELS: Record<VerificationLevel, { label: string; detail: string }> = {
  unsupported: { label: "Unsupported", detail: "Not supported on this platform." },
  experimental: {
    label: "Experimental",
    detail: "A work-in-progress definition that may be incomplete or wrong.",
  },
  defined: {
    label: "Layout design only",
    detail: "Layouts can be designed and saved. There is no driver for this model.",
  },
  driverImplemented: {
    label: "Driver, not tested",
    detail: "A driver exists for this platform, but it has not been tested.",
  },
  automatedTests: {
    label: "Automated tests only",
    detail: "The driver passes automated tests here, but has not been tested on a physical device.",
  },
  physicallyVerified: {
    label: "Physically verified",
    detail: "Tested on a physical device on this platform.",
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
