// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { useState } from "react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

import {
  AlertDialog,
  ConfirmDialog,
  DestructiveDialog,
  DialogProvider,
  type Dialogs,
  FormDialog,
  PermissionDialog,
  ProgressDialog,
  SelectionDialog,
  useDialogs,
} from "./index"

vi.mock("@/lib/api", () => ({
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}))

afterEach(cleanup)

function deferred() {
  let resolve: () => void = () => undefined
  let reject: (e: unknown) => void = () => undefined
  const promise = new Promise<void>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

describe("ConfirmDialog / DestructiveDialog", () => {
  it("opens from its trigger, cancels with Escape and returns the focus", async () => {
    const user = userEvent.setup()
    const onConfirm = vi.fn()
    render(
      <DestructiveDialog
        trigger={<Button>Delete profile</Button>}
        title="Delete “Work”?"
        description="This cannot be undone."
        onConfirm={onConfirm}
      />,
    )
    const trigger = screen.getByRole("button", { name: "Delete profile" })
    await user.click(trigger)
    const dialog = screen.getByRole("alertdialog", { name: "Delete “Work”?" })
    expect(dialog).toHaveAccessibleDescription("This cannot be undone.")
    expect(screen.getByRole("button", { name: "Delete" })).toHaveClass("bg-destructive")
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    await waitFor(() => {
      expect(trigger).toHaveFocus()
    })
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it("is not dismissed by a click outside", () => {
    render(
      <ConfirmDialog
        open
        title="Switch?"
        description="d"
        destructive={false}
        onConfirm={vi.fn()}
      />,
    )
    const dialog = screen.getByRole("alertdialog", { name: "Switch?" })
    fireEvent.pointerDown(document.body)
    expect(dialog).toBeInTheDocument()
  })

  it("stays open and busy while the action runs, submits once, closes on success", async () => {
    const user = userEvent.setup()
    const work = deferred()
    const onConfirm = vi.fn(() => work.promise)
    const onOpenChange = vi.fn()
    render(
      <ConfirmDialog
        open
        onOpenChange={onOpenChange}
        title="Delete?"
        description="d"
        busyLabel="Deleting…"
        onConfirm={onConfirm}
      />,
    )
    await user.click(screen.getByRole("button", { name: "Delete" }))
    const busy = screen.getByRole("button", { name: /Deleting/ })
    expect(busy).toBeDisabled()
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled()
    fireEvent.submit(busy.closest("form") as HTMLFormElement)
    expect(onConfirm).toHaveBeenCalledTimes(1)
    await act(async () => {
      work.resolve()
      await work.promise
    })
    await waitFor(() => {
      expect(onOpenChange).toHaveBeenCalledWith(false)
    })
  })

  it("shows a failure inline and stays open", async () => {
    const user = userEvent.setup()
    const onOpenChange = vi.fn()
    render(
      <ConfirmDialog
        open
        onOpenChange={onOpenChange}
        title="Delete?"
        description="d"
        onConfirm={() => Promise.reject(new Error("The profile is in use."))}
      />,
    )
    await user.click(screen.getByRole("button", { name: "Delete" }))
    expect(await screen.findByRole("alert")).toHaveTextContent("The profile is in use.")
    expect(screen.getByRole("alertdialog")).toBeInTheDocument()
    expect(onOpenChange).not.toHaveBeenCalledWith(false)
  })
})

describe("AlertDialog", () => {
  it("closes with its one button and with Enter", async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    render(<AlertDialog open onClose={onClose} tone="warning" title="Heads up" />)
    expect(screen.getByRole("alertdialog", { name: "Heads up" })).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "OK" }))
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})

describe("FormDialog", () => {
  function NameForm({ onSubmit }: { onSubmit: (name: string) => Promise<void> | void }) {
    const [open, setOpen] = useState(true)
    const [name, setName] = useState("")
    return (
      <FormDialog
        open={open}
        onClose={() => {
          setOpen(false)
        }}
        title="New layout"
        submitLabel="Create"
        canSubmit={name.trim().length > 0}
        dirty={name.length > 0}
        onSubmit={() => onSubmit(name)}
      >
        <div className="grid gap-2">
          <Label htmlFor="name">Name</Label>
          <Input
            id="name"
            autoFocus
            value={name}
            onChange={(e) => {
              setName(e.target.value)
            }}
          />
        </div>
      </FormDialog>
    )
  }

  it("submits with Enter only when valid, and closes after saving", async () => {
    const user = userEvent.setup()
    const onSubmit = vi.fn()
    render(<NameForm onSubmit={onSubmit} />)
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled()
    await user.type(screen.getByLabelText("Name"), "{Enter}")
    expect(onSubmit).not.toHaveBeenCalled()
    await user.type(screen.getByLabelText("Name"), "Coding{Enter}")
    expect(onSubmit).toHaveBeenCalledWith("Coding")
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "New layout" })).not.toBeInTheDocument()
    })
  })

  it("asks before discarding unsaved changes, and goes back to the form", async () => {
    const user = userEvent.setup()
    render(<NameForm onSubmit={vi.fn()} />)
    await user.type(screen.getByLabelText("Name"), "Draft")
    await user.click(screen.getByRole("button", { name: "Cancel" }))
    expect(screen.getByRole("alertdialog", { name: "Discard your changes?" })).toBeInTheDocument()
    await user.click(screen.getAllByRole("button", { name: "Cancel" }).at(-1) as HTMLElement)
    expect(screen.getByLabelText("Name")).toHaveValue("Draft")
    await user.click(screen.getByRole("button", { name: "Cancel" }))
    await user.click(screen.getByRole("button", { name: "Discard" }))
    expect(screen.queryByRole("dialog", { name: "New layout" })).not.toBeInTheDocument()
  })
})

