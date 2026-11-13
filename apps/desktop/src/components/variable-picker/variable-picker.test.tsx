// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest"

import { act, cleanup, screen, waitFor, within } from "@testing-library/react"
import { useState } from "react"
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest"

import { ParameterField } from "@/components/parameter-field"
import { describeValue } from "@/lib/parameters"
import type { ParameterSpec, ParameterValue, ReferencePreview } from "@/lib/types"
import { VARIABLES_EVENT, stepOutputEntries } from "@/lib/variables"
import { api, resetApi } from "@/test/api-mock"
import { installDomStubs } from "@/test/dom"
import { agentConnection } from "@/test/fixtures"
import { renderWithApp } from "@/test/render"
import { emitBackend, resetBackend } from "@/test/tauri"
import { SECRET, fakeVariableAgent } from "@/test/variables"

import { VariableScopeProvider } from "./scope-provider"

vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)

beforeAll(installDomStubs)

let agent: ReturnType<typeof fakeVariableAgent>
beforeEach(() => {
  agent = fakeVariableAgent().install(api)
})
afterEach(() => {
  cleanup()
  resetApi()
  resetBackend()
})

const message: ParameterSpec = {
  key: "message",
  label: "Message",
  kind: { type: "text", multiline: false },
  required: true,
}
const retries: ParameterSpec = {
  key: "retries",
  label: "Retries",
  kind: { type: "integer", min: 0 },
  required: false,
}

const live = { agent: agentConnection({ capabilities: ["variables"] }) }

/** A parameter field that keeps its value, like the editors do. */
function Field({
  spec,
  initial,
  onChange,
}: {
  spec: ParameterSpec
  initial: ParameterValue
  onChange?: (value: ParameterValue) => void
}) {
  const [value, setValue] = useState(initial)
  return (
    <ParameterField
      spec={spec}
      value={value}
      onChange={(next) => {
        setValue(next)
        onChange?.(next)
      }}
    />
  )
}

function renderField(
  spec: ParameterSpec,
  initial: ParameterValue,
  onChange = vi.fn<(value: ParameterValue) => void>(),
  overrides: Parameters<typeof renderWithApp>[1] = live,
) {
  const rendered = renderWithApp(
    <Field spec={spec} initial={initial} onChange={onChange} />,
    overrides,
  )
  return { ...rendered, onChange }
}

const text = (value: string): ParameterValue => ({ type: "text", value })
const options = () => screen.getAllByRole("option")
const optionLabels = () => options().map((o) => o.textContent)

async function openPicker(user: ReturnType<typeof renderWithApp>["user"], name: RegExp) {
  await user.click(screen.getByRole("button", { name }))
  return screen.findByRole("combobox", { name: "Search variables" })
}

describe("the picker", () => {
  it("is hidden while the agent cannot resolve variables", () => {
    renderField(message, text(""), vi.fn(), { agent: agentConnection() })
    expect(screen.queryByRole("button", { name: /insert a variable/i })).toBeNull()
    expect(screen.getByLabelText(/^Message/)).not.toHaveAttribute("role", "combobox")
  })

  it("searches by name, description, scope and module, debounced", async () => {
    const { user } = renderField(message, text(""))
    const search = await openPicker(user, /insert a variable into message/i)
    await waitFor(() => {
      expect(options().length).toBeGreaterThan(3)
    })
    await user.type(search, "checkouts")
    await waitFor(() => {
      expect(optionLabels()).toEqual([expect.stringContaining("Root folder")])
    })
    const queries = api.listVariables.mock.calls.map(([, filter]) => filter.query)
    // One request for the opened list and one for the settled search, none per key.
    expect(queries).toEqual(["", "checkouts"])
    await user.clear(search)
    await user.type(search, "projects")
    await waitFor(() => {
      expect(optionLabels()).toEqual([expect.stringContaining("Favorite project")])
    })
  })

  it("only lists variables whose type fits the field", async () => {
    const { user } = renderField(retries, { type: "integer", value: 0 })
    await openPicker(user, /use a variable for retries/i)
    await waitFor(() => {
      expect(optionLabels()).toEqual([expect.stringContaining("Retries")])
    })
    expect(api.listVariables).toHaveBeenCalledWith(
      expect.objectContaining({ profile: "profile-1", projectSource: "application" }),
      expect.objectContaining({ compatibleWith: retries.kind }),
    )
  })

  it("groups entries and has listbox semantics and accessible names", async () => {
    const { user } = renderField(message, text(""))
    const search = await openPicker(user, /insert a variable into message/i)
    expect(screen.getByRole("dialog", { name: "Variables for Message" })).toBeInTheDocument()
    const listbox = await screen.findByRole("listbox", { name: "Variables" })
    expect(search).toHaveAttribute("aria-controls", listbox.id)
    await waitFor(() => {
      expect(within(listbox).getByRole("group", { name: "Shared" })).toBeInTheDocument()
    })
    for (const name of ["System", "Profile", "Project", "Workflow"]) {
      expect(within(listbox).getByRole("group", { name })).toBeInTheDocument()
    }
    const sensitive = within(listbox).getByRole("option", { name: /deploy token/i })
    expect(within(sensitive).getByLabelText("sensitive")).toBeInTheDocument()
  })

  it("is driven by the keyboard", async () => {
    const onChange = vi.fn()
    const { user } = renderField(message, text(""), onChange)
    const search = await openPicker(user, /insert a variable into message/i)
    await waitFor(() => {
      expect(options().length).toBeGreaterThan(3)
    })
    const selected = () => options().find((o) => o.getAttribute("aria-selected") === "true")
    expect(selected()).toHaveTextContent("Root folder")
    expect(search).toHaveAttribute("aria-activedescendant", selected()?.id)
    await user.keyboard("{ArrowDown}")
    expect(selected()).toHaveTextContent("Retries")
    expect(search).toHaveAttribute("aria-activedescendant", selected()?.id)
    await user.keyboard("{End}")
    expect(selected()).toBe(options().at(-1))
    await user.keyboard("{Home}{ArrowDown}{Enter}")
    expect(onChange).toHaveBeenLastCalledWith({
      type: "template",
      value: "{{ shared.limits.retries }}",
    })
    expect(screen.queryByRole("dialog")).toBeNull()
    // Escape closes without changing anything.
    await openPicker(user, /insert a variable into message/i)
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("dialog")).toBeNull()
    expect(onChange).toHaveBeenCalledTimes(1)
  })
})

