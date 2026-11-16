// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { cleanup, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { ParameterField } from "@/components/parameter-field"
import { AppContext } from "@/lib/app-context"
import type { ParameterSpec, ParameterValue } from "@/lib/types"
import { installDomStubs } from "@/test/dom"
import { appContext } from "@/test/render"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)

beforeAll(installDomStubs)
afterEach(cleanup)

const profileSpec: ParameterSpec = {
  key: "profile",
  label: "Profile",
  kind: { type: "text", multiline: false, maxLength: 36 },
  required: false,
}

const profiles = {
  activeProfileId: "work",
  profiles: [
    { id: "work", name: "Work", updatedAt: 0 },
    { id: "home", name: "Home", updatedAt: 0 },
  ],
}

function Field({
  initial,
  actionType,
  onValue,
}: {
  initial: string
  actionType?: string
  onValue: (value: ParameterValue) => void
}) {
  const [value, setValue] = useState<ParameterValue>({ type: "text", value: initial })
  return (
    <AppContext.Provider value={appContext({ profiles })}>
      <ParameterField
        spec={profileSpec}
        value={value}
        actionType={actionType}
        onChange={(next) => {
          setValue(next)
          onValue(next)
        }}
      />
    </AppContext.Provider>
  )
}

describe("the profile of Switch profile", () => {
  it("lists the profiles that exist and stores the chosen one's id", async () => {
    const onValue = vi.fn()
    const user = userEvent.setup()
    render(<Field initial="" actionType="deck.switch-profile" onValue={onValue} />)
    const select = screen.getByRole("combobox", { name: "Profile" })
    expect(select).toHaveTextContent("No profile")
    await user.click(select)
    const options = within(screen.getByRole("listbox")).getAllByRole("option")
    expect(options.map((o) => o.textContent)).toEqual(["No profile", "Work", "Home"])
    await user.click(screen.getByRole("option", { name: "Home" }))
    expect(onValue).toHaveBeenLastCalledWith({ type: "text", value: "home" })
    expect(screen.getByRole("combobox", { name: "Profile" })).toHaveTextContent("Home")
  })

  it("keeps a profile that was deleted and says so", async () => {
    const onValue = vi.fn()
    const user = userEvent.setup()
    render(<Field initial="gone" actionType="deck.switch-profile" onValue={onValue} />)
    const select = screen.getByRole("combobox", { name: "Profile" })
    expect(select).toHaveTextContent("Deleted profile")
    expect(select).toHaveAttribute("aria-invalid", "true")
    expect(screen.getByRole("note")).toHaveTextContent("This profile no longer exists")
    // Nothing was changed by showing it.
    expect(onValue).not.toHaveBeenCalled()
    await user.click(select)
    await user.click(screen.getByRole("option", { name: "Work" }))
    expect(onValue).toHaveBeenLastCalledWith({ type: "text", value: "work" })
    expect(screen.queryByRole("note")).not.toBeInTheDocument()
  })

  it("is a plain text field for any other action", () => {
    render(<Field initial="work" actionType="general.log" onValue={vi.fn()} />)
    expect(screen.queryByRole("combobox", { name: "Profile" })).not.toBeInTheDocument()
    expect(screen.getByRole("textbox", { name: "Profile" })).toHaveValue("work")
  })
})
