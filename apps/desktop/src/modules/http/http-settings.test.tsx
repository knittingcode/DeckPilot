// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { AppContext, type AppContextValue } from "@/lib/app-context"
import { moduleInfo } from "@/lib/test-fixtures"
import type { CredentialInfo } from "@/lib/types"

import { CREDENTIAL_ID, HttpSettings } from "./http-settings"

const api = {
  listCredentials: vi.fn<() => Promise<CredentialInfo[]>>(),
  setCredential:
    vi.fn<(id: string, description: string, secret: string | null) => Promise<CredentialInfo[]>>(),
  deleteCredential: vi.fn<(id: string) => Promise<CredentialInfo[]>>(),
  setModuleConfig: vi.fn(),
}

vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: (_, name: string) => api[name as keyof typeof api] }),
  errorMessage: (error: unknown) => String(error),
}))
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }))

const http = moduleInfo({
  id: "http",
  name: "HTTP",
  config: [
    {
      key: "allowPrivateNetworks",
      label: "Allow this computer and private networks",
      kind: { type: "boolean" },
      required: true,
    },
  ],
})

function renderSettings() {
  const value = {
    modules: [http],
    setModules: vi.fn(),
    refresh: vi.fn(),
  } as unknown as AppContextValue
  return render(
    <AppContext.Provider value={value}>
      <HttpSettings />
    </AppContext.Provider>,
  )
}

describe("HTTP settings", () => {
  beforeEach(() => {
    api.listCredentials.mockResolvedValue([{ id: "github.token", description: "CI", updatedAt: 1 }])
    api.setCredential.mockImplementation((id, description) =>
      Promise.resolve([
        { id: "github.token", description: "CI", updatedAt: 1 },
        { id, description, updatedAt: 2 },
      ]),
    )
  })
  afterEach(() => {
    cleanup()
    vi.clearAllMocks()
  })

  it("shows the module settings and stored credentials without secrets", async () => {
    renderSettings()
    expect(screen.getByText("Allow this computer and private networks")).toBeInTheDocument()
    const list = await screen.findByRole("list", { name: "Stored credentials" })
    expect(within(list).getByText("github.token")).toBeInTheDocument()
    expect(within(list).getByText("CI")).toBeInTheDocument()
  })

  it("stores a new credential and clears the secret", async () => {
    const user = userEvent.setup()
    renderSettings()
    await screen.findByRole("list", { name: "Stored credentials" })
    const store = screen.getByRole("button", { name: "Store credential" })
    expect(store).toBeDisabled()
    await user.type(screen.getByLabelText("Name"), "Bad Name")
    expect(store).toBeDisabled()
    await user.clear(screen.getByLabelText("Name"))
    await user.type(screen.getByLabelText("Name"), "staging.api")
    await user.type(screen.getByLabelText("Description"), "Staging")
    await user.type(screen.getByLabelText("Secret"), "s3cret")
    expect(screen.getByLabelText("Secret")).toHaveAttribute("type", "password")
    await user.click(store)
    expect(api.setCredential).toHaveBeenCalledWith("staging.api", "Staging", "s3cret")
    await waitFor(() => {
      expect(screen.getByLabelText("Secret")).toHaveValue("")
    })
    expect(screen.getByText("staging.api")).toBeInTheDocument()
  })

  it("validates names like the agent", () => {
    for (const ok of ["github.token", "a", "api_key-2"]) expect(CREDENTIAL_ID.test(ok)).toBe(true)
    for (const bad of ["", "Token", "1a", "a b", "a/b"]) expect(CREDENTIAL_ID.test(bad)).toBe(false)
  })
})
