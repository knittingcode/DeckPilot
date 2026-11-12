// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest"

import { editorCommand, isTextEntry } from "./shortcuts"

// jsdom is not macOS: the command modifier is Ctrl.
function keydown(target: HTMLElement, key: string, init: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key, ctrlKey: true, bubbles: true, ...init })
  Object.defineProperty(event, "target", { value: target })
  return event
}

function mount(html: string): HTMLElement {
  document.body.innerHTML = html
  return document.body.firstElementChild as HTMLElement
}

afterEach(() => {
  document.body.innerHTML = ""
  window.getSelection()?.removeAllRanges()
})

describe("editorCommand", () => {
  it("maps the editing shortcuts", () => {
    const button = mount("<button>Key 1</button>")
    expect(editorCommand(keydown(button, "z"))).toBe("undo")
    expect(editorCommand(keydown(button, "Z", { shiftKey: true }))).toBe("redo")
    expect(editorCommand(keydown(button, "y"))).toBe("redo")
    expect(editorCommand(keydown(button, "c"))).toBe("copy")
    expect(editorCommand(keydown(button, "x"))).toBe("cut")
    expect(editorCommand(keydown(button, "v"))).toBe("paste")
    expect(editorCommand(keydown(button, "d"))).toBe("duplicate")
    expect(editorCommand(keydown(button, "s"))).toBe("save")
  })

  it("ignores other modifiers and plain keys", () => {
    const button = mount("<button>Key 1</button>")
    expect(editorCommand(keydown(button, "z", { ctrlKey: false }))).toBeNull()
    expect(editorCommand(keydown(button, "z", { altKey: true }))).toBeNull()
    expect(editorCommand(keydown(button, "z", { metaKey: true }))).toBeNull()
    expect(editorCommand(keydown(button, "c", { shiftKey: true }))).toBeNull()
    expect(editorCommand(keydown(button, "q"))).toBeNull()
  })

  it("leaves text fields their native editing, but saves from them", () => {
    for (const html of [
      "<input>",
      '<input type="search">',
      "<textarea></textarea>",
      "<select></select>",
      '<div contenteditable="true"><span>text</span></div>',
    ]) {
      const field = mount(html)
      const target = field.querySelector("span") ?? field
      expect(isTextEntry(target)).toBe(true)
      expect(editorCommand(keydown(target, "z"))).toBeNull()
      expect(editorCommand(keydown(target, "c"))).toBeNull()
      expect(editorCommand(keydown(target, "v"))).toBeNull()
      expect(editorCommand(keydown(target, "s"))).toBe("save")
    }
  })

  it("treats checkboxes and other non-text inputs as editor focus", () => {
    const checkbox = mount('<input type="checkbox">')
    expect(isTextEntry(checkbox)).toBe(false)
    expect(editorCommand(keydown(checkbox, "z"))).toBe("undo")
  })

  it("leaves everything to an open dialog or menu", () => {
    const button = mount('<div><button>Key 1</button><div role="dialog"></div></div>')
    const key = button.querySelector("button") as HTMLElement
    expect(editorCommand(keydown(key, "z"))).toBeNull()
    expect(editorCommand(keydown(key, "s"))).toBeNull()
    mount('<div role="menu"><button>Item</button></div>')
    const item = document.querySelector("button") as HTMLElement
    expect(editorCommand(keydown(item, "v"))).toBeNull()
  })

  it("leaves copying selected text to the browser", () => {
    const text = mount("<p>Some log line</p>")
    const range = document.createRange()
    range.selectNodeContents(text)
    window.getSelection()?.addRange(range)
    expect(editorCommand(keydown(text, "c"))).toBeNull()
    expect(editorCommand(keydown(text, "x"))).toBeNull()
    expect(editorCommand(keydown(text, "z"))).toBe("undo")
  })

  it("skips events another handler took", () => {
    const button = mount("<button>Key 1</button>")
    const event = keydown(button, "z", { cancelable: true })
    event.preventDefault()
    expect(editorCommand(event)).toBeNull()
  })
})
