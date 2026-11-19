import { ArrowLeft, Folder, FolderPlus, ListOrdered, ListPlus } from "lucide-react"
import type { ReactNode } from "react"

import { ActionIcon } from "@/components/action-icon"
import { controlKey, controlLabel } from "@/lib/layout-model"
import type { ActionInstance, ActionType, KeyFace, Workflow } from "@/lib/types"

import type { DragSource } from "./editing"

const ICON = "size-4 shrink-0"

/** What the drag preview shows of a dragged library entry or control. */
export interface DragLook {
  label: string
  icon: ReactNode
  /** For a dragged control: its image as the device draws it, else its face. */
  key?: { image?: string; face?: KeyFace; icon?: string | null }
}

/** The look of `source` while it is dragged. */
export function dragLook(
  source: DragSource,
  context: {
    actions: ActionInstance[]
    workflows: Workflow[]
    types: Map<string, ActionType>
    /** The name of a page, for "Open …". */
    pageName: (id: string) => string | undefined
    images?: Map<string, string>
    faces?: Map<string, KeyFace>
    icons?: Map<string, string | null | undefined>
  },
): DragLook {
  switch (source.type) {
    case "actionType": {
      const type = context.types.get(source.actionType)
      return {
        label: source.preset?.name ?? type?.name ?? "Action",
        icon: <ActionIcon name={source.preset?.icon ?? type?.node?.icon} className={ICON} />,
      }
    }
    case "newFolder":
      return { label: "New folder", icon: <FolderPlus className={ICON} aria-hidden /> }
    case "newMultiAction":
      return { label: "Multi Action", icon: <ListPlus className={ICON} aria-hidden /> }
    case "control": {
      const key = controlKey(source.control)
      return {
        label: controlLabel(source.control),
        icon: null,
        key: {
          image: context.images?.get(key),
          face: context.faces?.get(key),
          icon: context.icons?.get(key),
        },
      }
    }
    case "target": {
      const target = source.target
      switch (target.type) {
        case "action": {
          const action = context.actions.find((a) => a.id === target.action)
          return {
            label: action?.name ?? "Missing action",
            icon: <ActionIcon name={action?.icon} className={ICON} />,
          }
        }
        case "workflow":
          return {
            label: context.workflows.find((w) => w.id === target.workflow)?.name ?? "Workflow",
            icon: <ListOrdered className={ICON} aria-hidden />,
          }
        case "openPage":
          return {
            label: `Open ${context.pageName(target.page) ?? "page"}`,
            icon: <Folder className={ICON} aria-hidden />,
          }
        case "back":
          return { label: "Back", icon: <ArrowLeft className={ICON} aria-hidden /> }
        case "stateful":
          return { label: "Work Session", icon: <ActionIcon name={null} className={ICON} /> }
      }
    }
  }
}
