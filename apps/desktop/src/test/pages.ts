// The layout editor's page navigation (layout-editor/page-nav.tsx) as tests
// use it: the menu of every page behind the page number, the name of the
// page shown, and the pages panel with the page settings and the tree.

import { screen, within } from "@testing-library/react"
import type { UserEvent } from "@testing-library/user-event"

/** The page number button, which opens the menu of every page. */
export const pageNumber = () => screen.getByRole("button", { name: /^Page \d+ of \d+$/ })

/** The name of the page the editor shows. */
export function currentPage(): string | null | undefined {
  return document.querySelector("[data-slot=page-nav] [data-slot=page-name]")?.textContent
}

/** Every page of the layout, in its order, as the page menu lists them. */
export async function pageNames(user: UserEvent): Promise<string[]> {
  await user.click(pageNumber())
  const menu = await screen.findByRole("menu")
  const names = within(menu)
    .getAllByRole("menuitemradio")
    .map((item) => item.textContent)
  await user.keyboard("{Escape}")
  return names
}

/** Shows the page `name` from the page menu (the `index`th of that name). */
export async function goToPage(user: UserEvent, name: string, index = 0): Promise<void> {
  await user.click(pageNumber())
  const menu = await screen.findByRole("menu")
  const item = within(menu).getAllByRole("menuitemradio", { name })[index]
  if (!item) throw new Error(`no page “${name}” in the page menu`)
  await user.click(item)
}

/** Opens the pages panel (page settings and the page tree) and returns it. */
export async function managePages(user: UserEvent): Promise<HTMLElement> {
  await user.click(screen.getByRole("button", { name: "Manage pages" }))
  return screen.findByRole("dialog", { name: "Pages" })
}

/** Closes the pages panel. */
export async function closePages(user: UserEvent): Promise<void> {
  await user.keyboard("{Escape}")
}
