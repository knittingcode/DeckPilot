import type { TranslationKey } from "@/i18n"

import type { Capability } from "./types"

/** What each capability allows, as shown to the user: keys of `permissions.json`, resolved with `t`. */
export const CAPABILITIES: Record<
  Capability,
  { title: TranslationKey; description: TranslationKey }
> = {
  "profile-read": {
    title: "permissions:capability.profileRead.title",
    description: "permissions:capability.profileRead.description",
  },
  "project-context": {
    title: "permissions:capability.projectContext.title",
    description: "permissions:capability.projectContext.description",
  },
  "machine-resources": {
    title: "permissions:capability.machineResources.title",
    description: "permissions:capability.machineResources.description",
  },
  "open-external": {
    title: "permissions:capability.openExternal.title",
    description: "permissions:capability.openExternal.description",
  },
  "process-execution": {
    title: "permissions:capability.processExecution.title",
    description: "permissions:capability.processExecution.description",
  },
  "process-inspect": {
    title: "permissions:capability.processInspect.title",
    description: "permissions:capability.processInspect.description",
  },
  network: {
    title: "permissions:capability.network.title",
    description: "permissions:capability.network.description",
  },
  notifications: {
    title: "permissions:capability.notifications.title",
    description: "permissions:capability.notifications.description",
  },
  "variables-read": {
    title: "permissions:capability.variablesRead.title",
    description: "permissions:capability.variablesRead.description",
  },
  credentials: {
    title: "permissions:capability.credentials.title",
    description: "permissions:capability.credentials.description",
  },
}