describe("inserting", () => {
  it("opens an autocomplete on {{ and inserts the chosen reference", async () => {
    const onChange = vi.fn()
    const { user } = renderField(message, text(""), onChange)
    const input = screen.getByRole("combobox", { name: /^Message/ })
    expect(input).toHaveAttribute("aria-expanded", "false")
    await user.type(input, "cd {{{{root")
    expect(input).toHaveAttribute("aria-expanded", "true")
    await waitFor(() => {
      expect(optionLabels()).toEqual([expect.stringContaining("Root folder")])
    })
    await user.keyboard("{Enter}")
    expect(input).toHaveValue("cd {{ shared.paths.root }}")
    expect(onChange).toHaveBeenLastCalledWith({
      type: "template",
      value: "cd {{ shared.paths.root }}",
    })
    expect(input).toHaveAttribute("aria-expanded", "false")
    // The cursor is after the reference: typing goes on from there.
    await user.type(input, "/web")
    expect(input).toHaveValue("cd {{ shared.paths.root }}/web")
  })

  it("closes the autocomplete on Escape and keeps {{ as literal text", async () => {
    const onChange = vi.fn()
    const { user } = renderField(message, text(""), onChange)
    const input = screen.getByRole("combobox", { name: /^Message/ })
    await user.type(input, "{{{{")
    await screen.findByRole("listbox")
    await user.keyboard("{Escape}")
    expect(screen.queryByRole("listbox")).toBeNull()
    expect(onChange).toHaveBeenLastCalledWith({ type: "text", value: "{{" })
    expect(screen.getByText(/in plain text stays as typed/)).toBeInTheDocument()
  })

  it("inserts from the button at the cursor", async () => {
    const onChange = vi.fn()
    const { user } = renderField(message, text("hello world"), onChange)
    const input = screen.getByRole<HTMLInputElement>("combobox", { name: /^Message/ })
    input.setSelectionRange(6, 6)
    await openPicker(user, /insert a variable into message/i)
    await user.click(await screen.findByRole("option", { name: /root folder/i }))
    expect(onChange).toHaveBeenLastCalledWith({
      type: "template",
      value: "hello {{ shared.paths.root }}world",
    })
    expect(input).toHaveFocus()
    expect(input.selectionStart).toBe("hello {{ shared.paths.root }}".length)
  })

  it("saves a whole-value reference, never the previewed value", async () => {
    const onChange = vi.fn()
    const { user } = renderField(retries, { type: "integer", value: 0 }, onChange)
    await openPicker(user, /use a variable for retries/i)
    const option = await screen.findByRole("option", { name: /retries/i })
    await waitFor(() => {
      expect(screen.getByLabelText("Current value")).toHaveTextContent("3")
    })
    await user.click(option)
    const saved = { type: "reference", value: "shared.limits.retries" }
    expect(onChange).toHaveBeenLastCalledWith(saved)
    expect(describeValue(saved as ParameterValue)).toBe("{{ shared.limits.retries }}")
    expect(screen.getByLabelText("Reference")).toHaveTextContent("{{ shared.limits.retries }}")
    expect(await screen.findByText(/is now/)).toHaveTextContent("3")
    await user.click(screen.getByRole("button", { name: "Use a fixed value" }))
    expect(onChange).toHaveBeenLastCalledWith({ type: "integer", value: 0 })
  })
})

