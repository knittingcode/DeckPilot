/**
 * What a view of the main window works on, and so which of the toolbar's
 * selectors it shows: the device (and its connection), the profile being
 * edited (and whether it is active on the device), both or neither.
 * Hiding a selector never changes the selection: it is kept by the shell
 * (`EditorSelectionProvider`, `editingProfileId`) and shown again with the
 * next view that uses it.
 *
 * Plugin views declare theirs in their manifest (`contributes.views[].context`,
 * plugin API 1.4); the declaration only decides what the app's own toolbar
 * shows and gives the plugin no data.
 */
export type PluginToolbarContext = "none" | "device" | "profile" | "device+profile"

/**
 * A view's context: one a plugin view may declare, or the Device Editor's
 * own, which adds the layout selector. Plugins cannot declare that one: the
 * layout selector edits the app's layouts.
 */
export type ToolbarContext = PluginToolbarContext | "device+profile+layout"

/** The contexts plugin views may declare. */
export const TOOLBAR_CONTEXTS: readonly PluginToolbarContext[] = [
  "none",
  "device",
  "profile",
  "device+profile",
]

/** `value` as a plugin view's context; anything else (absent, unknown, malformed) is `none`. */
export function toolbarContext(value: unknown): PluginToolbarContext {
  return (TOOLBAR_CONTEXTS as readonly unknown[]).includes(value)
    ? (value as PluginToolbarContext)
    : "none"
}

export function showsDevice(context: ToolbarContext): boolean {
  return context === "device" || context === "device+profile" || showsLayout(context)
}

export function showsProfile(context: ToolbarContext): boolean {
  return context === "profile" || context === "device+profile" || showsLayout(context)
}

/** The Device Editor only: the layout of the device's model in the profile. */
export function showsLayout(context: ToolbarContext): boolean {
  return context === "device+profile+layout"
}

/**
 * The contexts of the core pages. The device editor works on a layout of a
 * device in a profile; the Action Library and Workflows on the profile being edited.
 * The dashboard (the active profile and every device), the Layouts page
 * (every profile's layouts, with its own filter) and the Marketplace work
 * on neither.
 */
export const CORE_PAGE_CONTEXTS = {
  designer: "device+profile+layout",
  actions: "profile",
  workflows: "profile",
  dashboard: "none",
  layouts: "none",
  marketplace: "none",
} as const satisfies Record<string, ToolbarContext>

/** The context of a core page; `undefined` for other pages. */
export function corePageContext(page: string): ToolbarContext | undefined {
  return Object.hasOwn(CORE_PAGE_CONTEXTS, page)
    ? CORE_PAGE_CONTEXTS[page as keyof typeof CORE_PAGE_CONTEXTS]
    : undefined
}
