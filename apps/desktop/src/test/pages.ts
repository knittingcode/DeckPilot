// The layout editor's page navigation (layout-editor/page-nav.tsx) as tests
// use it: the page number that unfolds the Page Navigator (the page tree),
// the name of the page shown, and the tree's rows and commands.

import { screen, within } from "@testing-library/react"
import type { UserEvent } from "@testing-library/user-event"

/** The page number button, which opens the menu of every page. */
export const pageNumber = () => screen.getByRole("button", { name: /^Page \d+ of \d+: / })

/** The name of the page the editor shows: in the page number's name ("Page 1 of 3: Home"). */
export function currentPage(): string | null | undefined {
  const label = document
    .querySelector("[data-slot=page-nav] [role=toolbar] [aria-label^='Page ']")
    ?.getAttribute("aria-label")
  return label?.replace(/^Page \d+ of \d+: /, "")
}

/** The Page Navigator: the panel the page counter unfolds under the bar (opened if it is not). */
export async function managePages(user: UserEvent): Promise<HTMLElement> {
  const open = screen.queryByRole("region", { name: "Pages" })
  if (open) return open
  await user.click(pageNumber())
  return screen.findByRole("region", { name: "Pages" })
}

/** Folds the Page Navigator: Escape in it. */
export async function closePages(user: UserEvent): Promise<void> {
  await user.keyboard("{Escape}")
}

/** The rows of the Page Navigator's tree, in tree order. */
export const treeRows = (navigator: HTMLElement) =>
  within(within(navigator).getByRole("tree", { name: "Page tree" })).getAllByRole("treeitem")

/** Every page of the layout as the Page Navigator lists them (tree order). */
export async function pageNames(user: UserEvent): Promise<string[]> {
  const navigator = await managePages(user)
  const names = treeRows(navigator).map((row) => row.getAttribute("aria-label"))
  await closePages(user)
  return names as string[]
}

/** Shows the page `name` from the Page Navigator (the `index`th of that name). */
export async function goToPage(user: UserEvent, name: string, index = 0): Promise<void> {
  const navigator = await managePages(user)
  const item = treeRows(navigator).filter((row) => row.getAttribute("aria-label") === name)[index]
  if (!item) throw new Error(`no page “${name}” in the Page Navigator`)
  await user.click(item)
}

/** Runs a command of a page's row in the Page Navigator (its "⋯" menu). */
export async function pageCommand(user: UserEvent, page: string, command: RegExp | string) {
  const navigator = await managePages(user)
  await user.click(within(navigator).getByRole("button", { name: `Actions for “${page}”` }))
  const menu = await screen.findByRole("menu")
  await user.click(within(menu).getByRole("menuitem", { name: command }))
}

/** Renames `page` in the Page Navigator. */
export async function renamePage(user: UserEvent, page: string, name: string) {
  await pageCommand(user, page, "Rename")
  const field = await screen.findByRole("textbox", { name: "Page name" })
  await user.clear(field)
  await user.type(field, `${name}{Enter}`)
  await closePages(user)
}

/** The breadcrumbs of the page navigation (where the editor is, from the top). */
export const breadcrumbs = () => screen.getByRole("navigation", { name: "Breadcrumb" })

/** The names in the breadcrumbs, the current page last. */
export const crumbNames = () =>
  within(breadcrumbs())
    .getAllByRole("listitem")
    .map((li) => li.textContent)

/** Goes up to the level `name` of the breadcrumbs (where Back used to go). */
export async function upTo(user: UserEvent, name: string): Promise<void> {
  await user.click(within(breadcrumbs()).getByRole("button", { name }))
}
