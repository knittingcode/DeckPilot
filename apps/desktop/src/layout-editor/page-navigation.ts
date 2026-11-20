// Where the layout editor is among the pages, for the page navigation under
// the device preview (page-nav.tsx).

import { ancestors, page as findPage, pagePosition, topLevelPages } from "@/lib/layout-model"
import { i18n } from "@/i18n"
import type { DeviceLayout } from "@/lib/types"

/**
 * The name of a new top-level page: "Page N" after the pages there are,
 * the first such name no page has (in the window's language: the name
 * is then the user's).
 */
export function newPageName(layout: DeviceLayout): string {
  const taken = new Set(layout.pages.map((p) => p.name.trim().toLowerCase()))
  for (let n = topLevelPages(layout).length + 1; ; n++) {
    const name = i18n.t("editor:pageNav.newPageName", { number: n })
    if (!taken.has(name.toLowerCase())) return name
  }
}

/**
 * Where the editor is among the top-level pages, which the device's "Next
 * page" and "Previous page" move along. A page inside a folder is where its
 * top-level page is. No wraparound: that is an option of the device's page
 * actions, not of the layout.
 */
export function pageNavigation(layout: DeviceLayout, pageId: string) {
  const pages = topLevelPages(layout)
  const { number, count } = pagePosition(layout, pageId)
  return {
    number,
    count,
    previous: number > 1 ? pages[number - 2] : undefined,
    next: number < pages.length ? pages[number] : undefined,
  }
}

/**
 * The path to `pageId`: how it was reached (`trail`), else where it is in
 * the page tree, starting at the home page.
 */
export function pagePath(layout: DeviceLayout, pageId: string, trail: string[]) {
  const route =
    trail.length > 0 ? [...trail, pageId] : [...ancestors(layout, pageId).reverse(), pageId]
  const path = route
    .map((id) => findPage(layout, id))
    .filter((p): p is NonNullable<typeof p> => p !== undefined)
  if (path[0]?.id !== layout.homePage && pageId !== layout.homePage) {
    const home = findPage(layout, layout.homePage)
    if (home) path.unshift(home)
  }
  return path
}