describe("live values", () => {
  const reference = (value: string): ParameterValue => ({ type: "reference", value })

  it("refreshes when the agent reports a change, without polling", async () => {
    renderField(message, { type: "template", value: "cd {{ shared.paths.root }}" })
    expect(await screen.findByText(/is now/)).toHaveTextContent("/src")
    const calls = api.previewReferences.mock.calls.length
    await new Promise((resolve) => setTimeout(resolve, 300))
    expect(api.previewReferences.mock.calls.length).toBe(calls)

    agent.set("shared.paths.root", { type: "string", value: "/work" })
    act(() => {
      emitBackend(VARIABLES_EVENT, { type: "sharedVariablesChanged", keys: ["paths.root"] })
    })
    await waitFor(() => {
      expect(screen.getByText(/is now/)).toHaveTextContent("/work")
    })
  })

  it("drops a preview that arrives after a newer one", async () => {
    const pending: ((previews: ReferencePreview[]) => void)[] = []
    api.previewReferences.mockImplementation(() => new Promise((resolve) => pending.push(resolve)))
    const { user } = renderField(message, text(""))
    await openPicker(user, /insert a variable into message/i)
    await waitFor(() => {
      expect(pending).toHaveLength(1)
    })
    await user.keyboard("{ArrowDown}")
    await waitFor(() => {
      expect(pending).toHaveLength(2)
    })
    const [first, second] = pending
    await act(() => {
      second?.([
        {
          reference: "shared.limits.retries",
          status: {
            status: "resolved",
            value: { type: "integer", value: 3 },
            source: { type: "default" },
          },
        },
      ])
      return Promise.resolve()
    })
    await act(() => {
      first?.([
        {
          reference: "shared.paths.root",
          status: {
            status: "resolved",
            value: { type: "string", value: "/stale" },
            source: { type: "default" },
          },
        },
      ])
      return Promise.resolve()
    })
    const value = screen.getByLabelText("Current value")
    expect(value).toHaveTextContent("3")
    expect(screen.queryByText("/stale")).toBeNull()
  })

  it("shows run-time only values and step outputs as unavailable until the run", async () => {
    const { user } = renderWithApp(
      <VariableScopeProvider
        context={{ workflow: "deploy", projectSource: "execution" }}
        stepOutputs={stepOutputEntries([
          { node: "build", output: "path", label: "Build · path", kind: "path" },
        ])}
      >
        <Field spec={message} initial={text("")} />
      </VariableScopeProvider>,
      live,
    )
    const search = await openPicker(user, /insert a variable into message/i)
    await user.type(search, "run")
    await waitFor(() => {
      expect(optionLabels()).toEqual([expect.stringContaining("Run id")])
    })
    await waitFor(() => {
      expect(screen.getByLabelText("Current value")).toHaveTextContent("Available when it runs")
    })
    expect(screen.getByText("Only while the workflow runs")).toBeInTheDocument()

    await user.clear(search)
    await user.type(search, "build")
    await waitFor(() => {
      expect(optionLabels()).toEqual([expect.stringContaining("Build · path")])
    })
    expect(screen.getByRole("group", { name: "Step outputs" })).toBeInTheDocument()
    expect(screen.getByLabelText("Current value")).toHaveTextContent("Available when it runs")
    const asked = api.previewReferences.mock.calls.flatMap(([, items]) =>
      items.map((i) => i.reference),
    )
    expect(asked).not.toContain("steps.build.output.path")
  })

  it("labels the context and keeps a run's project apart from the active one", async () => {
    renderWithApp(
      <>
        <VariableScopeProvider context={{ project: "project-1" }} names={{ project: "SHOP" }}>
          <Field
            spec={message}
            initial={{ type: "template", value: "{{ system.project.current.key }}" }}
          />
        </VariableScopeProvider>
        <VariableScopeProvider
          context={{ workflow: "deploy", projectSource: "execution" }}
          names={{ workflow: "Deploy" }}
        >
          <Field
            spec={{ ...message, key: "note", label: "Note" }}
            initial={{ type: "template", value: "{{ system.project.current.key }}" }}
          />
        </VariableScopeProvider>
      </>,
      live,
    )
    const [app, run] = screen.getAllByRole("list", { name: "References" })
    await waitFor(() => {
      expect(app).toHaveTextContent("is now SHOP")
    })
    expect(app).toHaveTextContent("Values for Profile Work · active project SHOP")
    await waitFor(() => {
      expect(run).toHaveTextContent("Available when it runs")
    })
    expect(run).toHaveTextContent("the project of the run (known when it runs) · workflow Deploy")
    expect(run).not.toHaveTextContent("SHOP")
    expect(api.previewReferences).toHaveBeenCalledWith(
      expect.objectContaining({ projectSource: "execution", workflow: "deploy" }),
      expect.anything(),
    )

    // The active project changes: only the application context asks again.
    const before = api.previewReferences.mock.calls.length
    act(() => {
      emitBackend(VARIABLES_EVENT, {
        type: "projectChanged",
        profile: "profile-1",
        project: "project-2",
        source: "application",
      })
    })
    await waitFor(() => {
      expect(api.previewReferences.mock.calls.length).toBe(before + 1)
    })
    expect(api.previewReferences.mock.calls.at(-1)?.[0]).toMatchObject({
      projectSource: "application",
    })
  })

  it("explains missing, mismatched, denied and invalid references", async () => {
    agent.variables.push({
      entry: {
        reference: "shared.loop.other",
        label: "Other",
        group: "shared",
        valueType: { type: "string" },
        availability: "available",
      },
      resolve: () => ({ status: "invalid", message: "circular reference: shared.loop.other" }),
    })
    renderField(message, {
      type: "template",
      value:
        "{{ shared.team.channel }} {{ shared.admin.key }} {{ shared.nope.x }} {{ shared.loop.other }} {{ shared.loop.self }} {{ x",
    })
    const list = screen.getByRole("list", { name: "References" })
    await waitFor(() => {
      expect(list).toHaveTextContent("{{ shared.team.channel }}: Missing — No value is set")
    })
    expect(list).toHaveTextContent("{{ shared.admin.key }}: Permission denied")
    expect(list).toHaveTextContent("{{ shared.nope.x }}: Missing — There is no such variable.")
    expect(list).toHaveTextContent("Invalid reference — circular reference: shared.loop.other")
    expect(list).toHaveTextContent("A reference is not closed")
    // A value that looks like a reference is that text: never expanded again.
    expect(list).toHaveTextContent("{{ shared.loop.self }} is now {{ shared.loop.self }}")
    const asked = api.previewReferences.mock.calls.flatMap(([, items]) =>
      items.map((i) => i.reference),
    )
    expect(asked.filter((r) => r === "shared.loop.self")).toHaveLength(1)
    expect(screen.getByRole("combobox", { name: /^Message/ })).toHaveAttribute(
      "aria-invalid",
      "true",
    )
  })

  it("reports a whole-value reference of the wrong type", async () => {
    renderField(retries, reference("shared.paths.root"))
    expect(await screen.findByText(/type mismatch/i)).toHaveTextContent("The value is text")
  })

  it("never shows a sensitive value", async () => {
    const log = vi.spyOn(console, "log")
    const warn = vi.spyOn(console, "warn")
    const error = vi.spyOn(console, "error")
    // Even a resolver that wrongly returns the value is not trusted with it.
    agent.set("shared.deploy.token", { type: "string", value: SECRET })
    const original = agent.preview.bind(agent)
    api.previewReferences.mockImplementation((context, items) =>
      Promise.resolve(
        original(context, items).map((p) =>
          p.reference === "shared.deploy.token"
            ? {
                ...p,
                status: {
                  status: "resolved",
                  value: { type: "string", value: SECRET },
                  source: { type: "default" },
                },
              }
            : p,
        ),
      ),
    )
    const { user } = renderField(message, text(""))
    const search = await openPicker(user, /insert a variable into message/i)
    await user.type(search, "deploy")
    await waitFor(() => {
      expect(optionLabels()).toEqual([expect.stringContaining("Deploy token")])
    })
    await waitFor(() => {
      expect(screen.getByLabelText("Current value")).toHaveTextContent("Redacted (sensitive)")
    })
    expect(document.body.innerHTML).not.toContain(SECRET)
    for (const spy of [log, warn, error]) {
      expect(JSON.stringify(spy.mock.calls)).not.toContain(SECRET)
    }
  })

  it("shows redacted for a sensitive reference in a field", async () => {
    renderField(message, { type: "template", value: "{{ shared.deploy.token }}" })
    expect(await screen.findByText(/redacted/i)).toBeInTheDocument()
    expect(document.body.innerHTML).not.toContain(SECRET)
  })

  it("lists system variables without values when the agent cannot list them", async () => {
    api.listVariables.mockRejectedValue("not supported")
    const { user } = renderField(message, text(""))
    const search = await openPicker(user, /insert a variable into message/i)
    expect(await screen.findByText("Values are not available: not supported")).toBeInTheDocument()
    await user.type(search, "platform")
    await waitFor(() => {
      expect(optionLabels()).toEqual([expect.stringContaining("Platform")])
    })
  })
})
