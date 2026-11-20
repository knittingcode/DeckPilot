import { Check, ChevronDown, Cpu, Ellipsis, Puzzle, Settings, Store, UserRound } from "lucide-react"
import { useState, useSyncExternalStore } from "react"

import { CONTROL, CONTROL_ICON } from "@/components/app-titlebar"
import { NotificationsButton } from "@/components/notifications"
import { Button } from "@/components/ui/button"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Separator } from "@/components/ui/separator"
import { LayoutMenu } from "@/layout-editor/layout-management"
import { useConfirmDiscard } from "@/layout-editor/layout-selection"
import { SHORTCUTS } from "@/hooks/use-navigation"
import { useStatus } from "@/hooks/use-status"
import { useTranslation } from "@/i18n"
import { useAppContext } from "@/lib/app-context"
import { THEME } from "@/lib/theme"
import { cn } from "@/lib/utils"

import { type DeviceOption, currentOption, deviceOptions } from "./device-options"
import { useEditorSelection } from "./editor-selection-context"
import {
  type PluginNavigationItem,
  loadNavigationOrder,
  moveInOrder,
  navigationKey,
  orderedItems,
  saveNavigationOrder,
  splitEntries,
  useNavigationIcon,
} from "./plugin-navigation"

// Narrower windows truncate the selectors' names; their tooltips and
// accessible names keep the whole name.
const SELECTOR = cn(
  CONTROL,
  "max-w-56 justify-start font-normal max-[1100px]:max-w-40 max-[820px]:max-w-28",
)

function Dot({ on, className }: { on: boolean; className?: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        "size-1.5 shrink-0 rounded-full",
        on ? "bg-emerald-500" : "bg-muted-foreground/40",
        className,
      )}
    />
  )
}

/**
 * The device the editor shows: a connected device, or a model of the
 * catalog without the hardware. Choosing one opens the editor.
 */
