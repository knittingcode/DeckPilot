import { type TranslationKey, currentLanguage, i18n } from "@/i18n"

import { controlLabel } from "./layout-model"
import type { LayoutStatus, Os, ResourceKind, Usage } from "./types"

/** The operating systems' names, keys of `common.json`. */
export const OS_LABELS: Record<Os, TranslationKey> = {
  linux: "common:os.linux",
  macOs: "common:os.macOs",
  windows: "common:os.windows",
  other: "common:os.other",
}

/** Keys of `workspace.json`, resolved with `t` where they are shown. */
export const RESOURCE_KIND_LABELS: Record<ResourceKind, TranslationKey> = {
  directory: "workspace:projects.kind.directory",
  application: "workspace:projects.kind.application",
  terminal: "workspace:projects.kind.terminal",
  dockerEnvironment: "workspace:projects.kind.dockerEnvironment",
  service: "workspace:projects.kind.service",
  url: "workspace:projects.kind.url",
  other: "workspace:projects.kind.other",
}

/** The name of `os` in the window's language. */
export function osLabel(os: Os): string {
  return i18n.t(OS_LABELS[os])
}

export function hex(value: number): string {
  return `0x${value.toString(16).padStart(4, "0")}`
}

/** Date and time in the window's language (`src/i18n`), e.g. "18.11.2026, 13:30:00" in Polish. */
export function formatTimestamp(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString(currentLanguage())
}

/** Date and time without seconds, e.g. "Nov 18, 2026, 1:30 PM": for table columns. */
export function formatShortTimestamp(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString(currentLanguage(), {
    dateStyle: "medium",
    timeStyle: "short",
  })
}

/** The device's layout state, in the window's language. */
export function describeLayout(status: LayoutStatus): string {
  switch (status.type) {
    case "noProfile":
      return i18n.t("devices:layoutStatus.noProfile")
    case "unknownModel":
      return i18n.t("devices:layoutStatus.unknownModel")
    case "missing":
      return i18n.t("devices:layoutStatus.missing")
    case "compatible":
      return i18n.t("devices:layoutStatus.compatible", { name: status.name })
    case "invalid":
      return i18n.t("devices:layoutStatus.invalid", { name: status.name })
  }
}

/** Where an action is used, in the window's language. */
export function describeUsage(usage: Usage): string {
  return usage.type === "workflowStep"
    ? i18n.t("settings:usage.workflowStep", {
        workflow: usage.workflowName,
        step: usage.position,
      })
    : i18n.t("settings:usage.layoutControl", {
        layout: usage.layoutName,
        page: usage.pageName,
        control: controlLabel(usage.control),
      })
}
