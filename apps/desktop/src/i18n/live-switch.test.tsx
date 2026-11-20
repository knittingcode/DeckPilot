// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { FormDialog } from "@/components/dialogs/templates"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

import { i18n, useTranslation } from "./index"
import { setLanguagePreference } from "./language"

vi.mock("@/lib/api", () => ({ errorMessage: (error: unknown) => String(error) }))

afterEach(async () => {
  cleanup()
  localStorage.clear()
  await act(async () => {
    await i18n.changeLanguage("en")
  })
})

let mounts = 0

/** A form whose field and state belong to the caller, as in the app. */
function NameForm() {
  const { t } = useTranslation()
  const [name, setName] = useState("")
  const [open, setOpen] = useState(true)
  const [mounted] = useState(() => ++mounts)
  return (
    <FormDialog
      open={open}
      onClose={() => {
        setOpen(false)
      }}
      title={`${t("common:actions.continue")} #${mounted}`}
      submitLabel={t("common:actions.continue")}
      dirty={name !== ""}
      onSubmit={() => undefined}
    >
      <Label htmlFor="name">Name</Label>
      <Input
        id="name"
        value={name}
        onChange={(e) => {
          setName(e.target.value)
        }}
      />
    </FormDialog>
  )
}

describe("switching the language", () => {
  it("keeps the open dialog and what was typed, and changes the labels", async () => {
    const user = userEvent.setup()
    mounts = 0
    render(<NameForm />)
    await user.type(screen.getByLabelText("Name"), "Biuro – układ główny")
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument()
    expect(screen.getByRole("dialog", { name: "Continue #1" })).toBeInTheDocument()

    await act(async () => {
      await setLanguagePreference("pl")
    })

    // The same dialog (not mounted again), the same text, new labels.
    expect(screen.getByRole("dialog", { name: "Kontynuuj #1" })).toBeInTheDocument()
    expect(screen.getByLabelText("Name")).toHaveValue("Biuro – układ główny")
    expect(screen.getByRole("button", { name: "Anuluj" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Kontynuuj" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument()
    expect(mounts).toBe(1)
    expect(document.documentElement.lang).toBe("pl")

    // The nested discard confirmation opens in the new language.
    await user.click(screen.getByRole("button", { name: "Anuluj" }))
    expect(await screen.findByRole("alertdialog", { name: "Odrzucić zmiany?" })).toBeInTheDocument()
    expect(screen.getByLabelText("Name")).toHaveValue("Biuro – układ główny")

    await act(async () => {
      await setLanguagePreference("en")
    })
    expect(screen.getByRole("alertdialog", { name: "Discard your changes?" })).toBeInTheDocument()
  })
})
