# Dialogs (plugin API 1.5)

A plugin does not draw windows. It describes a standard dialog as data —
a confirmation, an alert, a form or a selection — and DeckPilot shows it
with its own components, titled with the plugin's name. The plugin gets a
structured answer that the agent has already checked against the request.
The design and its security boundaries are in
[docs/dialogs.md](../../dialogs.md#plugin-dialog-api-plugin-api-15).

## Requirements

- `"engines": { "pluginApi": "^1.5" }` and, in code,
  `Plugin::requires_api(DIALOGS_API_LEVEL)`. The manifest validator
  refuses the `dialogs` permission below 1.5.
- The `dialogs` permission (medium tier, reviewed at install like any
  other):

  ```json
  { "kind": "dialogs", "justification": "Asks which repository the key opens." }
  ```

- A run the user started: a key press, a gesture, the app (the editor, the
  Action Library or the plugin's page), or a workflow the user started
  that way. In a run started by a schedule, an event, the agent's start or
  a work session, in a state call, and in `dp_init`/`dp_shutdown`, the call
  is denied with `NotPermitted`. This check comes before the rate limit,
  so a refused call does not use one.

## Example

```rust
use deckpilot_plugin_sdk::dialog::{Dialog, DialogResultExt, Field};
use deckpilot_plugin_sdk::prelude::*;

fn configure(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    let dialog = Dialog::form(
        "Configure repository",
        [
            Field::text("repository", "Repository path").required().max_length(200),
            Field::choice("branch", "Branch", [("main", "main"), ("develop", "develop")])
                .default_value("main"),
            Field::boolean("pull", "Pull before opening").default_value(true),
        ],
    )
    .message("Choose the repository this key opens.")
    .confirm_label("Save");

    let answer = match ctx.dialog(dialog) {
        Ok(answer) => answer,
        // No DeckPilot window shows plugin dialogs, or an agent before 1.5.
        Err(e) if matches!(e.kind(), CapabilityErrorKind::Unavailable | CapabilityErrorKind::Unsupported) => {
            ctx.notify("Repository", "Open DeckPilot to choose the repository.")?;
            return Ok(Output::message("waiting for the repository"));
        }
        Err(e) => return Err(e.into()),
    };
    if let Some(reason) = answer.cancel_reason() {
        return Ok(Output::message(format!("not changed ({reason:?})")));
    }
    let repository = answer.text("repository").unwrap_or_default();
    ctx.storage_set("repository", &repository)?;
    ctx.storage_set("pull", answer.flag("pull").unwrap_or(false))?;
    Ok(Output::ok().with("repository", repository))
}
```

The other types:

```rust
let sure = ctx.dialog(Dialog::confirm("Delete the branch “old”?").destructive().confirm_label("Delete"))?;
ctx.dialog(Dialog::alert("Build finished").message("All 214 tests passed."))?;
let profile = ctx.dialog(
    Dialog::selection("Switch profile", [("coding", "Coding"), ("meetings", "Meetings")])
        .option_description("meetings", "Calls and reviews"),
)?;
if let Some(id) = profile.selected() { /* … */ }
```

`Context::dialog` takes a `Dialog` (or a raw `DialogRequest`) and returns a
`DialogResult`; `DialogResultExt` reads it (`values`, `selected`, `text`,
`integer`, `flag`, `lines`, `cancel_reason`). The call blocks the
invocation until the user answers, the dialog times out or the plugin is
stopped.

## Request

| Member | Types | Constraints |
| --- | --- | --- |
| `type` | all | `confirm`, `alert`, `form` or `selection` |
| `title` | all | plain text, 1–80 characters, one line |
| `message` | all | optional plain text, at most 600 characters; line breaks kept, nothing else formatted |
| `fields` | `form` | 1–12; ids unique, `[a-z][a-zA-Z0-9_-]{0,31}` |
| `options` | `selection` | 1–100 of `{ id, label, description? }`: ids unique, 1–64 characters; labels at most 60, descriptions at most 200 |
| `confirmLabel`, `cancelLabel` | confirm, form, selection | at most 24 characters |
| `destructive` | `confirm` | a red confirm button |
| `timeoutSeconds` | all | 10–300, default 120, and never past the action's own time limit (see below) |

Field kinds, each with `id`, `label` (at most 60), optional `description`
(at most 200), `required` and `default` of its type:

| `kind` | Extra members | Value the plugin receives |
| --- | --- | --- |
| `text` | `maxLength` (1–2,000; 2,000 without it), `multiline` | a string, `\r\n` as `\n`; one line unless `multiline` |
| `integer` | `min`, `max` | an integer within them |
| `boolean` | | `true` or `false` |
| `choice` | `options`: 1–50 of `{ value, label }`, values unique | one of the values |
| `lines` | | a list of at most 100 non-empty lines of at most 200 characters |

There is **no secret or password kind**: secrets belong in
[credential slots](capabilities.md#credentials). Unknown members, types and
kinds are refused, as are members of another kind (`maxLength` on an
integer) and `destructive` on anything but a confirmation. No text is
interpreted: no markup, links, HTML or images. The whole request is at most
32 KiB of JSON, and a form's answer at most 48 KiB.

The SDK checks the request before it calls the host
(`CapabilityErrorKind::BadRequest`, naming the member); the agent checks it
again.

## Result

```json
{ "outcome": "confirmed", "values": { "repository": "~/code/shop", "branch": "main", "pull": true } }
{ "outcome": "confirmed", "selected": "coding" }
{ "outcome": "confirmed" }
{ "outcome": "cancelled", "reason": "user" }
```

`reason` is `user` (Cancel, Escape, the close button), `timeout` or
`appClosed` (DeckPilot's window went away). A reason added later reads as
`CancelReason::Other`. A form's `values` hold exactly the fields the user
filled in, of their kinds and within their bounds; an optional field left
empty is absent. When the plugin is disabled, updated, uninstalled or
crashes, or its action is cancelled, the dialog closes in the app
(`pluginStopped`) and the invocation ends without an answer.

## Errors

| `CapabilityErrorKind` | Wire code | When |
| --- | --- | --- |
| `BadRequest` | `bad-request` | the request breaks a rule above (the design's `invalid-request`) |
| `NotPermitted` | `not-permitted` | no `dialogs` grant, a run the user did not start, a state call, `dp_init`/`dp_shutdown` |
| `Throttled` | `throttled` | more than 6 dialogs a minute, or the plugin already shows one (the design's `rate-limited`) |
| `Unavailable` | `unavailable` | no DeckPilot window that shows plugin dialogs, or too little of the action's time left |
| `Unsupported` | `unsupported-op` | an agent before plugin API 1.5 |

## Time limits

A dialog waits inside the action's invocation, so the action's own time
limit (`node.timeoutMs`, 30 s unless the manifest says otherwise, at most
600 s) bounds it: the dialog times out at `timeoutSeconds` or 2 s before the
action's limit, whichever is first, so the plugin still receives `timeout`
and can finish. An action that asks the user something should declare a
`timeoutMs` that leaves room for it (for example 150,000 for the default
120 s). With less than 2 s left the call is `Unavailable`.

## Testing

`MockHost::allow_all()` confirms every dialog with its defaults (a
selection with its first option); `MockHost::new` answers as you script
it:

```rust
use deckpilot_plugin_sdk::testing::MockHost;
use deckpilot_plugin_sdk::{CallResult, CapabilityRequest};

let mut host = MockHost::new(|req| match req {
    CapabilityRequest::DialogOpen(_) => CallResult::Ok(serde_json::json!(
        {"outcome": "confirmed", "values": {"repository": "~/code/shop"}})),
    _ => CallResult::Ok(serde_json::Value::Null),
});
```
