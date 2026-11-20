// "Test action" of the device preview's context menu: the agent presses a
// control of the connected device as the user would
// (docs/editor-navigation-and-simulation.md, section 2). The editor never
// runs anything itself, and a double-click never simulates.

import { useContext, useRef } from "react"
import { toast } from "sonner"

import { DialogsContext } from "@/components/dialogs/use-dialogs"
import { i18n } from "@/i18n"
import { api, errorMessage } from "@/lib/api"
import { controlLabel } from "@/lib/layout-model"
import { shownLayout } from "@/lib/layout-navigation"
import type {
  AgentConnection,
  ControlId,
  DeviceInfo,
  PressKind,
  SimulatePressResult,
  SimulationRefusal,
} from "@/lib/types"

/** The IPC protocol that brought `simulatePress`. */
export const SIMULATION_PROTOCOL = 14

export interface SimulationTarget {
  profileId: string
  layoutId: string
  pageId: string
  device: DeviceInfo | null
  agent: AgentConnection | null
  /** The profile the agent runs (the device's). */
  activeProfileId: string | null
}

/**
 * Why the device cannot be pressed for the editor's page now, in words;
 * `null` when it can. The agent checks all of it again: this only keeps
 * the menu honest.
 */
export function simulationUnavailable(target: SimulationTarget): string | null {
  const { agent, device } = target
  const t = i18n.t.bind(i18n)
  if (!agent?.running) return t("editor:preview.why.agentNotRunning")
  const protocol = agent.protocol ?? agent.status?.protocol
  const capable =
    (agent.capabilities as string[] | undefined)?.includes("simulateInput") === true ||
    (protocol !== undefined && protocol >= SIMULATION_PROTOCOL)
  if (!capable && protocol !== undefined) return t("editor:preview.why.agentTooOld")
  if (!device || device.state !== "connected") return t("editor:preview.why.disconnected")
  if (target.activeProfileId !== target.profileId) return t("editor:preview.why.otherProfile")
  const shown = shownLayout(device)
  if (shown !== target.layoutId) return t("editor:preview.why.otherLayout")
  // `page` is the page's name; agents of protocol 16 also say its id.
  if (device.pageId !== undefined && device.pageId !== target.pageId)
    return t("editor:preview.why.otherPage")
  return null
}

/** A refusal in words, in the window's language. */
export function refusalMessage(code: SimulationRefusal, message: string): string {
  const t = i18n.t.bind(i18n)
  if (/commands-disabled|allowCommands|allow_changes|permission/i.test(message))
    return t("editor:preview.refused.settings")
  switch (code) {
    case "locked":
      return t("editor:preview.refused.locked")
    case "noActiveProfile":
    case "wrongProfile":
      return t("editor:preview.refused.wrongProfile")
    case "wrongLayout":
      return t("editor:preview.refused.wrongLayout")
    case "wrongPage":
      return t("editor:preview.refused.wrongPage")
    case "deviceNotConnected":
      return t("editor:preview.refused.disconnected")
    case "unknownControl":
    case "incompatibleControl":
      return t("editor:preview.refused.control")
    case "unsupported":
      return t("editor:preview.refused.unsupported")
    case "agentNotRunning":
      return t("editor:preview.refused.agentNotRunning")
    case "failed":
    case "other":
      return t("editor:preview.refused.failed")
  }
}

const RAN = {
  single: "editor:preview.ran.single",
  double: "editor:preview.ran.double",
  long: "editor:preview.ran.long",
} as const

/**
 * Simulated presses for the editor's page. `dirty`: the draft differs from
 * what is saved, and the device runs the saved layout — asked about first
 * (save, test the saved version, or cancel). `save` resolves whether it
 * saved. `live`: the device shows and runs the draft (the live preview),
 * so the press tests what is on screen without asking.
 */
export function useDeviceSimulation(
  target: SimulationTarget,
  dirty: boolean,
  save: () => Promise<boolean>,
  live = false,
) {
  const dialogs = useContext(DialogsContext)
  const busy = useRef(false)

  const send = async (control: ControlId, kind: PressKind, confirmed: boolean) => {
    const { device, profileId, layoutId, pageId } = target
    if (!device) return
    let result: SimulatePressResult
    try {
      result = await api.simulatePress(
        device.id,
        profileId,
        layoutId,
        pageId,
        control,
        kind,
        confirmed,
      )
    } catch (e) {
      toast.error(i18n.t("editor:preview.failedTitle", { control: controlLabel(control) }), {
        description: errorMessage(e),
      })
      return
    }
    const name = controlLabel(control)
    if (result.status === "ran") {
      toast.success(i18n.t(RAN[kind], { control: name }), {
        description: i18n.t("editor:preview.ranDetail"),
      })
      return
    }
    if (result.status === "refused") {
      toast.error(i18n.t("editor:preview.refusedTitle", { control: name }), {
        description: refusalMessage(result.code, result.message),
      })
      return
    }
    if (confirmed) return
    // Something outside DeckPilot may change: asked, and sent again.
    const description = (
      <div className="grid gap-2">
        <p>{result.reason}</p>
        {result.actionNames.length > 0 && (
          <ul className="list-disc pl-5" data-slot="simulation-actions">
            {result.actionNames.map((n, i) => (
              <li key={`${n}-${String(i)}`}>{n}</li>
            ))}
          </ul>
        )}
        <p className="text-muted-foreground">{i18n.t("editor:preview.confirm.note")}</p>
      </div>
    )
    const ok = dialogs
      ? await dialogs.destructive({
          title: i18n.t("editor:preview.confirm.title", { control: name }),
          description,
          confirmLabel: i18n.t("editor:preview.confirm.run"),
        })
      : false
    if (ok) await send(control, kind, true)
  }

  const run = async (control: ControlId, kind: PressKind) => {
    if (busy.current || simulationUnavailable(target) !== null) return
    busy.current = true
    try {
      if (dirty && !live) {
        if (!dialogs) {
          toast.warning(i18n.t("editor:preview.unsaved.title"))
          return
        }
        const choice = await dialogs.select({
          title: i18n.t("editor:preview.unsaved.title"),
          description: i18n.t("editor:preview.unsaved.description"),
          options: [
            {
              id: "save",
              label: i18n.t("editor:preview.unsaved.save"),
              description: i18n.t("editor:preview.unsaved.saveDetail"),
            },
            {
              id: "saved",
              label: i18n.t("editor:preview.unsaved.saved"),
              description: i18n.t("editor:preview.unsaved.savedDetail"),
            },
          ],
          initial: "save",
          submitLabel: i18n.t("editor:preview.unsaved.continue"),
        })
        if (choice === null) return
        if (choice === "save" && !(await save())) return
      }
      await send(control, kind, false)
    } finally {
      busy.current = false
    }
  }

  return { unavailable: simulationUnavailable(target), run }
}