export function DeviceSelector({ onChosen }: { onChosen?: () => void }) {
  const { t } = useTranslation()
  const { agent } = useAppContext()
  const selection = useEditorSelection()
  const models = selection.models.status === "ready" ? selection.models.data : []
  const options = deviceOptions(models, agent?.status?.devices ?? [])
  const current = currentOption(options, selection.modelId, selection.device)
  const connected = options.filter((o) => o.connected)
  const others = options.filter((o) => !o.connected)
  const label = current
    ? current.connected
      ? t("navigation:device.selectorConnected", { device: current.label })
      : t("navigation:device.selectorNotConnected", { device: current.label })
    : t("navigation:device.choose")
  const confirmDiscard = useConfirmDiscard()
  const choose = async (option: DeviceOption) => {
    // The device shown already: only opens the editor.
    if (option.key === current?.key) {
      onChosen?.()
      return
    }
    if (!(await confirmDiscard())) return
    if (selection.selectDevice(option.model, option.device)) onChosen?.()
  }
  const item = (option: DeviceOption) => (
    <DropdownMenuItem
      key={option.key}
      disabled={!option.available}
      aria-current={option.key === current?.key ? "true" : undefined}
      onSelect={() => {
        void choose(option)
      }}
    >
      <Check className={cn(option.key !== current?.key && "invisible")} />
      <span className="grid min-w-0 flex-1">
        <span className="truncate">{option.label}</span>
        <span className="truncate text-xs text-muted-foreground">{option.detail}</span>
      </span>
      <Dot on={option.connected} />
    </DropdownMenuItem>
  )
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className={SELECTOR}
          aria-label={label}
          title={label}
          disabled={selection.models.status !== "ready"}
        >
          <Cpu />
          <span className="truncate">{current?.label ?? t("navigation:device.placeholder")}</span>
          {current && <Dot on={current.connected} />}
          <ChevronDown className="opacity-60" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-72">
        {connected.length > 0 && (
          <DropdownMenuGroup aria-label={t("navigation:device.connectedGroup")}>
            <DropdownMenuLabel>{t("navigation:device.connected")}</DropdownMenuLabel>
            {connected.map(item)}
          </DropdownMenuGroup>
        )}
        {connected.length > 0 && others.length > 0 && <DropdownMenuSeparator />}
        {others.length > 0 && (
          <DropdownMenuGroup aria-label={t("navigation:device.modelsGroup")}>
            <DropdownMenuLabel>
              {connected.length > 0
                ? t("navigation:device.otherModels")
                : t("navigation:device.models")}
            </DropdownMenuLabel>
            {others.map(item)}
          </DropdownMenuGroup>
        )}
        {options.length === 0 && (
          <DropdownMenuItem disabled>{t("navigation:device.noModels")}</DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** The profile all editors work on; independent of the profile on the device. */
export function ProfileSelector() {
  const { t } = useTranslation()
  const { profiles, editingProfileId, openSettings } = useAppContext()
  const selection = useEditorSelection()
  const confirmDiscard = useConfirmDiscard()
  const editing = profiles.profiles.find((p) => p.id === editingProfileId)
  const onDevice = editing !== undefined && editing.id === profiles.activeProfileId
  const label = editing
    ? onDevice
      ? t("navigation:profile.selectorActive", { profile: editing.name })
      : t("navigation:profile.selector", { profile: editing.name })
    : t("navigation:profile.choose")
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="sm" className={SELECTOR} aria-label={label} title={label}>
          <UserRound />
          <span className="truncate">{editing?.name ?? t("navigation:profile.none")}</span>
          {editing && <Dot on={onDevice} />}
          <ChevronDown className="opacity-60" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="w-64" align="start">
        <DropdownMenuLabel>{t("navigation:profile.edit")}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {profiles.profiles.length === 0 && (
          <DropdownMenuItem disabled>{t("navigation:profile.noProfiles")}</DropdownMenuItem>
        )}
        {profiles.profiles.map((profile) => (
          <DropdownMenuItem
            key={profile.id}
            aria-current={profile.id === editingProfileId ? "true" : undefined}
            onSelect={() => {
              if (profile.id === editingProfileId) return
              void confirmDiscard().then((ok) => {
                if (ok) selection.selectProfile(profile.id)
              })
            }}
          >
            <Check className={cn(profile.id !== editingProfileId && "invisible")} />
            <span className="flex-1 truncate">{profile.name}</span>
            {profile.id === profiles.activeProfileId && (
              <span className="text-xs text-muted-foreground">
                {t("navigation:profile.onDevice")}
              </span>
            )}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={() => {
            openSettings("profiles")
          }}
        >
          <Settings /> {t("navigation:profile.manage")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * The layout the Device Editor shows, of the profile being edited for the
 * device's model, and the layouts' management (layout-editor/layout-management.tsx).
 */
export function LayoutSelector() {
  return <LayoutMenu className={SELECTOR} />
}

function PluginIcon({ item }: { item: PluginNavigationItem }) {
  // DeckPilot is always dark: the plugin's icon for dark backgrounds.
  const source = useNavigationIcon(item, THEME)
  // Only ever an image: the agent's sanitised data URL, else the app's own icon.
  if (!source) return <Puzzle aria-hidden data-slot="plugin-icon-fallback" />
  return (
    <img
      src={source}
      alt=""
      aria-hidden
      data-slot="plugin-icon"
      className="size-(--titlebar-icon) object-contain"
    />
  )
}

/** Below this window width every entry is in "More" (CSS pixels). */
export const NARROW_TOOLBAR_BELOW = 720

const NARROW = `(max-width: ${NARROW_TOOLBAR_BELOW - 1}px)`

function subscribeNarrow(listener: () => void) {
  if (typeof window.matchMedia !== "function") return () => undefined
  const media = window.matchMedia(NARROW)
  media.addEventListener("change", listener)
  return () => {
    media.removeEventListener("change", listener)
  }
}

function isNarrow() {
  return typeof window.matchMedia === "function" && window.matchMedia(NARROW).matches
}

/**
 * The entries plugins contribute (docs/device-controls.md section 4), in
 * the order the user gave them (kept per computer), with a "More" menu
 * when they do not fit. Nothing at all when there are none.
 */
export function PluginNavigation({
  items,
  current,
  onSelect,
}: {
  items: PluginNavigationItem[]
  /** The entry whose view is open (its `navigationKey`). */
  current?: string | null
  onSelect: (item: PluginNavigationItem) => void
}) {
  const { t } = useTranslation()
  const [order, setOrderState] = useState(loadNavigationOrder)
  const narrow = useSyncExternalStore(subscribeNarrow, isNarrow, () => false)
  if (items.length === 0) return null
  const ordered = orderedItems(items, order)
  const { inline, overflow } = splitEntries(ordered, narrow)
  const move = (item: PluginNavigationItem, index: number) => {
    const next = moveInOrder(ordered, navigationKey(item), index)
    setOrderState(next)
    saveNavigationOrder(next)
  }
  return (
    <nav
      aria-label={t("navigation:plugins.label")}
      className="flex items-center gap-(--titlebar-gap)"
    >
      {inline.map((item, index) => {
        const open = current === navigationKey(item)
        return (
          <ContextMenu key={navigationKey(item)}>
            <ContextMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className={cn(CONTROL_ICON, open && "bg-accent text-accent-foreground")}
                aria-label={item.label}
                aria-pressed={open}
                title={item.label}
                data-plugin-entry={navigationKey(item)}
                onClick={() => {
                  onSelect(item)
                }}
              >
                <PluginIcon item={item} />
              </Button>
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem
                disabled={index === 0}
                onSelect={() => {
                  move(item, index - 1)
                }}
              >
                {t("navigation:plugins.moveLeft")}
              </ContextMenuItem>
              <ContextMenuItem
                disabled={index === ordered.length - 1}
                onSelect={() => {
                  move(item, index + 1)
                }}
              >
                {t("navigation:plugins.moveRight")}
              </ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        )
      })}
      {overflow.length > 0 && (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className={CONTROL_ICON}
              aria-label={t("navigation:plugins.more")}
              title={t("navigation:plugins.more")}
            >
              <Ellipsis />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-56">
            <DropdownMenuLabel>{t("navigation:plugins.label")}</DropdownMenuLabel>
            {overflow.map((item) => (
              <DropdownMenuItem
                key={navigationKey(item)}
                aria-current={current === navigationKey(item) ? "page" : undefined}
                onSelect={() => {
                  onSelect(item)
                }}
              >
                <PluginIcon item={item} />
                <span className="truncate">{item.label}</span>
              </DropdownMenuItem>
            ))}
            {inline.length > 0 && (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuSub>
                  <DropdownMenuSubTrigger>
                    {t("navigation:plugins.moveToToolbar")}
                  </DropdownMenuSubTrigger>
                  <DropdownMenuSubContent>
                    {overflow.map((item) => (
                      <DropdownMenuItem
                        key={navigationKey(item)}
                        onSelect={() => {
                          move(item, inline.length - 1)
                        }}
                      >
                        {item.label}
                      </DropdownMenuItem>
                    ))}
                  </DropdownMenuSubContent>
                </DropdownMenuSub>
              </>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </nav>
  )
}

/**
 * The right end of the toolbar: plugin entries, a separator only when
 * there are any, then notifications, the marketplace (in builds with
 * plugins) and the settings.
 */
export function ToolbarEnd({
  plugins,
  pluginOpen = null,
  onPlugin = () => undefined,
  marketplace,
  marketplaceOpen = false,
  onMarketplace,
  settingsOpen = false,
}: {
  plugins: PluginNavigationItem[]
  /** The plugin entry whose view is open (its `navigationKey`). */
  pluginOpen?: string | null
  /** Opens a plugin entry's view. */
  onPlugin?: (item: PluginNavigationItem) => void
  /** The build has the plugin marketplace. */
  marketplace: boolean
  marketplaceOpen?: boolean
  onMarketplace: () => void
  /** The settings are the view on screen. */
  settingsOpen?: boolean
}) {
  const { t } = useTranslation()
  const { updates, openSettings } = useAppContext()
  const stores = useStatus()
  const updateAvailable = Boolean(updates?.available)
  const settingsLabel = updateAvailable
    ? t("navigation:toolbar.settingsUpdate")
    : t("navigation:toolbar.settings")
  return (
    <div
      role="group"
      aria-label={t("navigation:toolbar.app")}
      className="flex items-center gap-(--titlebar-gap)"
    >
      <PluginNavigation items={plugins} current={pluginOpen} onSelect={onPlugin} />
      {plugins.length > 0 && (
        <Separator
          orientation="vertical"
          data-slot="plugin-navigation-separator"
          className="mx-1 data-[orientation=vertical]:h-4"
        />
      )}
      {stores && <NotificationsButton store={stores.notifications} className={CONTROL_ICON} />}
      {marketplace && (
        <Button
          variant="ghost"
          size="icon"
          className={cn(CONTROL_ICON, marketplaceOpen && "bg-accent text-accent-foreground")}
          aria-label={t("navigation:toolbar.marketplace")}
          aria-pressed={marketplaceOpen}
          title={t("navigation:toolbar.marketplace")}
          onClick={onMarketplace}
        >
          <Store />
        </Button>
      )}
      <Button
        variant="ghost"
        size="icon"
        className={cn(CONTROL_ICON, "relative", settingsOpen && "bg-accent text-accent-foreground")}
        aria-label={settingsLabel}
        aria-pressed={settingsOpen}
        title={t("navigation:toolbar.withShortcut", {
          label: settingsLabel,
          shortcut: SHORTCUTS.settings,
        })}
        onClick={() => {
          openSettings(updateAvailable ? "updates" : undefined)
        }}
      >
        <Settings />
        {updateAvailable && (
          <span
            aria-hidden
            className="absolute top-0.5 right-0.5 size-1.5 rounded-full bg-primary"
          />
        )}
      </Button>
    </div>
  )
}
