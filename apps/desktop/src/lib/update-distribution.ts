// What the update UI shows for a build, from the backend's `UpdateState`
// only (docs/distribution/README.md, section 9): which controls exist, how
// the channel and provider are named, and how update errors read.

import type {
  DistributionChannel,
  PackageFormat,
  ReleaseChannel,
  UpdateProviderKind,
  UpdateState,
} from "@/lib/types"

/** Where a build was installed from, as the user knows it. */
export const CHANNEL_LABELS: Record<DistributionChannel, string> = {
  github: "GitHub Releases",
  homebrew: "Homebrew",
  winget: "WinGet",
  apt: "APT repository",
  flathub: "Flathub",
  mac_app_store: "Mac App Store",
  microsoft_store: "Microsoft Store",
}

/** Who delivers updates, in "Updates are delivered by …". */
const DELIVERED_BY: Record<DistributionChannel, string> = {
  github: "GitHub Releases",
  homebrew: "Homebrew",
  winget: "WinGet",
  apt: "APT",
  flathub: "Flathub",
  mac_app_store: "the Mac App Store",
  microsoft_store: "the Microsoft Store",
}

export const FORMAT_LABELS: Record<PackageFormat, string> = {
  app: ".dmg / .app",
  msi: "MSI",
  nsis: "NSIS installer",
  deb: ".deb",
  rpm: ".rpm",
  appimage: "AppImage",
  flatpak: "Flatpak",
  mas_pkg: "App Store package",
  msix: "MSIX",
}

export const RELEASE_CHANNEL_LABELS: Record<ReleaseChannel, string> = {
  stable: "Stable",
  beta: "Beta (pre-release)",
  alpha: "Alpha (pre-release)",
}

export const PROVIDER_LABELS: Record<UpdateProviderKind, string> = {
  github_release: "DeckPilot installs updates from GitHub Releases",
  external_package_manager: "Your package manager",
  store_managed: "The store",
  none: "Not available in this build",
}

/** Today's text for a build without a key; reasons are matched on the backend's wording. */
const NO_KEY = "no update key is compiled into this build"

/** The "Installed from" line. */
export function installedFrom(state: UpdateState): string {
  if (state.updateProvider === "none" && state.distributionChannel === "github") {
    if (state.notConfiguredReason === NO_KEY)
      return "Development or self-built version (no update key)"
    if (!state.packageFormat) return "Development build"
  }
  const channel = CHANNEL_LABELS[state.distributionChannel]
  return state.distributionChannel === "github" && state.packageFormat
    ? `${channel} (${FORMAT_LABELS[state.packageFormat]})`
    : channel
}

/** Who updates the installation, for the "Updates" line. */
export function providerLabel(state: UpdateState): string {
  if (
    state.updateProvider === "external_package_manager" ||
    state.updateProvider === "store_managed"
  ) {
    const by = state.managedBy ?? state.distributionChannel
    return state.updateProvider === "store_managed"
      ? `Delivered by ${DELIVERED_BY[by]}`
      : `Your package manager (${DELIVERED_BY[by]})`
  }
  return PROVIDER_LABELS[state.updateProvider]
}

/** "Updates are delivered by …", for package-manager and store builds. */
export function deliveredBy(state: UpdateState): string {
  return `Updates are delivered by ${DELIVERED_BY[state.managedBy ?? state.distributionChannel]}`
}

/** The controls a build supports; everything else is not rendered. */
export interface UpdateControls {
  /** "Check for updates" exists (it may still be disabled, see `checkEnabled`). */
  check: boolean
  checkEnabled: boolean
  /** Available version, notes, download, install and progress. */
  selfUpdate: boolean
  /** "When an update is available" (notify, download-and-ask, install-when-idle). */
  mode: boolean
  /** Stable or pre-release. */
  channelChoice: boolean
  /** Automatic checks and their interval. */
  schedule: boolean
  /** The copyable update command of a package manager. */
  command: string | null
  /** Store builds: the store's instruction (no command, no button). */
  managed: string | null
  /** Package managers: "Version X is available; update with your package manager". */
  versionNotice: boolean
}

