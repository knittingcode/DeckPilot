import { controlLabel } from "./layout-model"
import type { DeviceModel, LayoutStatus, Os, ResourceKind, Usage } from "./types"

const OS_LABELS: Record<Os, string> = {
  linux: "Linux",
  macOs: "macOS",
  windows: "Windows",
  other: "Other",
}

export const RESOURCE_KIND_LABELS: Record<ResourceKind, string> = {
  directory: "Directory",
  application: "Application",
  terminal: "Terminal",
  dockerEnvironment: "Docker environment",
  service: "Local service",
  url: "URL",
  other: "Other",
}

export function osLabel(os: Os): string {
  return OS_LABELS[os]
}

export function hex(value: number): string {
  return `0x${value.toString(16).padStart(4, "0")}`
}

export function formatTimestamp(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString()
}

export function describeLayout(status: LayoutStatus): string {
  switch (status.type) {
    case "noProfile":
      return "No active profile"
    case "unknownModel":
      return "Unsupported device model"
    case "missing":
      return "No layout for this model"
    case "compatible":
      return `Layout “${status.name}”`
    case "invalid":
      return `Layout “${status.name}” has problems`
  }
}

export function describeUsage(usage: Usage): string {
  return usage.type === "workflowStep"
    ? `Workflow “${usage.workflowName}”, step ${usage.position}`
    : `Layout “${usage.layoutName}”, page “${usage.pageName}”, ${controlLabel(usage.control)}`
}

export const SUPPORT: Record<DeviceModel["support"], { label: string; detail: string }> = {
  verified: { label: "Supported", detail: "The driver has been tested on hardware." },
  unverified: {
    label: "Driver not yet hardware-tested",
    detail: "A driver exists, but it has not been verified on a physical device.",
  },
  descriptorOnly: {
    label: "Layout design only",
    detail: "Layouts can be designed and saved. There is no driver for this model yet.",
  },
}
