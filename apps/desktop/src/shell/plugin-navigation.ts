/**
 * An entry a plugin contributes to the toolbar (`contributes.navigation`,
 * plugin API 1.3). Only installed, enabled and compatible plugins have
 * entries. The view is rendered by the host; a plugin provides no markup,
 * only a label and icons.
 */
export interface PluginNavigationItem {
  /** Unique across plugins, e.g. `<plugin id>/<entry id>`. */
  id: string
  plugin: string
  label: string
  /**
   * Icon images as data URLs, prepared and sanitised by the host. Without
   * one a generic icon is shown.
   */
  icon?: { light?: string; dark?: string }
  /** Opens the entry's host-rendered view. */
  onSelect: () => void
}

const NO_ITEMS: PluginNavigationItem[] = []

/**
 * The plugin entries of the toolbar. None yet: plugins declare them from
 * API level 1.3 on, and the agent does not report them to the app yet.
 * The shell already reserves their place (and the separator after them).
 */
export function usePluginNavigation(): PluginNavigationItem[] {
  return NO_ITEMS
}
