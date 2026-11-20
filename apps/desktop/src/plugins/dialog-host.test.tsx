// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import "@testing-library/jest-dom/vitest"
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest"

import { DialogProvider, useDialogs } from "@/components/dialogs"
import { installDomStubs } from "@/test/dom"
import { emitBackend, resetBackend } from "@/test/tauri"

import { PLUGIN_EVENT } from "./client"
import { PluginDialogHost } from "./dialog-host"
import { fieldValue } from "./dialog-values"
import type { PluginDialog, PluginDialogField, PluginDialogSpec } from "./types"

const invoke = vi.hoisted(() => vi.fn())
vi.mock("@tauri-apps/api/core", () => ({ invoke }))
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

beforeAll(() => {
  installDomStubs()
})

afterEach(() => {
  cleanup()
  invoke.mockReset()
  resetBackend()
})

function dialog(id: string, spec: Partial<PluginDialogSpec> & Pick<PluginDialogSpec, "type">) {
  return {
    id,
    plugin: "acme.repo",
    pluginName: "Repo tools",
    spec: { title: "Open the repository?", timeoutSeconds: 120, ...spec },
  } satisfies PluginDialog
}

async function mount(children?: React.ReactNode) {
  invoke.mockResolvedValue(null)
  render(
    <DialogProvider>
      {children}
      <PluginDialogHost />
    </DialogProvider>,
  )
  // Ready once it listens.
  await waitFor(() => {
    expect(invoke).toHaveBeenCalledWith("plugin_dialogs_ready", { ready: true })
  })
}

function open(d: PluginDialog) {
  act(() => {
    emitBackend(PLUGIN_EVENT, { type: "dialogOpened", dialog: d })
  })
}

function answers() {
  return invoke.mock.calls.filter(([command]) => command === "answer_plugin_dialog")
}