export function updateControls(state: UpdateState): UpdateControls {
  const none: UpdateControls = {
    check: false,
    checkEnabled: false,
    selfUpdate: false,
    mode: false,
    channelChoice: false,
    schedule: false,
    command: null,
    managed: null,
    versionNotice: false,
  }
  const { capabilities } = state
  switch (state.updateProvider) {
    case "github_release":
      return {
        ...none,
        check: true,
        checkEnabled: state.configured,
        selfUpdate: capabilities.selfUpdate && state.configured,
        mode: capabilities.automaticUpdates,
        channelChoice: capabilities.channelChoice,
        schedule: true,
      }
    case "external_package_manager":
      return {
        ...none,
        // Only with the version notice, which only reads a version.
        check: capabilities.versionCheck,
        checkEnabled: capabilities.versionCheck,
        schedule: capabilities.versionCheck,
        versionNotice: capabilities.versionCheck,
        command: state.updateInstruction,
      }
    case "store_managed":
      return { ...none, managed: state.updateInstruction ?? deliveredBy(state) }
    case "none":
      // Disabled, with the reason as its hint.
      return { ...none, check: true }
  }
}

/** Why a build has no updates, worded for the user. */
export function notConfiguredExplanation(reason: string | null): string {
  if (!reason || reason === NO_KEY) {
    return "Development and self-built versions have no update signing key. Official releases check for updates automatically."
  }
  if (reason === "the package format of this build is unknown") {
    return "DeckPilot cannot tell which package this copy was installed from (for example a development build run from its build folder), so it does not update itself. Install an official release to get updates."
  }
  const mismatch = /^the compiled package format (\S+) differs from the bundled (\S+)$/.exec(reason)
  if (mismatch) {
    const [, compiled, bundled] = mismatch
    return `This build was made as a ${formatName(compiled)} package but is installed as ${formatName(bundled)}, so it does not update itself: an update could replace it with the wrong package. Reinstall DeckPilot from where you got it.`
  }
  const notUpdatable = /^GitHub updates do not install (\S+) packages on this platform$/.exec(
    reason,
  )
  if (notUpdatable) {
    return `GitHub updates cannot install ${formatName(notUpdatable[1])} packages on this platform, so this copy does not update itself. Install DeckPilot from the releases page to get updates.`
  }
  return `Updates are not available in this build: ${reason}.`
}

function formatName(name: string | undefined): string {
  return name && name in FORMAT_LABELS ? FORMAT_LABELS[name as PackageFormat] : (name ?? "another")
}

export interface DescribedUpdateError {
  title: string
  /** What happened and what to do, as a sentence. */
  message: string
  /** The technical detail the backend appended, if any. */
  detail: string | null
}

const STAGES: Record<string, string> = {
  check: "Checking for updates failed",
  download: "The update was not downloaded",
  installation: "The update was not installed",
}

/**
 * An update error of the backend (`updates.rs`: refusals of the update
 * check, updater failures, and the outcome of recovering an interrupted
 * update), split into a title, the explanation and the detail.
 */
export function describeUpdateError(error: string): DescribedUpdateError {
  const failed = /^update (check|download|installation) failed: ([\s\S]*)$/.exec(error)
  const [body, detail] = failed ? splitDetail(failed[2] ?? "") : [error, null]
  let title = failed ? (STAGES[failed[1] ?? ""] ?? "The update failed") : "Update problem"
  if (body.includes("outside DeckPilot's releases")) {
    title = "Update refused: unexpected download address"
  } else if (body.includes("signature")) {
    title = "Update refused: the signature did not verify"
  } else if (body.includes("not a package of the format")) {
    title = "Update refused: different package format"
  } else if (/^the update from .* did not complete/.test(body)) {
    title = "The last update did not complete"
  } else if (/^version .* was installed, but its agent is not running/.test(body)) {
    title = "Updated, but the background agent is not running"
  }
  return { title, message: sentence(body), detail }
}

/** Splits a trailing, balanced " (detail)" off `text`. */
function splitDetail(text: string): [string, string | null] {
  if (!text.endsWith(")")) return [text, null]
  let depth = 0
  for (let i = text.length - 1; i >= 0; i--) {
    if (text[i] === ")") depth++
    else if (text[i] === "(" && --depth === 0) {
      return i > 0 && text[i - 1] === " "
        ? [text.slice(0, i - 1), text.slice(i + 1, -1)]
        : [text, null]
    }
  }
  return [text, null]
}

function sentence(text: string): string {
  const trimmed = text.trim()
  if (!trimmed) return trimmed
  const capital = trimmed.charAt(0).toUpperCase() + trimmed.slice(1)
  return /[.!?]$/.test(capital) ? capital : `${capital}.`
}
