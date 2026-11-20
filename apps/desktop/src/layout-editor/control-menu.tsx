import {
  ArrowLeft,
  ClipboardPaste,
  Copy,
  CopyPlus,
  Eraser,
  FlaskConical,
  FolderOpen,
  FolderPlus,
  Plus,
  Scissors,
  SlidersHorizontal,
} from "lucide-react"
import { useRef, type KeyboardEvent, type ReactElement, type ReactNode } from "react"

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
import { useTranslation } from "@/i18n"
import type { BindingTarget, PressKind } from "@/lib/types"

import type { TargetGroup } from "./targets"
import { shortcutLabel } from "@/lib/shortcuts"

export interface ControlMenuState {
  /** Number of controls the commands apply to. */
  count: number
  hasBindings: boolean
  canPaste: boolean
  folderName: string | undefined
  /** The control is empty: a folder can be created on it. */
  canCreateFolder: boolean
  /** A double-click on it goes back (a Back key, the automatic one included). */
  goesBack?: boolean
  /**
   * Why the control cannot be pressed on the device now (no action, no
   * device, another page shown…); `null` when it can. Absent: no "Test
   * action" (a menu without a device to test on).
   */
  testUnavailable?: string | null
  /**
   * The control is the folder's protected system Back: only going back and
   * testing it are offered (nothing to assign, move or remove).
   */
  systemBack?: boolean
  /**
   * The control is a folder key (`lib/folder-control.ts`): it only opens its
   * folder, so nothing can be assigned or pasted onto it and only a single
   * press can be tested. Opening the folder, copying, moving and removing
   * stay.
   */
  folderControl?: boolean
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
  newFolder: () => void
  /** Back in the editor (as a double-click on a Back key). */
  goBack?: () => void
  /** Asks the agent to press the control (a simulation; never the editor). */
  test?: (kind: PressKind) => void
  /** Selects the control alone and moves focus to the inspector. */
  configure?: () => void
}

/** A menu item's text with, when it is disabled, why under it. */
function ItemText({ children, reason }: { children: ReactNode; reason?: string | null }) {
  return (
    <span className="grid min-w-0">
      <span className="truncate">{children}</span>
      {reason && (
        <span data-slot="menu-reason" className="text-xs whitespace-normal text-muted-foreground">
          {reason}
        </span>
      )}
    </span>
  )
}

const PRESSES: {
  kind: PressKind
  label: "editor:preview.single" | "editor:preview.double" | "editor:preview.long"
}[] = [
  { kind: "single", label: "editor:preview.single" },
  { kind: "double", label: "editor:preview.double" },
  { kind: "long", label: "editor:preview.long" },
]

/**
 * Opens the menu from the keyboard (Shift+F10, the context menu key) at the
 * control, as a right-click there would.
 */
