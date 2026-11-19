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
export type ToolbarContext = "none" | "device" | "profile" | "device+profile"

export const TOOLBAR_CONTEXTS: readonly ToolbarContext[] = [
  "none",
  "device",
  "profile",
  "device+profile",
]

/** `value` as a context; anything else (absent, unknown, malformed) is `none`. */
export function toolbarContext(value: unknown): ToolbarContext {
  return (TOOLBAR_CONTEXTS as readonly unknown[]).includes(value)
    ? (value as ToolbarContext)
    : "none"
}

export function showsDevice(context: ToolbarContext): boolean {
  return context === "device" || context === "device+profile"
}

export function showsProfile(context: ToolbarContext): boolean {
  return context === "profile" || context === "device+profile"
}

/**
 * The contexts of the core pages. The device editor works on a device of a
 * profile; the Action Library and Workflows on the profile being edited.
 * The dashboard (the active profile and every device), the Layouts page
 * (every profile's layouts, with its own filter) and the Marketplace work
 * on neither.
 */
export const CORE_PAGE_CONTEXTS = {
  designer: "device+profile",
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
