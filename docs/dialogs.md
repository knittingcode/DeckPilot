# Dialogs

DeckPilot has one dialog system. Every modal surface, whether it is a native window used as a dialog
(unlock, export, update), an in-app dialog (confirmations, forms, plugin consent) or a dialog a
plugin asks for, is drawn by the same frame with the same tokens and follows the same lifecycle
rules. This page describes the architecture, how to use it in core code, the plugin dialog API,
and the security boundaries.

- [Architecture](#architecture)
- [Templates](#templates)
- [Opening dialogs from code](#opening-dialogs-from-code)
- [Lifecycle rules](#lifecycle-rules)
- [Visual standards](#visual-standards)
- [Accessibility](#accessibility)
- [Native windows](#native-windows)
- [Security boundaries](#security-boundaries)
- [Plugin dialog API (plugin API 1.5)](#plugin-dialog-api-plugin-api-15)
- [Migration guidance](#migration-guidance)
- [Development: Dialog Showcase](#development-dialog-showcase)
- [Inventory](#inventory)

## Architecture

```
apps/desktop/src/components/
├── modal-shell.tsx          ModalShell: the frame (window | dialog presentation), ModalIcon
├── field-error.tsx          FieldError: the inline error of a control
├── password-field.tsx       PasswordField: masked input, show/hide, inline error
└── dialogs/
    ├── index.ts             the public surface: import from "@/components/dialogs"
    ├── templates.tsx        Confirm, Destructive, Alert, Form, Selection, Permission, Progress
    ├── credential-dialog.tsx Credential (password) template
    └── dialog-service.tsx   DialogProvider + useDialogs(): open dialogs from code
apps/desktop/src/lib/credential-errors.ts   backend error → user-facing credential message
apps/desktop/src/plugins/dialog-host.tsx    PluginDialogHost: plugin dialogs (plugin API 1.5) on the templates
apps/desktop/src/dev/dialog-showcase.tsx    development-only showcase
```

Layers, from the bottom up:

1. **Primitives:** shadcn/ui `Dialog` (Radix) for in-app dialogs, focus trap and portal; native
   Tauri windows (`src-tauri/src/windows.rs`) for dialogs that must be isolated from the main
   window; `Button`, `Input`, `Label`, `Progress`.
2. **Frame:** `ModalShell`. It renders the parts every dialog has: header (optional icon tile,
   title, description), body, an inline error, and the footer (optional status text, secondary and
   primary action). `presentation="dialog"` puts it in a Radix dialog; `presentation="window"` makes it
   the content of a native window, which sizes itself to it (`lib/window-fit.ts`). Both
   presentations render the identical `Parts`.
3. **Templates:** typed arrangements of the frame for one purpose each (below). They own the
   behaviour that used to be re-implemented per dialog: busy state, inline errors, double-submission
   protection, unsaved changes.
4. **Service:** `DialogProvider` and `useDialogs()` open the confirm, destructive, alert and
   selection templates from code and resolve a promise with the answer.
5. **Plugin bridge** (plugin API 1.5): the agent relays a plugin's declarative dialog request to the
   app, which renders it with the same templates; see below.

There is no second dialog system: anything modal is a template, or `ModalShell` directly when no
template fits (multi-step windows such as import).

## Templates

| Template | Component | Use for | Notes |
| --- | --- | --- | --- |
| Alert | `AlertDialog` | Information, warnings, recoverable errors | One button; `tone`: info, warning, error |
| Confirm | `ConfirmDialog destructive={false}` | Ordinary confirmations | `alertdialog`; a click outside does not dismiss |
| Destructive | `DestructiveDialog` (or `ConfirmDialog`, whose default is destructive) | Deleting profiles, layouts, plugins, credentials | Red icon tile and button; default label "Delete" |
| Form | `FormDialog` | Creating or editing structured data | `canSubmit`, async `onSubmit`, `dirty` asks before discarding |
| Selection | `SelectionDialog` | Choosing a layout, profile, page… | Radio group (arrow keys), filter above 8 items |
| Permission | `PermissionDialog` | Reviewing and granting plugin capabilities | Host-only; always names the requester |
| Progress | `ProgressDialog` | Long-running operations | `value` 0–100 or `null`; blocking unless `onCancel` |
| Credential | `CredentialDialog` | Password entry and authentication | Host-only; window or dialog presentation |

Shared behaviour of the templates:

- **Async actions.** `onConfirm`, `onSubmit`, `onSelect` and `onAllow` may return a promise. While it
  runs, the primary button shows its busy label with a spinner, other actions are disabled, a
  second Enter or click is ignored, and the dialog cannot be dismissed. On success the dialog
  closes; on failure the error appears inline and the dialog stays open for another try.
- **Errors** are user-facing sentences. A field's error goes below the field (`FieldError`), an
  error of the whole dialog at the end of the body (the frame's `error`). `describeError` maps
  thrown values; the default is the backend's message, which DeckPilot's commands word for users.
  Credential failures always go through `credential-errors.ts`, so internal details (paths, SQL,
  protocol versions) never appear.

```tsx
import { DestructiveDialog } from "@/components/dialogs"

<DestructiveDialog
  trigger={<Button variant="ghost">Delete…</Button>}
  title={`Delete “${layout.name}”?`}
  description="Its pages and keys are deleted. This cannot be undone."
  busyLabel="Deleting…"
  onConfirm={() => api.deleteLayout(layout.id)} // stays open until this settles
/>
```

```tsx
<FormDialog
  open={open}
  onClose={() => setOpen(false)}
  title="New layout"
  submitLabel="Create"
  canSubmit={name.trim().length > 0}
  dirty={name.length > 0}
  onSubmit={() => api.createLayout({ name })}
>
  <Label htmlFor="name">Name</Label>
  <Input id="name" autoFocus value={name} onChange={(e) => setName(e.target.value)} />
</FormDialog>
```

## Opening dialogs from code

```tsx
const dialogs = useDialogs()

if (await dialogs.destructive({ title: "Delete “Work”?", description: "…" })) { … }
const ok = await dialogs.confirm({
  title: "Use this layout in “Work”?",
  description: "…",
  confirmLabel: "Use layout",
  onConfirm: () => api.useLayout(id), // runs inside the dialog; resolves true only on success
})
await dialogs.alert({ title: "Layout copied", tone: "info" })
const page = await dialogs.select({ title: "Move to", options }) // id or null
```

`DialogProvider` is mounted in the main window inside the security gate (so it exists only while
the configuration is unlocked) and in every native window.

## Lifecycle rules

- **One at a time.** The service shows one dialog; further requests wait in a FIFO queue. Nothing
  overlaps unpredictably.
- **Nesting.** A dialog a component renders (a form, the authentication dialog) may have exactly one
  dialog on top: a confirmation it owns (e.g. "Discard your changes?") or a service request.
  Radix keeps the focus inside the topmost dialog. Deeper nesting is not used.
- **Every request settles once.** Confirmed, cancelled (Cancel, Escape, close button), or cancelled
  because its provider went away (the window closed, the app locked). Nothing waits on a dialog
  that can no longer be answered, and late answers are ignored.
- **Escape** cancels non-blocking dialogs, never while an action runs. **Enter** submits the
  primary action (dialogs with an `onSubmit` are forms), and never when the primary action is
  disabled or busy. **Clicks outside** dismiss ordinary dialogs, but not `alertdialog`s
  (confirmations, alerts), blocking dialogs or a dialog that is busy.
- **Focus.** On open: the field marked `autoFocus`, else the first field, else the dialog itself.
  A button is never focused automatically, so a stray Return cannot confirm anything. On close the
  focus returns to the element that had it when the dialog opened.
- **Unsaved changes.** `FormDialog` with `dirty` asks before discarding; after "Cancel" the user is
  back in the form with their input.

## Visual standards

All values come from the theme tokens (`index.css`), so light and dark follow the selected theme.

| Part | Default | Compact (`density="compact"`) |
| --- | --- | --- |
| Header | `px-6 pt-6 pb-4`, icon tile 36 px, top-aligned | `pt-5 pb-4`, icon tile 32 px, centred on the text |
| Title / description | `text-base font-semibold` / `text-sm text-muted-foreground` | same |
| Body | `px-6 pb-4`, `gap-4` | `pb-2` |
| Footer | border-top, `px-6 py-4` (≈ 69 px with a 36 px button) | `py-3` (≈ 61 px) |
| Buttons | default size (36 px); secondary `outline`, destructive `variant="destructive"` | same |
| Errors | inline: 14 px warning icon, `text-xs text-destructive`, no box | same |

- No decorative cards or bordered alert boxes for validation. A bordered `Alert` is still right for
  persistent explanatory notices *inside* a body (e.g. "The unencrypted original is kept").
- Footers are content-driven: no minimum heights or spacers.
- Widths: `size` (fit, compact 360–440, medium 480–680, large, fullscreen) in windows; the same
  names map to `max-w` in dialogs. Long content scrolls in the body; the footer stays visible.

## Accessibility

- `role="dialog"`, or `alertdialog` for confirmations, alerts and blocking prompts; labelled by the
  title and described by the description.
- Invalid controls have `aria-invalid` and `aria-describedby` pointing at their `FieldError`, which
  is a `role="alert"`; space for it can be reserved (`reserve`) so the layout does not jump. Each
  new attempt clears and re-announces the message.
- Busy buttons have `aria-busy`; progress is a `progressbar` in a polite live region.
- The show/hide password button has `aria-label` "Show password"/"Hide password", `aria-pressed`
  and `aria-controls`; a revealed password has spell checking, autocorrect and capitalisation off.

## Native windows

Dialogs that must stay outside the main window are native windows (`src-tauri/src/windows.rs`):
`security.setup`, `security.unlock`, `security.migrate`, `security.confirm` (blocking),
`portability.export`, `portability.import`, `updates.progress`, `workflows.execution-summary` and
`app.logs`. They render `ModalShell presentation="window"` and size themselves to the content. The
size is reported again whenever the content or the viewport changes; the native side derives the
title bar from window size minus viewport, so a measurement taken mid-resize is corrected by the
next one (`lib/window-fit.ts`).

The security windows stay native: the main window shows only a locked placeholder (no content) and
cannot show app dialogs, because `DialogProvider` exists only behind the security gate.

## Security boundaries

- **Host-only templates.** `CredentialDialog` and `PermissionDialog` are rendered only by DeckPilot
  code. Plugins cannot open them, change their text or pre-answer them.
- **Passwords** live only in the credential template's component state and are passed to the one
  backend command that checks them. They are never logged, stored, sent to telemetry or exposed to
  other components or plugins.
- **No plugin-provided UI code.** Plugin dialogs are declarative data (below), rendered by the
  host's own components. HTML, JavaScript, CSS, React components, URLs to load and images are not
  accepted.
- **No impersonation.** Plugin dialogs always show the plugin's name and icon from its installed
  manifest, never DeckPilot's lock or shield icons, and cannot be blocking. They have no secret
  field type.
- **Consent** is never bypassed: a plugin needs the `dialogs` permission, granted by the user like
  any other.

## Plugin dialog API (plugin API 1.5)

Implemented in plugin API 1.5 and IPC protocol 12, as approved; the few places where the
implementation had to be more precise than the design are listed under
[Implementation notes](#implementation-notes). Plugins do not implement modal windows; they ask
DeckPilot to show a standard dialog and receive a structured result. SDK guide:
[marketplace/sdk/dialogs.md](marketplace/sdk/dialogs.md).

### Requirements

- `engines.pluginApi` of at least `^1.5`.
- The `dialogs` permission (medium tier, reviewed at install like any permission):

  ```json
  { "permissions": [{ "kind": "dialogs", "justification": "Asks which repository the key opens." }] }
  ```

- A **user-initiated** run: a key press or an action the user ran. Calls from state updates,
  `dp_init`/`dp_shutdown` or background work are refused with `not-permitted`, before any rate
  limit, like `url.open`.

### Request

Host call `dialog.open` (Rust SDK: `ctx.dialog(...)`). The arguments are strict: unknown fields
are rejected.

```json
{
  "type": "form",
  "title": "Configure repository",
  "message": "Choose the repository this key opens.",
  "fields": [
    { "id": "repository", "kind": "text", "label": "Repository path", "required": true, "maxLength": 200 },
    { "id": "branch", "kind": "choice", "label": "Branch", "options": [
      { "value": "main", "label": "main" }, { "value": "develop", "label": "develop" } ], "default": "main" },
    { "id": "pull", "kind": "boolean", "label": "Pull before opening", "default": true }
  ],
  "confirmLabel": "Save",
  "timeoutSeconds": 120
}
```

| Field | Types | Constraints |
| --- | --- | --- |
| `type` | all | `confirm`, `alert`, `form` or `selection` |
| `title` | all | plain text, 1–80 characters, one line |
| `message` | all | optional plain text, at most 600 characters; no markup |
| `fields` | `form` | 1–12 fields; ids unique, `[a-z][a-zA-Z0-9_-]{0,31}` |
| `options` | `selection` | 1–100 of `{ id, label, description? }` |
| `confirmLabel`, `cancelLabel` | confirm, form, selection | at most 24 characters |
| `destructive` | `confirm` | red confirm button |
| `timeoutSeconds` | all | 10–300, default 120; capped by the invocation's own deadline |

Field kinds reuse the manifest's parameter kinds: `text` (`maxLength` ≤ 2000, `multiline`),
`integer` (`min`, `max`), `boolean`, `choice` (`options`, ≤ 50), `lines`. Every field has a
`label` (≤ 60), optional `description` (≤ 200), `required` and `default`. There is **no secret or
password kind**; secrets belong in credential slots.

### Result

```json
{ "outcome": "confirmed", "values": { "repository": "~/code/shop", "branch": "main", "pull": true } }
{ "outcome": "confirmed", "selected": "coding" }
{ "outcome": "cancelled", "reason": "user" }
```

`reason` is one of `user`, `timeout`, `pluginStopped` or `appClosed`. Values are validated by the
agent against the request before the plugin receives them. A plugin only ever receives the
answer to its own call.

Errors: `invalid-request` (schema), `not-permitted` (no `dialogs` grant, or not user-initiated),
`rate-limited`, `unavailable` (no DeckPilot window open, or an app without plugin dialogs). On the
wire these are ABI 1's existing codes `bad-request`, `not-permitted`, `throttled` and
`unavailable` (SDK: `CapabilityErrorKind::BadRequest`, `NotPermitted`, `Throttled`,
`Unavailable`).

### Lifecycle and limits

- One open dialog per plugin. At most 6 requests per minute per plugin. Plugin dialogs wait behind
  DeckPilot's own dialogs and never cover a security window.
- The call blocks the plugin's invocation until answered, timed out or interrupted. Disabling,
  updating or uninstalling the plugin, a plugin crash, or an invocation deadline closes its dialog
  (`pluginStopped`). Closing the app answers `appClosed`. Callbacks never reach a stopped plugin.
- Plugins cannot close, change or read other plugins' dialogs: there is no operation for it, and
  dialog ids are random and known only to the agent and app.

### Compatibility

- IPC protocol 12 adds the plugin dialog event and its answer request, behind the feature string
  `pluginDialogs`. An app that does not announce the feature never receives plugin dialogs; the
  plugin then gets `unavailable`.
- A host older than 1.5 refuses the unknown op (`unsupported-op`, SDK
  `CapabilityErrorKind::Unsupported`); plugins that must run on older hosts treat that error (and
  `unavailable`) as the signal to fall back, for example to a notification.
- New dialog types, field kinds or constraints are additive and versioned by the plugin API minor.
  Removing or changing the meaning of a field requires a new major.

### In the app

The main window renders plugin dialogs in builds with plugins (`VITE_DECKPILOT_PLUGINS=1`), only
while the configuration is unlocked (`PluginDialogHost` inside `DialogProvider`). While the host is
mounted, the app announces `pluginDialogs` to the agent; when it unmounts, the window closes or the
app locks, it stops announcing, and the agent cancels open dialogs with `appClosed`. Plugin dialogs
are shown one at a time, first in first out, and a new one waits while one of DeckPilot's own
dialogs is open. Each shows a "Requested by ‹plugin name›" line with a generic plugin icon (never a
lock or shield), renders all text as plain text, and can always be dismissed with Cancel, Escape or
the close button. Fields use the app's own controls; required fields keep the confirm button
disabled, limits are checked while typing, and an answer over 48 KiB is refused before it is sent.
When the agent refuses an answer, its message appears inside the dialog; when the agent closes a
dialog, it disappears without an answer. A hidden main window comes forward when a plugin dialog
opens.

### Implementation notes

Where the implementation is more precise than the approved design, and why:

- **Error names.** `invalid-request` and `rate-limited` travel as ABI 1's existing codes
  `bad-request` and `throttled`. `CallErrorCode` is a closed enum that every built guest parses, so
  new codes would have broken guests built before 1.5.
- **Justification.** The permission needs a `justification` like every other kind; the consent
  review shows it.
- **Fallback signal.** The SDK has no `ctx.api_level()` (a guest is not told the host's level);
  an older host answers `unsupported-op`, which is the fallback signal.
- **`lines` bounds.** The design gives none. A `lines` field takes at most 100 non-empty lines of
  at most 200 characters, and its value is a list of strings.
- **Size bounds.** IPC messages are at most 64 KiB, so a request is at most 32 KiB of JSON and a
  form's answer at most 48 KiB. Labels of options are at most 60 characters, option ids and values
  at most 64.
- **Deadline.** The dialog times out 2 s before the invocation's deadline at the latest, so the
  plugin receives `timeout` and can finish. With less time left the call is `unavailable`. The
  action's `timeoutMs` (30 s by default) therefore bounds a dialog: actions that ask the user
  should declare more.
- **Stopped plugins.** Disabling, updating or uninstalling the plugin, cancelling its run, or a
  crash of its host closes the dialog in the app with `pluginStopped`; the invocation ends without
  a result (a stopped plugin receives nothing).
- **Close events.** Besides `timeout`, `pluginStopped` and `appClosed`, the agent's close event
  has `answered` (the dialog was answered on another connection of the app).
- **One per plugin** is refused as `throttled`.
- **`dp_init`/`dp_shutdown`.** The host refuses `dialog.open` there with `not-permitted`; other
  capabilities stay `unavailable` there as before.
- **Translations (plugin API 1.6).** The title, message, button labels, field labels and
  descriptions and option labels and descriptions may be `t:` keys of the plugin's own locale
  files; the agent resolves them in the app's language, within the limits above, before the app
  draws the dialog ([sdk/localization.md](marketplace/sdk/localization.md)). Ids, values and
  defaults are never translated, and the result is plain text like every dialog text.

## Migration guidance

1. Prefer a template; use `ModalShell` directly only for multi-step windows.
2. Replace `Dialog`/`DialogContent`/`AlertDialog*` in feature code with the template. Feature code
   must not import `@/components/ui/dialog` or `@/components/ui/alert-dialog`.
3. Move `error` boxes to the frame's `error` or a `FieldError`; keep success toasts.
4. Return the promise from `onConfirm`/`onSubmit` instead of closing first and toasting errors.
5. Destructive buttons use `variant="destructive"` (no class overrides).

## Development: Dialog Showcase

In `tauri dev` (any Vite dev build), **Settings › Advanced › Dialog showcase** opens every
template in its states (default, busy, inline error, destructive, permission request, long
content, progress, credential, service queue) plus native-window frames at the narrowest and
widest compact width, with a light/dark/system switch. It uses the real components with simulated
actions. Release builds do not contain the page (`import.meta.env.DEV` is `false` there).

## Inventory

The state at the time of the unification (November 2026). Before it, ModalShell already served
the native windows and three in-app dialogs. Fourteen raw Radix dialogs, five raw alert dialogs
and thirteen `ConfirmDialog` uses had their own frames, error boxes and footers, and there was no
service. The three plugin consent dialogs were near-identical copies.

| Area | Surfaces | Now |
| --- | --- | --- |
| Security windows (setup, unlock, migrate, confirm) | 4 native windows, 7 sub-views | ModalShell window; password prompts on `CredentialDialog` |
| Sensitive operations | AuthenticateDialog, ChoosePasswordDialog | `CredentialDialog` / ModalShell dialog |
| Export, import, update, logs, workflow summary | 5 native windows | ModalShell window |
| Confirmations | 13 `ConfirmDialog` call sites | Confirm/Destructive template |
| Plugin consent and management | review, migration consent, install plan, removal, configure, package install, uninstall | Permission / Destructive / Form templates |
| Profiles, layouts, projects | name, new layout ×2, profile, project, binding, layout delete | Form / Destructive templates |
| Layout and gesture editors | page delete, move page, Multi Action steps, gesture conversion | Destructive / Selection / ModalShell / Confirm |
| Native file pickers | export save, import open, resource path, plugin package | Tauri dialog plugin (system UI) |
| Browser `alert`/`confirm`/`prompt` | none | — |
