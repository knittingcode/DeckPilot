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
  "open-external": {
    title: "Open applications, files and web pages",
    description:
      "Opens applications, files, folders and web pages with the programs your system chooses for them.",
  },
  "process-execution": {
    title: "Run programs on this computer",
    description:
      "Starts programs such as git, docker or a command you bound, with a time limit. Running your own commands must also be allowed in the module's settings.",
  },
  "process-inspect": {
    title: "See which programs are running",
    description: "Reads the names of running programs, never their command lines or environment.",
  },
  network: {
    title: "Send network requests",
    description:
      "Sends HTTP requests. Requests to this computer or the local network must be allowed separately.",
  },
  notifications: {
    title: "Show desktop notifications",
    description: "Shows notifications on this computer and in the status bar.",
  },
  "variables-read": {
    title: "Read shared variables",
    description:
      "Reads shared and system variables while an action runs. Sensitive values stay hidden everywhere else.",
  },
  credentials: {
    title: "Use stored credentials",
    description:
      "Uses credentials stored on this computer by their name. Secrets are never part of a workflow or an export.",
  },
}
