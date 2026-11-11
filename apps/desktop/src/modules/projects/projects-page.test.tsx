// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import type { ResourceCheck } from "@/lib/types"
import { ProjectsPage } from "@/modules/projects/projects-page"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { profileView, project } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { resetToasts, toast } from "@/test/toast"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)

const shop = project("SHOP", {
  name: "Online Shop",
  resources: { repository: "shop.repository", docs: "shop.docs" },
})
const cmr = project("CMR", { name: "Customer Relations" })

const checks: ResourceCheck[] = [
  {
    key: "shop.repository",
    kind: "directory",
    value: "/src/shop",
    status: "ok",
    usedBy: ["SHOP"],
  },
  { key: "shop.docs", kind: "url", value: "https://docs", status: "missing", usedBy: ["SHOP"] },
]

beforeAll(installDomStubs)
beforeEach(() => {
  resetApi()
  resetToasts()
})
afterEach(cleanup)

async function renderProjects(projects = [shop, cmr]) {
  api.getProfile.mockResolvedValue(profileView({ projects }, { activeProjectId: shop.id }))
  api.checkResources.mockResolvedValue(checks)
  const rendered = renderWithApp(<ProjectsPage />)
  await screen.findByRole("tab", { name: "Projects" })
  return rendered
}

describe("Projects page", () => {
  it("creates a project with resource roles", async () => {
    api.createProject.mockResolvedValue(project("API"))
    const { user, titlebarAction, context } = await renderProjects()
    titlebarAction("projects.new")
    const dialog = await screen.findByRole("dialog", { name: "New project" })
    // The key field takes focus; saving needs a key.
    expect(within(dialog).getByLabelText("Key")).toHaveFocus()
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeDisabled()

    await user.type(within(dialog).getByLabelText("Key"), "API")
    await user.type(within(dialog).getByLabelText("Name"), "Public API")
    await user.click(within(dialog).getByRole("button", { name: "Add resource" }))
    await user.type(within(dialog).getByRole("textbox", { name: "Role" }), " repository ")
    await user.type(within(dialog).getByRole("textbox", { name: "Resource key" }), "api.repo")
    await user.click(within(dialog).getByRole("button", { name: "Save" }))

    expect(api.createProject).toHaveBeenCalledWith("profile-1", {
      key: "API",
      name: "Public API",
      resources: { repository: "api.repo" },
    })
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(toast.success).toHaveBeenCalledWith("Project saved")
    expect(context.refresh).toHaveBeenCalled()
    expect(api.getProfile).toHaveBeenCalledTimes(2)
  })

  it("refuses half-filled and repeated resource roles", async () => {
    const { user, titlebarAction } = await renderProjects()
    titlebarAction("projects.new")
    const dialog = await screen.findByRole("dialog", { name: "New project" })
    await user.type(within(dialog).getByLabelText("Key"), "API")
    await user.click(within(dialog).getByRole("button", { name: "Add resource" }))
    await user.type(within(dialog).getByRole("textbox", { name: "Resource key" }), "api.repo")
    await user.click(within(dialog).getByRole("button", { name: "Save" }))
    expect(within(dialog).getByText("Each resource needs a role and a resource key.")).toBeVisible()

    await user.type(within(dialog).getByRole("textbox", { name: "Role" }), "repository")
    await user.click(within(dialog).getByRole("button", { name: "Add resource" }))
    const roles = within(dialog).getAllByRole("textbox", { name: "Role" })
    const keys = within(dialog).getAllByRole("textbox", { name: "Resource key" })
    await user.type(roles[1] as HTMLElement, "repository")
    await user.type(keys[1] as HTMLElement, "api.other")
    await user.click(within(dialog).getByRole("button", { name: "Save" }))
    expect(within(dialog).getByText("The role “repository” is used twice.")).toBeVisible()
    expect(api.createProject).not.toHaveBeenCalled()

    // Removing the extra row makes the form valid again.
    await user.click(
      within(dialog).getAllByRole("button", { name: "Remove resource" })[1] as HTMLElement,
    )
    await user.click(within(dialog).getByRole("button", { name: "Save" }))
    expect(api.createProject).toHaveBeenCalledWith("profile-1", {
      key: "API",
      name: "",
      resources: { repository: "api.repo" },
    })
  })

  it("shows each project's resources with their state on this machine", async () => {
    await renderProjects()
    const item = screen.getByText("Online Shop").closest("li") as HTMLElement
    expect(within(item).getByText("Active")).toBeInTheDocument()
    expect(within(item).getByText("(Found)")).toBeInTheDocument()
    expect(within(item).getByText("(Missing)")).toBeInTheDocument()
    expect(within(item).getByLabelText("Missing")).toBeInTheDocument()
  })

  it("edits a project and removes a resource", async () => {
    const { user } = await renderProjects()
    await user.click(screen.getByRole("button", { name: "Edit SHOP" }))
    const dialog = await screen.findByRole("dialog", { name: "Edit SHOP" })
    expect(within(dialog).getByLabelText("Name")).toHaveValue("Online Shop")
    expect(within(dialog).getAllByRole("textbox", { name: "Role" })).toHaveLength(2)
    await user.click(
      within(dialog).getAllByRole("button", { name: "Remove resource" })[1] as HTMLElement,
    )
    await user.click(within(dialog).getByRole("button", { name: "Save" }))
    expect(api.updateProject).toHaveBeenCalledWith("profile-1", shop.id, {
      key: "SHOP",
      name: "Online Shop",
      resources: { repository: "shop.repository" },
    })
  })

  it("switches, sets the default and deletes with a confirmation", async () => {
    const { user } = await renderProjects()
    await user.click(screen.getByRole("radio", { name: "CMR" }))
    expect(api.selectProject).toHaveBeenCalledWith("profile-1", cmr.id)

    await user.click(screen.getByRole("combobox", { name: "Default project" }))
    await user.click(screen.getByRole("option", { name: "CMR · Customer Relations" }))
    expect(api.setDefaultProject).toHaveBeenCalledWith("profile-1", cmr.id)

    await user.click(screen.getByRole("button", { name: "Delete CMR" }))
    const confirm = screen.getByRole("alertdialog", { name: "Delete CMR?" })
    await user.click(within(confirm).getByRole("button", { name: "Delete" }))
    expect(api.deleteProject).toHaveBeenCalledWith("profile-1", cmr.id)
  })

  it("has empty states for projects and machine resources", async () => {
    api.checkResources.mockResolvedValue([])
    api.getProfile.mockResolvedValue(profileView())
    const { user } = renderWithApp(<ProjectsPage />)
    expect(await screen.findByText("No projects in this profile")).toBeInTheDocument()
    await user.click(screen.getByRole("tab", { name: "This machine" }))
    expect(await screen.findByText("No resources used or mapped yet")).toBeInTheDocument()
  })
})