describe("SelectionDialog", () => {
  it("chooses with the keyboard", async () => {
    const user = userEvent.setup()
    const onSelect = vi.fn()
    render(
      <SelectionDialog
        open
        onClose={vi.fn()}
        title="Move to"
        options={[
          { id: "a", label: "Main" },
          { id: "b", label: "Coding", description: "2 pages" },
        ]}
        submitLabel="Move"
        onSelect={onSelect}
      />,
    )
    expect(screen.getByRole("button", { name: "Move" })).toBeDisabled()
    await user.click(screen.getByRole("radio", { name: /Main/ }))
    await user.keyboard("{ArrowDown}")
    expect(screen.getByRole("radio", { name: /Coding/ })).toBeChecked()
    await user.keyboard("{Enter}")
    expect(onSelect).toHaveBeenCalledWith("b")
  })

  it("filters long lists", async () => {
    const user = userEvent.setup()
    const options = Array.from({ length: 12 }, (_, i) => ({ id: `${i}`, label: `Layout ${i}` }))
    render(
      <SelectionDialog open onClose={vi.fn()} title="Pick" options={options} onSelect={vi.fn()} />,
    )
    await user.type(screen.getByRole("searchbox", { name: "Filter" }), "11")
    expect(screen.getAllByRole("radio")).toHaveLength(1)
  })
})

describe("PermissionDialog", () => {
  it("always names who asks", () => {
    render(
      <PermissionDialog
        open
        onClose={vi.fn()}
        requester={{ name: "GitHub", detail: "by DeckPilot" }}
        title="Allow GitHub to…"
        onAllow={vi.fn()}
      >
        <p>Network access to api.github.com</p>
      </PermissionDialog>,
    )
    expect(screen.getByLabelText("Requested by")).toHaveTextContent("GitHub")
    expect(screen.getByRole("button", { name: "Allow" })).toBeEnabled()
  })
})

describe("ProgressDialog", () => {
  it("cannot be dismissed without a cancel action", async () => {
    const user = userEvent.setup()
    render(<ProgressDialog open title="Installing" value={40} status="2 of 5" />)
    await user.keyboard("{Escape}")
    expect(screen.getByRole("alertdialog", { name: "Installing" })).toBeInTheDocument()
    expect(screen.getByRole("progressbar", { name: "Installing" })).toBeInTheDocument()
  })

  it("cancels when it offers to", async () => {
    const user = userEvent.setup()
    const onCancel = vi.fn()
    render(<ProgressDialog open title="Importing" value={null} onCancel={onCancel} />)
    await user.click(screen.getByRole("button", { name: "Cancel" }))
    expect(onCancel).toHaveBeenCalled()
  })
})

describe("dialog service", () => {
  let captured: Dialogs | null = null
  function Capture() {
    captured = useDialogs()
    return null
  }
  function service(): Dialogs {
    if (!captured) throw new Error("DialogProvider not rendered")
    return captured
  }

  it("shows one dialog at a time, in order, and settles each request once", async () => {
    const user = userEvent.setup()
    render(
      <DialogProvider>
        <Capture />
      </DialogProvider>,
    )
    let first: Promise<boolean> = Promise.resolve(false)
    let second: Promise<string | null> = Promise.resolve(null)
    act(() => {
      first = service().destructive({ title: "Delete A?", description: "d" })
      second = service().select({ title: "Pick one", options: [{ id: "x", label: "X" }] })
    })
    expect(screen.getByRole("alertdialog", { name: "Delete A?" })).toBeInTheDocument()
    expect(screen.queryByRole("dialog", { name: "Pick one" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "Delete" }))
    await expect(first).resolves.toBe(true)
    expect(await screen.findByRole("dialog", { name: "Pick one" })).toBeInTheDocument()
    await user.keyboard("{Escape}")
    await expect(second).resolves.toBeNull()
  })

  it("runs onConfirm inside the dialog and resolves only after it succeeded", async () => {
    const user = userEvent.setup()
    render(
      <DialogProvider>
        <Capture />
      </DialogProvider>,
    )
    const onConfirm = vi
      .fn()
      .mockRejectedValueOnce(new Error("Still in use."))
      .mockResolvedValueOnce(undefined)
    let result: Promise<boolean> = Promise.resolve(false)
    act(() => {
      result = service().confirm({ title: "Switch?", description: "d", onConfirm })
    })
    await user.click(screen.getByRole("button", { name: "Continue" }))
    expect(await screen.findByRole("alert")).toHaveTextContent("Still in use.")
    await user.click(screen.getByRole("button", { name: "Continue" }))
    await expect(result).resolves.toBe(true)
    expect(onConfirm).toHaveBeenCalledTimes(2)
  })

  it("cancels open and queued requests when the provider goes away", async () => {
    const { unmount } = render(
      <DialogProvider>
        <Capture />
      </DialogProvider>,
    )
    let a: Promise<boolean> = Promise.resolve(true)
    let b: Promise<void> = Promise.resolve()
    act(() => {
      a = service().confirm({ title: "A", description: "d" })
      b = service().alert({ title: "B" })
    })
    unmount()
    await expect(a).resolves.toBe(false)
    await expect(b).resolves.toBeUndefined()
  })

  it("must be used inside the provider", () => {
    function Outside() {
      useDialogs()
      return null
    }
    vi.spyOn(console, "error").mockImplementation(() => undefined)
    expect(() => render(<Outside />)).toThrow(/DialogProvider/)
  })
})
