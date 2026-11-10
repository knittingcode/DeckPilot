import type { Capability } from "./types"

/** What each capability allows, as shown to the user. */
export const CAPABILITIES: Record<Capability, { title: string; description: string }> = {
  "profile-read": {
    title: "Read the active profile",
    description: "Reads the actions, workflows and layouts of the profile active on the device.",
  },
  "project-context": {
    title: "Change the active project",
    description: "Switches the active project of the active profile, for example from a key.",
  },
  "machine-resources": {
    title: "Read this computer's resource bindings",
    description:
      "Reads the paths, URLs and commands bound to project resources on this computer. They are never exported.",
  },
}