function openFromKeyboard(event: KeyboardEvent<HTMLElement>) {
  if (!(event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))) return
  event.preventDefault()
  const element = event.currentTarget
  const box = element.getBoundingClientRect()
  element.dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      button: 2,
      clientX: box.left + box.width / 2,
      clientY: box.top + box.height / 2,
    }),
  )
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
  const { t } = useTranslation()
  const subject = state.count > 1 ? t("editor:controlMenu.controls", { count: state.count }) : label
  // "Configure action" moves focus to the inspector, not back to the key.
  const toInspector = useRef(false)
  const noBindings = state.hasBindings ? null : t("editor:preview.why.noAction")
  return (
    <ContextMenu
      onOpenChange={(open) => {
        if (open) commands.onOpen()
      }}
    >
      <ContextMenuTrigger asChild onKeyDown={openFromKeyboard}>
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent
        className="w-64"
        aria-label={t("editor:controlMenu.label", { subject })}
        onCloseAutoFocus={(event) => {
          if (!toInspector.current) return
          toInspector.current = false
          event.preventDefault()
          commands.configure?.()
        }}
      >
        <ContextMenuLabel>{subject}</ContextMenuLabel>
        {state.testUnavailable !== undefined && commands.test && (
          <ContextMenuSub>
            <ContextMenuSubTrigger
              disabled={state.testUnavailable !== null}
              data-slot="test-action"
              aria-description={state.testUnavailable ?? t("editor:preview.testHint")}
            >
              <FlaskConical />
              <ItemText reason={state.testUnavailable}>{t("editor:preview.test")}</ItemText>
            </ContextMenuSubTrigger>
            <ContextMenuSubContent className="w-56">
              <ContextMenuLabel className="text-xs font-normal">
                {t("editor:preview.testHint")}
              </ContextMenuLabel>
              {PRESSES.filter((press) => !state.folderControl || press.kind === "single").map(
                (press) => (
                  <ContextMenuItem
                    key={press.kind}
                    onSelect={() => {
                      commands.test?.(press.kind)
                    }}
                  >
                    {t(press.label)}
                  </ContextMenuItem>
                ),
              )}
            </ContextMenuSubContent>
          </ContextMenuSub>
        )}
        {state.systemBack ? (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem
              onSelect={commands.goBack}
              disabled={!state.goesBack || !commands.goBack}
              data-slot="system-back-go"
            >
              <ArrowLeft /> {t("editor:preview.goBack")}
            </ContextMenuItem>
          </>
        ) : (
          <>
            {commands.configure && !state.folderControl && (
              <ContextMenuItem
                disabled={state.count !== 1 || noBindings !== null}
                onSelect={() => {
                  toInspector.current = true
                }}
              >
                <SlidersHorizontal />
                <ItemText reason={noBindings}>{t("editor:preview.configure")}</ItemText>
              </ContextMenuItem>
            )}
            <ContextMenuSeparator />
            {!(state.folderControl && state.count === 1) && (
              <ContextMenuSub>
                <ContextMenuSubTrigger>
                  <Plus /> {t("editor:controlMenu.assign")}
                </ContextMenuSubTrigger>
                <ContextMenuSubContent className="w-56">
                  {groups.map((group, index) => (
                    <ContextMenuGroup key={group.label}>
                      {index > 0 && <ContextMenuSeparator />}
                      <ContextMenuLabel>{group.label}</ContextMenuLabel>
                      {group.options.map((option) => (
                        <ContextMenuItem
                          key={option.value}
                          disabled={
                            option.unavailable !== undefined || option.incompatible !== undefined
                          }
                          title={option.unavailable ?? option.incompatible}
                          onSelect={() => {
                            commands.assign(option.target)
                          }}
                        >
                          <span className="truncate">
                            {option.unavailable !== undefined
                              ? t("editor:inspector.optionUnavailable", { option: option.label })
                              : option.incompatible !== undefined
                                ? t("editor:inspector.optionNotHere", { option: option.label })
                                : option.label}
                          </span>
                        </ContextMenuItem>
                      ))}
                    </ContextMenuGroup>
                  ))}
                </ContextMenuSubContent>
              </ContextMenuSub>
            )}
            {state.goesBack && commands.goBack && (
              <ContextMenuItem onSelect={commands.goBack}>
                <ArrowLeft /> {t("editor:preview.goBack")}
              </ContextMenuItem>
            )}
            {state.folderName ? (
              <ContextMenuItem onSelect={commands.openFolder}>
                <FolderOpen /> {t("editor:controlMenu.openFolder", { folder: state.folderName })}
              </ContextMenuItem>
            ) : (
              <ContextMenuItem
                onSelect={commands.newFolder}
                disabled={!state.canCreateFolder}
                title={state.canCreateFolder ? undefined : t("editor:controlMenu.folderTaken")}
              >
                <FolderPlus /> {t("editor:controlMenu.newFolder")}
              </ContextMenuItem>
            )}
            <ContextMenuSeparator />
            <ContextMenuItem onSelect={commands.copy}>
              <Copy /> {t("editor:controlMenu.copy")}{" "}
              <ContextMenuShortcut>{shortcutLabel("C")}</ContextMenuShortcut>
            </ContextMenuItem>
            <ContextMenuItem onSelect={commands.cut} disabled={!state.hasBindings}>
              <Scissors /> {t("editor:controlMenu.cut")}{" "}
              <ContextMenuShortcut>{shortcutLabel("X")}</ContextMenuShortcut>
            </ContextMenuItem>
            <ContextMenuItem
              onSelect={commands.paste}
              disabled={!state.canPaste || (state.folderControl && state.count === 1)}
            >
              <ClipboardPaste /> {t("editor:controlMenu.paste")}{" "}
              <ContextMenuShortcut>{shortcutLabel("V")}</ContextMenuShortcut>
            </ContextMenuItem>
            <ContextMenuItem onSelect={commands.duplicate} disabled={!state.hasBindings}>
              <CopyPlus /> {t("editor:controlMenu.duplicate")}{" "}
              <ContextMenuShortcut>{shortcutLabel("D")}</ContextMenuShortcut>
            </ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem
              variant="destructive"
              onSelect={commands.clear}
              disabled={!state.hasBindings}
            >
              <Eraser />
              <ItemText reason={noBindings}>{t("editor:preview.removeAssignment")}</ItemText>
              <ContextMenuShortcut>{t("editor:controlMenu.deleteKey")}</ContextMenuShortcut>
            </ContextMenuItem>
          </>
        )}
      </ContextMenuContent>
    </ContextMenu>
  )
}
