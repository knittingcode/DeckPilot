// The one menu of the key header (`KeyMenu`): attached to the key preview
// (or, for several keys, a button), the same for every configurable key —
// actions, Multi Actions, plugin actions, workflows, layout navigation and
// folder keys. Two sections:
//
// - Icon: choose from the Icon Library, import a PNG or JPEG file, paste an
//   image from the clipboard, remove the key's own icon;
// - Background: choose from the Background Library, a custom colour, back
//   to the device's default.
//
// A menu item closes the menu and then opens its picker in one popover
// anchored to the preview (nothing nested); Escape or a click outside
// closes it and the focus goes back to the preview. Every change goes
// through the editor's draft (an undo step, shown by the live preview);
// nothing here runs the key. Imports and pastes go through the shared
// import pipeline (`useIcons`).

import {
  ClipboardPaste,
  ImageOff,
  Images,
  Palette,
  RotateCcw,
  Upload,
  Wallpaper,
} from "lucide-react"
import { type ReactNode, useRef, useState } from "react"

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Popover, PopoverAnchor, PopoverContent } from "@/components/ui/popover"
import { useTranslation } from "@/i18n"

import {
  MAX_ICONS,
  type SharedAppearance,
  contrastHint,
  isAssetBackground,
  keyColors,
  withIconFit,
} from "./appearance"
import { BackgroundLibrary } from "./background-library"
import { ColorField } from "./color-field"
import { IconLibrary } from "./icon-library"
import type { KeyLook } from "./key-look-context"
import { clipboardImage } from "./title-look"

const PASTE_MESSAGES = {
  empty: "editor:iconMenu.pasteEmpty",
  format: "editor:iconMenu.pasteFormat",
  denied: "editor:iconMenu.pasteDenied",
  unsupported: "editor:iconMenu.pasteUnsupported",
} as const

/** How readable the title is on the chosen colour, when it reads badly. */
function ContrastNote({ look, shared }: { look: KeyLook; shared: SharedAppearance }) {
  const background =
    shared.background === "mixed" || isAssetBackground(shared.background)
      ? undefined
      : shared.background
  const hint = contrastHint(
    background,
    shared.textColor === "mixed" ? undefined : shared.textColor,
    look.defaults,
  )
  if (hint.level === "good") return null
  return (
    <p role="status" className="mb-2 text-xs text-amber-700 dark:text-amber-400">
      {hint.message}
    </p>
  )
}

/** What the popover shows. */
type Picker = "icons" | "backgrounds" | "colour"

/**
 * The key menu around `trigger` (the key preview, a button). `shared` is
 * the look the selected keys share; a change goes to every selected key.
 */
