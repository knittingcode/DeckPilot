// What the update UI shows for a build, from the backend's `UpdateState`
// only (docs/distribution/README.md, section 9): which controls exist, how
// the channel and provider are named, and how update errors read.

import { type TranslationKey, i18n } from "@/i18n"
import type {
  DistributionChannel,
  PackageFormat,
  ReleaseChannel,
  UpdateProviderKind,
  UpdateState,
} from "@/lib/types"

/** Where a build was installed from, as the user knows it (keys of `settings.json`). */
export const CHANNEL_LABELS: Record<DistributionChannel, TranslationKey> = {
  github: "settings:updates.distribution.channel.github",
  homebrew: "settings:updates.distribution.channel.homebrew",
  winget: "settings:updates.distribution.channel.winget",
  apt: "settings:updates.distribution.channel.apt",
  flathub: "settings:updates.distribution.channel.flathub",
  mac_app_store: "settings:updates.distribution.channel.macAppStore",
  microsoft_store: "settings:updates.distribution.channel.microsoftStore",
}

/** Who delivers updates, in "Updates are delivered by …". */
const DELIVERED_BY: Record<DistributionChannel, TranslationKey> = {
  github: "settings:updates.distribution.by.github",
  homebrew: "settings:updates.distribution.by.homebrew",
  winget: "settings:updates.distribution.by.winget",
  apt: "settings:updates.distribution.by.apt",
  flathub: "settings:updates.distribution.by.flathub",
  mac_app_store: "settings:updates.distribution.by.macAppStore",
  microsoft_store: "settings:updates.distribution.by.microsoftStore",
}

export const FORMAT_LABELS: Record<PackageFormat, TranslationKey> = {
  app: "settings:updates.distribution.format.app",
  msi: "settings:updates.distribution.format.msi",
  nsis: "settings:updates.distribution.format.nsis",
  deb: "settings:updates.distribution.format.deb",
  rpm: "settings:updates.distribution.format.rpm",
  appimage: "settings:updates.distribution.format.appimage",
  flatpak: "settings:updates.distribution.format.flatpak",
  mas_pkg: "settings:updates.distribution.format.masPkg",
  msix: "settings:updates.distribution.format.msix",
}

export const RELEASE_CHANNEL_LABELS: Record<ReleaseChannel, TranslationKey> = {
  stable: "settings:updates.releaseChannelName.stable",
  beta: "settings:updates.releaseChannelName.beta",
  alpha: "settings:updates.releaseChannelName.alpha",
}

export const PROVIDER_LABELS: Record<UpdateProviderKind, TranslationKey> = {
  github_release: "settings:updates.distribution.provider.githubRelease",
  external_package_manager: "settings:updates.distribution.provider.externalPackageManager",
  store_managed: "settings:updates.distribution.provider.storeManaged",
  none: "settings:updates.distribution.provider.none",
}

/** Today's text for a build without a key; reasons are matched on the backend's wording. */
const NO_KEY = "no update key is compiled into this build"

/** The "Installed from" line, in the window's language. */
export function installedFrom(state: UpdateState): string {
  if (state.updateProvider === "none" && state.distributionChannel === "github") {
    if (state.notConfiguredReason === NO_KEY)
      return i18n.t("settings:updates.distribution.selfBuilt")
    if (!state.packageFormat) return i18n.t("settings:updates.distribution.developmentBuild")
  }
  const channel = i18n.t(CHANNEL_LABELS[state.distributionChannel])
  return state.distributionChannel === "github" && state.packageFormat
    ? i18n.t("settings:updates.distribution.installedWithFormat", {
        channel,
        format: i18n.t(FORMAT_LABELS[state.packageFormat]),
      })
    : channel
}

/** Who updates the installation, for the "Updates" line. */
export function providerLabel(state: UpdateState): string {
  if (
    state.updateProvider === "external_package_manager" ||
    state.updateProvider === "store_managed"
  ) {
    const provider = i18n.t(DELIVERED_BY[state.managedBy ?? state.distributionChannel])
    return state.updateProvider === "store_managed"
      ? i18n.t("settings:updates.distribution.storeDelivers", { provider })
      : i18n.t("settings:updates.distribution.packageManagerOf", { provider })
  }
  return i18n.t(PROVIDER_LABELS[state.updateProvider])
}

/** "Updates are delivered by …", for package-manager and store builds. */
export function deliveredBy(state: UpdateState): string {
  return i18n.t("settings:updates.distribution.deliveredBy", {
    provider: i18n.t(DELIVERED_BY[state.managedBy ?? state.distributionChannel]),
  })
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

/** Why a build has no updates, worded for the user in the window's language. */
export function notConfiguredExplanation(reason: string | null): string {
  if (!reason || reason === NO_KEY) {
    return i18n.t("settings:updates.distribution.explanation.noKey")
  }
  if (reason === "the package format of this build is unknown") {
    return i18n.t("settings:updates.distribution.explanation.unknownFormat")
  }
  const mismatch = /^the compiled package format (\S+) differs from the bundled (\S+)$/.exec(reason)
  if (mismatch) {
    const [, compiled, bundled] = mismatch
    return i18n.t("settings:updates.distribution.explanation.formatMismatch", {
      compiled: formatName(compiled),
      bundled: formatName(bundled),
    })
  }
  const notUpdatable = /^GitHub updates do not install (\S+) packages on this platform$/.exec(
    reason,
  )
  if (notUpdatable) {
    return i18n.t("settings:updates.distribution.explanation.notUpdatable", {
      format: formatName(notUpdatable[1]),
    })
  }
  // The backend's reason is shown as it is (docs/i18n.md, fallback rules).
  return i18n.t("settings:updates.distribution.explanation.other", { reason })
}

function formatName(name: string | undefined): string {
  if (name && name in FORMAT_LABELS) return i18n.t(FORMAT_LABELS[name as PackageFormat])
  return name ?? i18n.t("settings:updates.distribution.format.another")
}

export interface DescribedUpdateError {
  /** In the window's language. */
  title: string
  /** What happened and what to do, as a sentence: the backend's text, as it is. */
  message: string
  /** The technical detail the backend appended, if any. */
  detail: string | null
}

const STAGES: Record<string, TranslationKey> = {
  check: "settings:updates.checkFailed",
  download: "settings:updates.notDownloaded",
  installation: "settings:updates.notInstalled",
}

/**
 * An update error of the backend (`updates.rs`: refusals of the update
 * check, updater failures, and the outcome of recovering an interrupted
 * update), split into a title, the explanation and the detail.
 */
export function describeUpdateError(error: string): DescribedUpdateError {
  const failed = /^update (check|download|installation) failed: ([\s\S]*)$/.exec(error)
  const [body, detail] = failed ? splitDetail(failed[2] ?? "") : [error, null]
  let title: TranslationKey = failed
    ? (STAGES[failed[1] ?? ""] ?? "settings:updates.error.failed")
    : "settings:updates.error.problem"
  if (body.includes("outside DeckPilot's releases")) {
    title = "settings:updates.error.untrustedUrl"
  } else if (body.includes("signature")) {
    title = "settings:updates.error.signature"
  } else if (body.includes("not a package of the format")) {
    title = "settings:updates.error.otherFormat"
  } else if (/^the update from .* did not complete/.test(body)) {
    title = "settings:updates.error.incomplete"
  } else if (/^version .* was installed, but its agent is not running/.test(body)) {
    title = "settings:updates.error.agentNotRunning"
  }
  return { title: i18n.t(title), message: sentence(body), detail }
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
