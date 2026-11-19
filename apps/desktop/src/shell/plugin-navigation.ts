// The toolbar entries plugins contribute (`contributes.navigation`, plugin
// API 1.3; docs/device-controls.md section 4). An entry opens a view the
// app draws itself: the plugin's variables, its actions or its settings. A
// plugin provides only a label, the kind of view and icons, which the
// agent checks and sanitises and the app shows only through `<img>`; no
// markup, URL or code of a plugin reaches the shell.
//
// Only builds with `VITE_DECKPILOT_PLUGINS=1` ask the agent for entries;
// every other build has none and loads no plugin client.

import { createContext, useContext, useEffect, useState } from "react"

import { usePluginClientOverride, type PluginClient } from "@/plugins/api"
import type { NavigationIconTheme, PluginNavigationItem, PluginViewKind } from "@/plugins/types"

export type { PluginNavigationItem } from "@/plugins/types"

/** The view kinds this app draws; entries of other kinds are not shown. */
export const VIEW_KINDS: readonly PluginViewKind[] = ["variables", "actions", "settings"]

export function isKnownViewKind(kind: string): kind is PluginViewKind {
  return (VIEW_KINDS as readonly string[]).includes(kind)
}

/** An entry's key among all plugins' entries. */
export function navigationKey(item: Pick<PluginNavigationItem, "plugin" | "id">): string {
  return `${item.plugin}/${item.id}`
}

// ---------------------------------------------------------------------------
// The view's page in the main window

const VIEW_PAGE = "plugin-view"

/** The page of the main window that shows the entry's view. */
export function pluginViewPage(item: Pick<PluginNavigationItem, "plugin" | "id">): string {
  return `${VIEW_PAGE}/${navigationKey(item)}`
}

/** The entry a page of `pluginViewPage` is for; `null` for any other page. */
export function parsePluginViewPage(page: string): { plugin: string; entry: string } | null {
  const [root, plugin, entry, ...rest] = page.split("/")
  if (root !== VIEW_PAGE || !plugin || !entry || rest.length > 0) return null
  return { plugin, entry }
}

/**
 * A plugin's variable namespace, `plugins.<publisherCamel>.<nameCamel>`
 * (`PluginId::variable_namespace`).
 */
export function pluginVariableNamespace(plugin: string): string {
  const camel = (text: string) =>
    text
      .split("-")
      .map((part, i) => (i === 0 ? part : part.charAt(0).toUpperCase() + part.slice(1)))
      .join("")
  const dot = plugin.indexOf(".")
  return `plugins.${camel(plugin.slice(0, dot))}.${camel(plugin.slice(dot + 1))}`
}

// ---------------------------------------------------------------------------
// Order, kept per computer

export const NAVIGATION_ORDER_KEY = "deckpilot.toolbar.pluginOrder"

type Storage = Pick<globalThis.Storage, "getItem" | "setItem">

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage
  } catch {
    return null
  }
}

/** The order the user gave the entries (their keys); empty when none. */
export function loadNavigationOrder(store: Storage | null = storage()): string[] {
  try {
    const parsed: unknown = JSON.parse(store?.getItem(NAVIGATION_ORDER_KEY) ?? "[]")
    return Array.isArray(parsed) ? parsed.filter((k): k is string => typeof k === "string") : []
  } catch {
    return []
  }
}

export function saveNavigationOrder(order: string[], store: Storage | null = storage()) {
  try {
    store?.setItem(NAVIGATION_ORDER_KEY, JSON.stringify(order))
  } catch {
    // Remembered for this session only.
  }
}

/**
 * The entries in the user's order; entries the order does not know (new
 * plugins) follow in the agent's order.
 */
export function orderedItems<T extends Pick<PluginNavigationItem, "plugin" | "id">>(
  items: T[],
  order: string[],
): T[] {
  const rank = new Map(order.map((key, i) => [key, i]))
  return items
    .map((item, i) => ({ item, i, rank: rank.get(navigationKey(item)) }))
    .sort((a, b) => {
      if (a.rank !== undefined && b.rank !== undefined) return a.rank - b.rank
      if (a.rank !== undefined) return -1
      if (b.rank !== undefined) return 1
      return a.i - b.i
    })
    .map(({ item }) => item)
}

/** The order after moving the entry `key` to `index` of `items` (as shown). */
export function moveInOrder(
  items: Pick<PluginNavigationItem, "plugin" | "id">[],
  key: string,
  index: number,
): string[] {
  const keys = items.map(navigationKey)
  const from = keys.indexOf(key)
  if (from < 0 || index < 0 || index >= keys.length) return keys
  const next = [...keys]
  next.splice(from, 1)
  next.splice(index, 0, key)
  return next
}

