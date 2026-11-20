# Actions: registration, parameters, results

An action is declared twice: in `manifest.json` (what the agent, the
editors and the user see) and in code (`deckpilot_plugin_sdk::Action`,
what runs). They must agree ([manifest.md](manifest.md#the-module-must-agree)).

## Registering actions

```rust
use deckpilot_plugin_sdk::prelude::*;

pub fn plugin() -> Plugin {
    Plugin::new("acme.developer-tools", "0.1.0")          // = manifest id, version
        .action(
            Action::new("timestamp", timestamp)            // = contributes.actions[].id
                .param(Param::text("value").required().max_length(40))
                .param(
                    Param::choice("from", &["auto", "seconds", "milliseconds", "rfc3339"])
                        .default("auto"),
                ),
        )
        .action(Action::new("last-values", last_values))  // no parameters
}

deckpilot_plugin_sdk::plugin!(plugin);
```

- `Plugin::new(id, version)`, `.action(Action)`, `.on_init`, `.on_shutdown`
  (`src/definition.rs`). `Plugin::validate()` checks ids, keys,
  uniqueness, kinds and defaults against the manifest's rules; call it in
  a unit test. An invalid definition fails every invocation with
  `plugin-error` and makes `dp_init` return 2 (the load fails).
- `Action::new(id, handler)` where `handler` is a
  `Handler = fn(&mut Context<'_>, &Args<'_>) -> ActionResult` (a plain
  function or a non-capturing closure).
- `plugin!(build_fn)` generates the guest ABI 1 exports
  (`dp_abi_version`, `dp_alloc`, `dp_invoke`, `dp_describe`, `dp_init`,
  `dp_shutdown`, `dp_features`, `dp_state`). Use it once, at the root of the `cdylib` crate. The
  plugin is built on first use and kept for the life of the module. The
  plugin crate needs no `unsafe` of its own; use `#![deny(unsafe_code)]`
  (not `forbid`: the macro's export attributes need an `allow`).

`dp_describe` reports a `PluginDescriptor` (SDK name and version, API
level, action ids, parameter kinds; never the plugin id, which comes from
the channel). The supervisor compares its action ids with the manifest's
and refuses a mismatch.

## Parameters

### In the manifest (`contributes.actions[].parameters[]`)

`{ "key", "label", "description"?, "kind", "required", "default"? }`:
`key` camelCase `[a-z][A-Za-z0-9]{0,31}`, `label` 1-48, `description`
≤ 300, `required` a boolean (always present), `default` only on optional
parameters, as `{ "type": "<text|integer|boolean|choice>", "value": … }`.

| `kind.type` | Kind fields | Arrives as | SDK declaration | Read as |
| --- | --- | --- | --- | --- |
| `text` | `maxLength` (1-100,000), `multiline` | string | `Param::text(k)` `.max_length(n)` | `String` |
| `integer` | `min`, `max` | integer | `Param::integer(k)` `.min(a)` `.max(b)` (whole numbers) | `i64` (or `f64`) |
| `boolean` | | boolean | `Param::boolean(k)` | `bool` |
| `choice` | `options` (1-64 unique, 1-64 characters each) | string | `Param::choice(k, &[..])` | `String` |
| `project` | | the project's id, as a string | `Param::text(k)` | `String` |
| `lines` | `itemLabel`, `maxItems`, `maxLength` | the text | `Param::text(k)` | `String` |
| `pairs` | `separator` (`=` or `:`), `nameLabel`, `valueLabel`, `maxItems`, `caseInsensitive`, `noSecrets` | the text | `Param::text(k)` | `String` |
| `code` | `language` (`json`, `text`, `yaml`, `markdown`), `maxLength` | the text | `Param::text(k)` | `String` |

The SDK also has `Param::number` (finite `f64`) and `Param::secret` (a
credential slot key, read as `SecretHandle`). **Manifest version 1 has no
kind for them**, so the agent refuses any value for such a parameter as
undeclared: they work under `testing::MockHost` but cannot be reached
through the agent. Use `integer` or `text`, and name credential slots in
code (`SecretHandle::slot`).

Mind the ABI's bounds when you choose `maxLength`: one text value is at
most 16 KiB of UTF-8 (`MAX_STRING_LEN`), so a `maxLength` above 4,096
characters can be refused for non-ASCII text by the host rather than by
the agent's friendlier check.

### Validation, twice, before your handler runs

1. **The agent** (every source: `invokePluginAction`, keys, `runAction`,
   workflow nodes) normalizes the parameters against the manifest with the
   core's `ActionType::normalize`: unknown parameters, wrong types, values
   out of range, options not offered, too long texts and missing required
   parameters (a blank required text counts as missing) fail with
   `invalid-parameters`, and a message that lists every problem and names
   the action type; defaults are filled in. References and templates are
   resolved by the agent; the plugin only ever sees literal values.
2. **The SDK** (`args::normalize`) checks the same against the `Param`
   declarations: unknown keys, kinds, bounds, options; it fills defaults
   and refuses with `invalid-parameters`, naming the parameter but never
   quoting its value.

### Reading them (`Args`)

```rust
fn build_status(ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let branch: Option<String> = args.get("branch")?;   // optional, no default
    let from: String = args.get("from")?;                // default filled in
    ...
}
```

`Args::get::<T>(key)` with `T` = `String`, `i64`, `f64`, `bool`,
`SecretHandle` or `Option<T>` of those (`src/args.rs`, `FromArg`). Errors
are typed: a required value absent (an optional parameter read as `T`
instead of `Option<T>`) is `invalid-parameters`; an undeclared key or a
`T` that does not fit the kind is `plugin-error` (a bug in the plugin).
`Args::has(key)` and `Args::values()` give the raw checked values.

## Results

A handler returns `ActionResult = Result<Output, PluginError>`
(`src/output.rs`):

| Constructor | Outcome | Meaning |
| --- | --- | --- |
| `Output::ok()` | `ok` | Success, no message |
| `Output::message(text)` | `ok` | Success with a message for the user (≤ 1,024 characters, `MAX_MESSAGE_LEN`; never secrets) |
| `Output::skipped(text)` | `skipped` | Nothing to do; a workflow step counts as skipped |
| `.with(key, value)` | | Adds one output: `String`/`&str`, `bool`, `i8`…`i64`, `u8`…`u32`, `f32`/`f64`, `Vec<T>`, or a `ParamValue` (`IntoValue`) |
| `Err(PluginError::new(code, message))` | `failed` | A typed failure ([errors-and-versions.md](errors-and-versions.md#error-codes)); `.retryable()` marks it transient |

Output keys should be the manifest's `node.outputs[].key`: **the agent
drops undeclared outputs**. Before the result leaves the module, the SDK
checks it (`ActionOutcome::validate`): at most 64 entries
(`MAX_MAP_ENTRIES`), names `[A-Za-z][A-Za-z0-9._-]*` ≤ 64, text ≤ 16 KiB,
lists ≤ 256 items, nesting ≤ 4, numbers finite, code
`[a-z][a-z0-9-]*` ≤ 64; anything else becomes `invalid-result`. The whole
output is at most 192 KiB of JSON (`MAX_GUEST_OUTPUT`).

How the result reaches the user:

- `invokePluginAction` (the app's **Run** button): `{status: "ok",
  message?, data}` with the declared outputs as JSON, `{status:
  "skipped", message?}` or `{status: "failed", code, message?,
  retryable}`.
- Keys, `runAction` and workflow nodes: an `ActionResult` of the engine;
  success uses the message or "<action name> finished", outputs become
  texts (`engine.rs`), a failure carries the code and message.

## Key state

What a key bound to an action shows can follow the plugin (plugin API
1.2). **The plugin describes, the agent draws**: a state handler returns a
small typed `KeyState`, and the agent's renderer draws it at the device's
resolution. There are no pixels, fonts or markup, and a plugin learns
nothing about devices or keys.

```rust
fn shown(ctx: &mut Context<'_>, args: &Args<'_>) -> Result<KeyState, PluginError> {
    let until = ctx.storage_get_as::<u64>("focusUntil")?.unwrap_or(0);
    let left = until.saturating_sub(ctx.now()?.unix_ms) / 1000;
    if left == 0 {
        return Ok(KeyState::new().label("Focus").glyph(Glyph::Play));
    }
    Ok(KeyState::new()
        .label(format!("{}:{:02}", left / 60, left % 60))
        .detail("Focus")
        .accent(Accent::Green)
        .glyph(Glyph::Timer)
        .progress(u8::try_from(100 - left * 100 / 1500).unwrap_or(0)))
}

Action::new("focus", focus).state(shown)
```

| `KeyState` member | Bound | Drawn as |
| --- | --- | --- |
| `label`, `detail` | 32 characters each, no control characters | the key's title and second line |
| `indicator` | `Indicator::{Ok, Warning, Error, Running, Unknown}` | a mark in the top right corner |
| `progress` | 0 to 100 | a bar along the bottom |
| `toggle` | `bool` | a mark along the top |
| `accent` | `Accent::{Blue, Green, Amber, Red, Purple, Teal, Grey}` | the key's background; the user's own colours win |
| `glyph` | `Glyph::{Check, Cross, Play, Pause, Stop, Timer, Bell, Up, Down, Dot, Circle, Square, Diamond, Triangle, Star, Heart}` | a shape above the title; the user's own icon wins |
| `badge` | 4 characters | a small label in the top left corner |
| `series` | at most 32 values 0 to 100, oldest first | a line behind the texts |

**Text modes come for free.** The user chooses how a key's title is laid
out (wrapped, on one line, fitted by shrinking, or scrolling sideways), its
size and alignment, in the key's Text appearance; the agent's renderer
applies that to your `label` and `detail` as to any other key's text
([key-text.md](../../key-text.md)), at the resolution of each device, and a
new label is fitted again. A plugin sends text only: it does not measure,
shorten, wrap or animate it, and needs no permission or API version for
this.

Every member is optional; `KeyState::new()` leaves the key as the user
designed it. A state out of bounds is replaced by the failure
`invalid-state` before it leaves the plugin. `Err(PluginError)` shows the
key's static face with an error mark.

- **A state handler only reads.** Inside it `storage_get`,
  `storage_keys`, `settings`, `now` and the log answer; everything else
  (`fetch`, `notify`, `open_url`, `storage_set`, `storage_delete`,
  `publish_variable`, `random_bytes`, `run_context`…) is `NotPermitted`,
  whatever the plugin is granted. Do the work in the action, store what
  the key should show, and read it here.
- **Budgets.** At most 8 capability calls (then `Throttled`), 4 KiB of
  output, a tenth of an action's fuel, 250 ms of CPU between host calls.
  A handler that panics or never returns ends the host process, like an
  action that does, and counts for the crash-loop breaker.
- **Parameters** are the key's, checked like an action's. Keys that show
  different things (the cells of a board) differ by a parameter.
- **Manifest**: the action entry declares `"state": { "refresh":
  "onEvents" | "every", "intervalSeconds": n }`, and the plugin the
  `device` permission with `"state": true`
  ([permissions.md](permissions.md)). The manifest's rule that `state`
  needs an action whose `node.effect` is `readOnly` (the default) still
  holds: an action that `opens` or `modifies` cannot declare state in
  this version. The module's descriptor does
  not change, and a plugin whose keys merely look better with state keeps
  `"pluginApi": "^1.0"`: an older agent never calls the handler.
- **Testing**: `MockHost::key_state(&plugin, "focus", json!({}))` returns
  the `StateOutcome`.

**Who asks, and when.** The agent (plugin API 1.2), through the pipeline
of its built-in live key states, for a key that is **shown on a device
now**:

- when the key appears, and after something happened in DeckPilot (a key
  press, a finished action or workflow, a project switch, a reload). A
  plugin's own action is therefore followed by a fresh state; a plugin
  cannot ask for a redraw itself;
- with `"refresh": "every"`: again each `intervalSeconds`;
- never more often than every **5 seconds** for the same key, whatever
  the manifest says, and only when the state changed is anything sent to
  the device.

One state call of a plugin runs at a time; its keys take turns. While the
plugin runs an action its keys keep what they showed. A state call has
2 s of wall clock.

| Situation | The key shows |
| --- | --- |
| A valid state | the plugin's description |
| First state pending | the static face with the loading mark |
| `Err`, an invalid state, a time-out, a crash | the static face with the error mark; after 3 in a row the key is asked again only after 5 s, 10 s … 5 min |
| `device:state` not granted or revoked, no state handler | the static face, without a mark |
| Plugin disabled, uninstalled, stopped by the crash-loop breaker, or plugins off | what any unavailable action shows; no state call |
| A control without a display, an agent below API 1.2 | no state call |

The configuration app's layout preview runs no providers (for built-in
modules either), so it shows the static face; devices, physical and
simulated, show the state.

## Controls

From plugin API 1.3 an action declares which controls of a device it can
be bound to: per control kind (`key`, `encoder`, `touchZone`), the inputs
it reacts to. Without a declaration the agent offers it on **keys only**
(press, release, click, double click, long press), never on dials or
touch zones; the manifest's `device.requires` grants nothing on its own.

```rust
Plugin::new("acme.pager", "1.0.0")
    .requires_api(ApiLevel { major: 1, minor: 3 })
    .action(Action::new("next", next).controls(
        Controls::new()
            .key([Input::Press, Input::Click])
            .encoder([Input::RotateClockwise, Input::RotateCounterClockwise]),
    ))
```

- **Rules** (`Plugin::validate`, the same as the manifest validator's): at
  least one kind, no kind without inputs, only inputs the kind has (no
  rotation on a key, no tap on a dial), at most 8 inputs per kind. Keys:
  `press`, `release`, `click`, `double-click`, `long-press`; encoders:
  `rotate-clockwise`, `rotate-counter-clockwise` and, for pressable dials,
  the key inputs; touch zones: `tap`, `long-touch`, `swipe`.
- **API level**: declaring controls needs `requires_api` 1.3;
  `Plugin::validate` refuses them below it. Agents below plugin API 1.3
  refuse a manifest with `controls`.
- **Manifest**: the action entry declares the same `controls`
  ([manifest.md](manifest.md#controls)); `Controls::to_json()` gives the
  manifest form and `manifest_mismatches` compares the two (per kind, as
  sets). The controls are not part of the module's descriptor.
- An action runs once per event: a dial's rotation arrives as one
  `rotate-clockwise` or `rotate-counter-clockwise` per event, without the
  number of detents.

## A complete handler

From `plugins/examples/developer-tools/src/lib.rs`:

```rust
fn json_format(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let text: String = args.get("json")?;
    let style: String = args.get("style")?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| PluginError::new("invalid-input", format!("not JSON: line {}", e.line())))?;
    let formatted = if style == "compact" {
        serde_json::to_string(&value)
    } else {
        serde_json::to_string_pretty(&value)
    }
    .map_err(|_| PluginError::new("invalid-input", "the JSON does not serialize"))?;
    if formatted.len() > 16 * 1024 {
        return Err(PluginError::new("output-too-large", "the result is too long"));
    }
    Ok(Output::message(format!("{} bytes of JSON", formatted.len())).with("result", formatted))
}
```

## How large a plugin may be

The manifest's own limits are the real ones: up to **64 actions**, each
with up to **32 parameters** and **32 outputs**, and a 128 px icon of up to
**256 KiB**. The agent sends the app a plugin's actions (each with its full
definition and manifest entry) in pages and its icon in chunks
(`getPluginActions`, `getPluginIcon`; see
[agent-integration.md](../agent-integration.md#answers-of-any-size)), so
the 64 KiB of one IPC message (`deckpilot_ipc::framing::MAX_MESSAGE_BYTES`)
no longer limits how many actions or outputs a plugin has:
`apps/agent/tests/plugins_e2e.rs` lists, shows and edits a plugin with
40 actions of 20 outputs and a 200 KiB icon.

What must still fit one message:

- **one action** with its definition and manifest entry. Only an action
  with dozens of parameters that each list dozens of long choice options
  comes near; such an action is listed without its definition, and the
  editors do not offer it (the agent logs it).
- **the rest of the manifest** (settings fields, credential slots,
  variables, permissions) with the plugin's details. Within the manifest's
  limits this needs very long descriptions or defaults to go over; the
  app then shows the refusal `answerTooLarge`.

Keep definitions lean anyway: declare only the outputs an action really
returns, and keep labels and descriptions short (48 and 300 characters at
most; the description is repeated in the manifest and in the definition).
An app older than paged answers receives what fits one message: the icon,
then the definitions of the largest plugins are left out of its answers.