describe("the plugin dialog host", () => {
  it("shows a confirmation with the plugin's name and answers it", async () => {
    const user = userEvent.setup()
    await mount()
    open(
      dialog("d1", {
        type: "confirm",
        message: "<b>Not markup</b>",
        confirmLabel: "Open",
        destructive: true,
      }),
    )
    const shown = await screen.findByRole("alertdialog")
    expect(within(shown).getByText("Open the repository?")).toBeInTheDocument()
    // Plain text, never markup.
    expect(within(shown).getByText("<b>Not markup</b>")).toBeInTheDocument()
    expect(shown.querySelector("b")).toBeNull()
    const requester = shown.querySelector("[data-dialog-part=requester]")
    expect(requester).toHaveTextContent("Requested by Repo tools")
    await user.click(within(shown).getByRole("button", { name: "Open" }))
    await waitFor(() => {
      expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument()
    })
    expect(answers()).toEqual([
      [
        "answer_plugin_dialog",
        { dialog: "d1", plugin: "acme.repo", answer: { outcome: "confirmed" } },
      ],
    ])
  })

  it("answers cancelled when the user cancels, and stops announcing on unmount", async () => {
    const user = userEvent.setup()
    await mount()
    open(dialog("d2", { type: "confirm", cancelLabel: "Not now" }))
    const shown = await screen.findByRole("alertdialog")
    await user.click(within(shown).getByRole("button", { name: "Not now" }))
    await waitFor(() => {
      expect(answers()).toHaveLength(1)
    })
    expect(answers()[0]?.[1]).toEqual({
      dialog: "d2",
      plugin: "acme.repo",
      answer: { outcome: "cancelled", reason: "user" },
    })
    cleanup()
    expect(invoke).toHaveBeenCalledWith("plugin_dialogs_ready", { ready: false })
  })

  it("acknowledges an alert, and Escape cancels it", async () => {
    const user = userEvent.setup()
    await mount()
    open(dialog("a1", { type: "alert", title: "Build finished" }))
    await user.click(await screen.findByRole("button", { name: "OK" }))
    open(dialog("a2", { type: "alert", title: "Second" }))
    await screen.findByText("Second")
    await user.keyboard("{Escape}")
    await waitFor(() => {
      expect(answers()).toHaveLength(2)
    })
    expect(answers().map(([, args]) => (args as { answer: unknown }).answer)).toEqual([
      { outcome: "confirmed" },
      { outcome: "cancelled", reason: "user" },
    ])
  })

  it("answers a form with typed values", async () => {
    const user = userEvent.setup()
    await mount()
    open(
      dialog("f1", {
        type: "form",
        title: "Configure repository",
        confirmLabel: "Save",
        fields: [
          { id: "repository", kind: "text", label: "Repository path", required: true },
          { id: "depth", kind: "integer", label: "Depth", min: 1, max: 10, default: 3 },
          { id: "pull", kind: "boolean", label: "Pull first", default: true },
          {
            id: "branch",
            kind: "choice",
            label: "Branch",
            default: "main",
            options: [
              { value: "main", label: "main" },
              { value: "develop", label: "develop" },
            ],
          },
          { id: "remotes", kind: "lines", label: "Remotes" },
        ],
      }),
    )
    const shown = await screen.findByRole("dialog")
    expect(shown.querySelector("[data-dialog-part=requester]")).toHaveTextContent("Repo tools")
    const save = within(shown).getByRole("button", { name: "Save" })
    // The required path is empty.
    expect(save).toBeDisabled()
    await user.type(within(shown).getByLabelText(/Repository path/), "~/code/shop")
    const depth = within(shown).getByLabelText("Depth")
    await user.clear(depth)
    await user.type(depth, "12")
    expect(within(shown).getByText("At most 10.")).toBeInTheDocument()
    expect(save).toBeDisabled()
    await user.clear(depth)
    await user.type(depth, "5")
    await user.type(within(shown).getByLabelText("Remotes"), "origin\n\n upstream ")
    await user.click(within(shown).getByRole("switch"))
    await user.click(save)
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    })
    expect(answers()[0]?.[1]).toEqual({
      dialog: "f1",
      plugin: "acme.repo",
      answer: {
        outcome: "confirmed",
        values: {
          repository: "~/code/shop",
          depth: 5,
          pull: false,
          branch: "main",
          remotes: ["origin", "upstream"],
        },
      },
    })
  })

  it("keeps a form open with the agent's refusal", async () => {
    const user = userEvent.setup()
    await mount()
    invoke.mockImplementation((command: string) =>
      command === "answer_plugin_dialog"
        ? Promise.reject(
            Object.assign(new Error("The path does not exist."), { reason: "invalidAnswer" }),
          )
        : Promise.resolve(null),
    )
    open(
      dialog("f2", {
        type: "form",
        fields: [{ id: "path", kind: "text", label: "Path", default: "x" }],
      }),
    )
    const shown = await screen.findByRole("dialog")
    await user.click(within(shown).getByRole("button", { name: "OK" }))
    expect(await within(shown).findByText("The path does not exist.")).toBeInTheDocument()
    expect(screen.getByRole("dialog")).toBeInTheDocument()
  })

  it("refuses an answer too large for the agent before sending it", async () => {
    const user = userEvent.setup()
    await mount()
    const fields: PluginDialogField[] = Array.from({ length: 12 }, (_, i) => ({
      id: `f${i}`,
      kind: "text",
      label: `Field ${i}`,
      maxLength: 2000,
      // Three UTF-8 bytes each: 72,000 bytes in all.
      default: "€".repeat(2000),
    }))
    open(dialog("f3", { type: "form", fields }))
    const shown = await screen.findByRole("dialog")
    await user.click(within(shown).getByRole("button", { name: "OK" }))
    expect(
      await within(shown).findByText("The answer is too long. Shorten the text and try again."),
    ).toBeInTheDocument()
    expect(answers()).toHaveLength(0)
  })

  it("answers a selection with the chosen id", async () => {
    const user = userEvent.setup()
    await mount()
    open(
      dialog("s1", {
        type: "selection",
        title: "Choose a profile",
        options: [
          { id: "coding", label: "Coding" },
          { id: "meetings", label: "Meetings", description: "Calls and calendar" },
        ],
      }),
    )
    const shown = await screen.findByRole("dialog")
    expect(shown.querySelector("[data-dialog-part=requester]")).toHaveTextContent("Repo tools")
    await user.click(within(shown).getByLabelText(/Coding/))
    await user.click(within(shown).getByRole("button", { name: "Choose" }))
    await waitFor(() => {
      expect(answers()).toHaveLength(1)
    })
    expect(answers()[0]?.[1]).toEqual({
      dialog: "s1",
      plugin: "acme.repo",
      answer: { outcome: "confirmed", selected: "coding" },
    })
  })

  it("takes a dialog down when the agent closes it, open or queued, without answering", async () => {
    await mount()
    open(dialog("q1", { type: "confirm", title: "First" }))
    open(dialog("q2", { type: "confirm", title: "Second" }))
    open(dialog("q3", { type: "confirm", title: "Third" }))
    await screen.findByText("First")
    // One at a time.
    expect(screen.queryByText("Second")).not.toBeInTheDocument()
    act(() => {
      emitBackend(PLUGIN_EVENT, {
        type: "dialogClosed",
        dialog: "q2",
        plugin: "acme.repo",
        reason: "timeout",
      })
      emitBackend(PLUGIN_EVENT, {
        type: "dialogClosed",
        dialog: "q1",
        plugin: "acme.repo",
        reason: "pluginStopped",
      })
    })
    expect(await screen.findByText("Third")).toBeInTheDocument()
    expect(screen.queryByText("First")).not.toBeInTheDocument()
    expect(screen.queryByText("Second")).not.toBeInTheDocument()
    expect(answers()).toHaveLength(0)
  })

  it("waits behind DeckPilot's own dialogs", async () => {
    const user = userEvent.setup()
    let ask: ReturnType<typeof useDialogs> | undefined
    function Asker() {
      ask = useDialogs()
      return null
    }
    await mount(<Asker />)
    act(() => {
      void ask?.alert({ title: "DeckPilot asks first" })
    })
    await screen.findByText("DeckPilot asks first")
    open(dialog("w1", { type: "confirm", title: "Plugin asks" }))
    expect(screen.queryByText("Plugin asks")).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "OK" }))
    expect(await screen.findByText("Plugin asks")).toBeInTheDocument()
  })
})

