---
title: Layouts
description: Design what each key shows and does in the Layout Designer, with pages, folders, colours, your own icons and keyboard shortcuts.
group: design
order: 5
---

A layout says what every key of a device model shows and does. Layouts belong to a profile and to a device model, and can be designed without the device being connected.

## The Layout Designer

Open **Devices › Layout Designer**. Choose the **Profile**, the **Device model** and the **Layout**; a star marks the layout that is active for that model.

- **New layout** creates a layout. The switch **Place the profile's actions and workflows on the keys** is on by default and fills the new layout for you.
- **Duplicate** copies the layout, **Make active** puts it on the device, **Delete** removes it.

The editor has three parts: the **Library** (Actions, Workflows, Navigation), the device canvas, and the inspector for the selected control.

## Putting something on a key

- Drag an action or a workflow from the Library onto a key, or
- select a key and choose in the inspector, or
- right-click a key and use **Assign**.

The context menu also has Copy, Cut, Paste, Duplicate and Clear. What a key does on a single click, a double click and a long press is described in [Gestures](../gestures/).

## Pages and folders

A layout is a tree of pages. The page bar shows the pages as tabs, the path to the current page and **Page N of M**.

- Type a name in **New folder** to add a page.
- A folder is opened by a key bound to **Open** and the page's name. A folder without a **Back** key gets one automatically on its first free key.
- **Page name** renames a page and **Delete page** removes it. Neither is offered for the home page.

The page tree panel moves pages up and down, and **Move or copy** puts a page at the top level, inside another page, or into another layout. These are unavailable while you have unsaved changes: save or revert first.

## How a key looks

Select a key that has something assigned and use the appearance settings:

- **Background** and **Text colour**: swatches, a picker or a hex value, and **Default**.
- **Text size**: Small, Medium or Large.
- **Title position**: Auto, Top, Middle or Bottom.
- **Show title**.
- The title itself is the binding's **Label (optional)**, up to 32 characters.

### Icons

**Import icon** accepts PNG or JPEG files of up to 2 MiB. Icons are scaled to at most 256 pixels and stored in the profile, which holds up to 64 of them. Fit an icon with **Fit**, **Fill** or **Crop**. An icon that is in use cannot be deleted. DeckPilot has no bundled icon set: the library is what you import.

**Preview with project** shows the keys as they look for a given project, for example with the project's key in a label.

## Saving, undo and shortcuts

Changes are kept in the editor until you **Save**; **Revert** drops them, and an **Unsaved** badge shows that there are some. **Undo** and **Redo** cover the last 100 steps.

| Shortcut | Does |
| --- | --- |
| Cmd/Ctrl + S | Save |
| Cmd/Ctrl + Z | Undo |
| Cmd/Ctrl + Shift + Z, Cmd/Ctrl + Y | Redo |
| Cmd/Ctrl + C, X, V | Copy, cut, paste |
| Cmd/Ctrl + D | Duplicate |
| Cmd/Ctrl + A | Select all |
| Delete | Clear the selected keys |
| Escape | Deselect |
| Shift + click | Select a range |
| Arrow keys | Move the selection |
| Shift + Enter | Open the folder on the selected key |