/** At most this many entries in the toolbar; with more, the rest are in "More". */
export const INLINE_PLUGIN_ENTRIES = 4

/** The entries shown in the toolbar and those in its "More" menu. */
export function splitEntries<T>(items: T[], narrow: boolean): { inline: T[]; overflow: T[] } {
  if (narrow) return { inline: [], overflow: items }
  if (items.length <= INLINE_PLUGIN_ENTRIES) return { inline: items, overflow: [] }
  const inline = items.slice(0, INLINE_PLUGIN_ENTRIES - 1)
  return { inline, overflow: items.slice(inline.length) }
}

// ---------------------------------------------------------------------------
// Loading

async function pluginClient(override: PluginClient | null): Promise<PluginClient> {
  return override ?? (await import("@/plugins/client")).createTauriPluginClient()
}

const NONE: PluginNavigationItem[] = []

/**
 * The entries of installed, enabled plugins whose view kind this app
 * draws, read again when `reloadKey` changes (the agent or the plugins
 * changed) and whenever the agent reports that a plugin was installed,
 * enabled, disabled or uninstalled. None while the agent cannot answer.
 */
export function usePluginNavigation(reloadKey: unknown): PluginNavigationItem[] {
  const override = usePluginClientOverride()
  const [items, setItems] = useState<PluginNavigationItem[]>(NONE)
  useEffect(() => {
    // Inline, so that Vite folds it and builds without plugins have no client.
    if (import.meta.env.VITE_DECKPILOT_PLUGINS !== "1") return
    let active = true
    let stop: (() => void) | undefined
    let generation = 0
    const load = (client: PluginClient) => {
      const current = ++generation
      client.listNavigation().then(
        (listed) => {
          if (!active || current !== generation) return
          const shown = listed.filter((item) => isKnownViewKind(item.view.kind))
          setItems(shown.length === 0 ? NONE : shown)
        },
        () => {
          if (active && current === generation) setItems(NONE)
        },
      )
    }
    void pluginClient(override).then((client) => {
      if (!active) return
      load(client)
      stop = client.subscribe((event) => {
        if (event.type === "changed") load(client)
      })
    })
    return () => {
      active = false
      stop?.()
    }
  }, [override, reloadKey])
  return items
}

/** Icons already read, by `<plugin>/<entry>/<theme>`; `null`: none to show. */
const ICONS = new Map<string, string | null>()

/** Forgets the icons read so far (tests; a plugin update brings new ones). */
export function clearNavigationIcons() {
  ICONS.clear()
}

/** Only images the agent prepared: anything else shows the fallback icon. */
function safeImage(url: string | null): string | null {
  return url !== null && /^data:image\/(png|webp|svg\+xml);base64,/.test(url) ? url : null
}

/**
 * The entry's icon for the theme as a `data:` URL; `null` while it loads
 * and when there is none or it failed (the fallback icon is shown).
 */
export function useNavigationIcon(
  item: Pick<PluginNavigationItem, "plugin" | "id">,
  theme: NavigationIconTheme,
): string | null {
  const override = usePluginClientOverride()
  const key = `${navigationKey(item)}/${theme}`
  const [loaded, setLoaded] = useState<{ key: string; url: string | null } | null>(null)
  const { plugin, id } = item
  useEffect(() => {
    if (ICONS.has(key)) return
    let active = true
    void pluginClient(override)
      .then((client) => client.getNavigationIcon(plugin, id, theme))
      .then(
        (url) => safeImage(url),
        () => null,
      )
      .then((url) => {
        ICONS.set(key, url)
        if (active) setLoaded({ key, url })
      })
    return () => {
      active = false
    }
  }, [override, key, plugin, id, theme])
  if (ICONS.has(key)) return ICONS.get(key) ?? null
  return loaded?.key === key ? loaded.url : null
}

// ---------------------------------------------------------------------------
// For the views

export interface PluginNavigationState {
  items: PluginNavigationItem[]
  /** The page of the main window, to find the entry a view is for. */
  page: string
}

export const PluginNavigationContext = createContext<PluginNavigationState>({
  items: NONE,
  page: "",
})

/** The entry whose view the main window shows, if it still exists. */
export function useCurrentPluginView(): {
  item: PluginNavigationItem | undefined
  plugin: string | null
} {
  const { items, page } = useContext(PluginNavigationContext)
  const parsed = parsePluginViewPage(page)
  return {
    item: parsed
      ? items.find((i) => i.plugin === parsed.plugin && i.id === parsed.entry)
      : undefined,
    plugin: parsed?.plugin ?? null,
  }
}