export function KeyMenu({
  look,
  shared,
  trigger,
  onProblem,
}: {
  look: KeyLook
  shared: SharedAppearance
  /** The menu's trigger: a button (Radix `asChild`). */
  trigger: ReactNode
  /** A paste that put nothing on the key, and why (`null` clears it). */
  onProblem: (message: string | null) => void
}) {
  const { t } = useTranslation()
  const file = useRef<HTMLInputElement>(null)
  const anchor = useRef<HTMLDivElement>(null)
  const [picker, setPicker] = useState<Picker | null>(null)
  // Set while the menu closes into a picker, so the menu does not take the
  // focus back from it.
  const opening = useRef(false)
  const { icons, library } = look
  const own = shared.iconId !== undefined
  const explicitBackground = shared.background !== undefined
  const full = icons.icons.length >= MAX_ICONS
  const put = (id: string) => {
    const fit = shared.iconFit === "mixed" ? undefined : shared.iconFit
    look.restyle({ icon: fit && fit !== "fit" ? { id, fit } : { id } })
  }
  const importFile = async (chosen: File) => {
    onProblem(null)
    const icon = await icons.importFile(chosen)
    // A new icon goes onto the selected keys right away (an undo step).
    if (icon) put(icon.id)
  }
  const open = (next: Picker) => {
    opening.current = true
    setPicker(next)
  }
  const unset = keyColors({}, look.defaults)

  return (
    <Popover
      open={picker !== null}
      onOpenChange={(isOpen) => {
        if (!isOpen) setPicker(null)
      }}
    >
      <DropdownMenu modal={false}>
        <PopoverAnchor asChild ref={anchor}>
          <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
        </PopoverAnchor>
        <DropdownMenuContent
          align="start"
          collisionPadding={8}
          data-slot="key-menu"
          onCloseAutoFocus={(event) => {
            if (opening.current) {
              event.preventDefault()
              opening.current = false
            }
          }}
        >
          <DropdownMenuGroup aria-label={t("editor:keyMenu.icon")}>
            <DropdownMenuLabel className="text-xs text-muted-foreground">
              {t("editor:keyMenu.icon")}
            </DropdownMenuLabel>
            <DropdownMenuItem
              onSelect={() => {
                open("icons")
              }}
            >
              <Images /> {t("editor:iconMenu.library")}
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={icons.busy || full}
              onSelect={() => {
                file.current?.click()
              }}
            >
              <Upload /> {t("editor:iconMenu.import")}
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={icons.busy || full}
              onSelect={() => {
                void clipboardImage().then((found) => {
                  if (typeof found === "string") {
                    onProblem(t(PASTE_MESSAGES[found]))
                    return
                  }
                  void importFile(found)
                })
              }}
            >
              <ClipboardPaste /> {t("editor:iconMenu.paste")}
            </DropdownMenuItem>
            {own && (
              <DropdownMenuItem
                onSelect={() => {
                  // The action's (or the plugin's) own icon again.
                  look.restyle({ icon: null })
                }}
              >
                <ImageOff /> {t("editor:iconMenu.remove")}
              </DropdownMenuItem>
            )}
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup aria-label={t("editor:keyMenu.background")}>
            <DropdownMenuLabel className="text-xs text-muted-foreground">
              {t("editor:keyMenu.background")}
            </DropdownMenuLabel>
            <DropdownMenuItem
              onSelect={() => {
                open("backgrounds")
              }}
            >
              <Wallpaper /> {t("editor:keyMenu.chooseBackground")}
            </DropdownMenuItem>
            <DropdownMenuItem
              onSelect={() => {
                open("colour")
              }}
            >
              <Palette /> {t("editor:keyMenu.customColour")}
            </DropdownMenuItem>
            {explicitBackground && (
              <DropdownMenuItem
                onSelect={() => {
                  look.restyle({ background: null })
                }}
              >
                <RotateCcw /> {t("editor:keyMenu.resetBackground")}
              </DropdownMenuItem>
            )}
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      <input
        ref={file}
        type="file"
        accept="image/png,image/jpeg"
        className="sr-only"
        tabIndex={-1}
        aria-label={t("editor:icons.file")}
        onChange={(event) => {
          const chosen = event.target.files?.[0]
          event.target.value = ""
          if (chosen) void importFile(chosen)
        }}
      />
      <PopoverContent
        align="start"
        collisionPadding={8}
        data-slot="key-picker-popover"
        data-picker={picker ?? undefined}
        aria-label={
          picker === "icons"
            ? t("editor:iconMenu.library")
            : picker === "backgrounds"
              ? t("editor:keyMenu.backgroundLibrary")
              : t("editor:keyMenu.customColour")
        }
        role="dialog"
        className="max-h-[min(32rem,var(--radix-popover-content-available-height))] w-80 overflow-hidden p-3"
        onCloseAutoFocus={(event) => {
          // Back to the preview the menu was opened from.
          event.preventDefault()
          anchor.current?.focus()
        }}
      >
        {picker === "icons" && (
          <IconLibrary
            library={library}
            icons={icons.icons}
            usage={icons.usage}
            selected={shared.iconId}
            fit={shared.iconFit}
            busy={icons.busy}
            problem={icons.problem}
            onSelect={(id) => {
              if (id) put(id)
              else look.restyle({ icon: null })
            }}
            onFit={(fit) => {
              look.restyle(withIconFit(fit))
            }}
            onImport={(chosen) => {
              void importFile(chosen)
            }}
            onDelete={icons.remove}
          />
        )}
        {picker === "backgrounds" && (
          <BackgroundLibrary
            library={library}
            icons={icons.icons}
            value={shared.background}
            onChoose={(background) => {
              look.restyle({ background })
            }}
            onCustom={() => {
              setPicker("colour")
            }}
            onReset={() => {
              look.restyle({ background: null })
            }}
          />
        )}
        {picker === "colour" && <ContrastNote look={look} shared={shared} />}
        {picker === "colour" && (
          <ColorField
            allowTransparent
            label={t("editor:appearance.background")}
            value={shared.background}
            defaultValue={look.defaults.keyBackground}
            effectiveDefault={unset.background}
            transparentShows={look.defaults.displayBackground}
            onChange={(value, group) => {
              look.restyle({ background: value }, group)
            }}
          />
        )}
      </PopoverContent>
    </Popover>
  )
}
