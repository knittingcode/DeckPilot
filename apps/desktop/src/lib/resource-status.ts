// Names and marks of resource bindings and their check results, shared by
// Settings › Resources and the Projects screen.

import { CircleAlert, CircleCheck, CircleDashed } from "lucide-react"

import type { TranslationKey } from "@/i18n"
import type { ResourceKind, ResourceStatus } from "@/lib/types"

export const STATUS: Record<
  ResourceStatus,
  { label: TranslationKey; icon: typeof CircleCheck; className: string }
> = {
  ok: { label: "workspace:projects.status.ok", icon: CircleCheck, className: "text-emerald-600" },
  missing: {
    label: "workspace:projects.status.missing",
    icon: CircleAlert,
    className: "text-destructive",
  },
  unbound: {
    label: "workspace:projects.status.unbound",
    icon: CircleAlert,
    className: "text-amber-600",
  },
  unchecked: {
    label: "workspace:projects.status.unchecked",
    icon: CircleDashed,
    className: "text-muted-foreground",
  },
}

export const KIND_LABELS: Record<ResourceKind, TranslationKey> = {
  directory: "workspace:projects.kind.directory",
  application: "workspace:projects.kind.application",
  terminal: "workspace:projects.kind.terminal",
  dockerEnvironment: "workspace:projects.kind.dockerEnvironment",
  service: "workspace:projects.kind.service",
  url: "workspace:projects.kind.url",
  other: "workspace:projects.kind.other",
}
