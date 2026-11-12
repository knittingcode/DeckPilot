import { ClipboardPaste, Copy, CopyPlus, Eraser, FolderOpen, Plus, Scissors } from "lucide-react"
import type { ReactElement } from "react"

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import type { BindingTarget } from "@/lib/types"

import type { TargetGroup } from "./targets"
import { shortcutLabel } from "@/lib/shortcuts"

export interface ControlMenuState {
  /** Number of controls the commands apply to. */
  count: number
  hasBindings: boolean
  canPaste: boolean
  folderName: string | undefined
}

export interface ControlMenuCommands {
  onOpen: () => void
  assign: (target: BindingTarget) => void
  copy: () => void
  cut: () => void
  paste: () => void
  duplicate: () => void
  clear: () => void
  openFolder: () => void
}

/** Right-click menu of a control; commands apply to the whole selection. */
export function ControlMenu({
  children,
  label,
  groups,
  state,
  commands,
}: {
  children: ReactElement
  label: string
  groups: TargetGroup[]
  state: ControlMenuState
  commands: ControlMenuCommands
}) {
  const subject = state.count > 1 ? `${state.count} controls` : label
  return (
    <ContextMenu
      onOpenChange={(open) => {
        if (open) commands.onOpen()
      }}
    >
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent className="w-56" aria-label={`${subject} actions`}>
        <ContextMenuLabel>{subject}</ContextMenuLabel>
        <ContextMenuSub>
          <ContextMenuSubTrigger>
            <Plus /> Assign
          </ContextMenuSubTrigger>
          <ContextMenuSubContent className="w-56">
            {groups.map((group, index) => (
              <ContextMenuGroup key={group.label}>
                {index > 0 && <ContextMenuSeparator />}
                <ContextMenuLabel>{group.label}</ContextMenuLabel>
                {group.options.map((option) => (
                  <ContextMenuItem
                    key={option.value}
                    onSelect={() => {
                      commands.assign(option.target)
                    }}
                  >
                    <span className="truncate">{option.label}</span>
                  </ContextMenuItem>
                ))}
              </ContextMenuGroup>
            ))}
          </ContextMenuSubContent>
        </ContextMenuSub>
        {state.folderName && (
          <ContextMenuItem onSelect={commands.openFolder}>
            <FolderOpen /> Open “{state.folderName}”
          </ContextMenuItem>
        )}
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={commands.copy}>
          <Copy /> Copy <ContextMenuShortcut>{shortcutLabel("C")}</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem onSelect={commands.cut} disabled={!state.hasBindings}>
          <Scissors /> Cut <ContextMenuShortcut>{shortcutLabel("X")}</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem onSelect={commands.paste} disabled={!state.canPaste}>
          <ClipboardPaste /> Paste <ContextMenuShortcut>{shortcutLabel("V")}</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem onSelect={commands.duplicate} disabled={!state.hasBindings}>
          <CopyPlus /> Duplicate <ContextMenuShortcut>{shortcutLabel("D")}</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem
          variant="destructive"
          onSelect={commands.clear}
          disabled={!state.hasBindings}
        >
          <Eraser /> Clear <ContextMenuShortcut>Del</ContextMenuShortcut>
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}