describe("plugin dialog field values", () => {
  const text: PluginDialogField = { id: "t", kind: "text", label: "T", maxLength: 3 }
  it.each([
    [text, "abc", { value: "abc" }],
    [text, "abcd", { error: "At most 3 characters." }],
    [text, "  ", {}],
    [{ ...text, required: true }, "", { missing: true }],
    [{ id: "i", kind: "integer", label: "I" }, "1.5", { error: "Enter a whole number." }],
    [{ id: "i", kind: "integer", label: "I", min: 2 }, "1", { error: "At least 2." }],
    [{ id: "i", kind: "integer", label: "I" }, "-4", { value: -4 }],
    [{ id: "b", kind: "boolean", label: "B" }, undefined, { value: false }],
    [
      { id: "c", kind: "choice", label: "C", options: [{ value: "a", label: "A" }] },
      "z",
      { error: "Choose one of the options." },
    ],
    [
      { id: "l", kind: "lines", label: "L" },
      "a".repeat(201),
      { error: "Each line has at most 200 characters." },
    ],
    [
      { id: "l", kind: "lines", label: "L" },
      Array.from({ length: 101 }, (_, i) => `x${i}`).join("\n"),
      { error: "At most 100 lines." },
    ],
  ] satisfies [PluginDialogField, string | undefined, object][])(
    "%o with %o",
    (field, draft, expected) => {
      expect(fieldValue(field, draft)).toEqual(expected)
    },
  )
})
