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
  `dp_shutdown`, `dp_features`). Use it once, at the root of the `cdylib` crate. The
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

## Keep the definitions small

Every action's full definition (parameters, kinds, outputs) travels in the
agent's `listPlugins` and `getPlugin` answers, and `getPlugin` also
carries the whole manifest and the icon. An IPC message is at most
**64 KiB** (`deckpilot_ipc::framing::MAX_MESSAGE_BYTES`); an answer over it
cannot be sent, and the app then cannot list or show the plugins (a test
plugin with 18 actions of 20 outputs each went over). Until the IPC carries
definitions in pages or on demand, keep a plugin's `getPlugin` answer under
about half of that:

- at most about **16 actions**, and fewer when they have many outputs;
- at most about **8 outputs** and **8 parameters** per action; declare only
  the outputs an action really returns;
- short labels and descriptions (the limits are 48 and 300 characters;
  the descriptions are repeated in the manifest and in each definition);
- choice options: a handful, not dozens;
- an icon of a few KiB (it is sent base64-encoded).

For scale: Developer Tools (7 actions, 1-8 outputs and 0-3 parameters
each, a 1 KiB icon) answers `getPlugin` in about 20 KB and `listPlugins`
in about 9 KB; `apps/agent/tests/plugins_examples.rs` fails if either
exceeds 32 KiB.
