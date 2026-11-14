# Hello Weather: a sample DeckPilot plugin

A small, complete plugin built with the Rust plugin SDK
(`deckpilot-plugin-sdk`, plugin API 1.0, guest ABI 1). Use it as the
template for your own plugin.

> **Experimental.** Plugins are not enabled in any DeckPilot release yet.
> The SDK, the manifest schema and the runtime are the implementation of
> the marketplace design (#191) and can still change.

| Action | What it does | Capabilities |
| --- | --- | --- |
| `greet` | Greets someone (`name`, `style`) | none (pure) |
| `current-temperature` | Current temperature of a city (`city`, `units`) from Open-Meteo | one `GET` to `https://api.open-meteo.com` |

## Files

| File | Purpose |
| --- | --- |
| `manifest.json` | Metadata, permissions, actions, outputs and error codes. Valid against `plugins/schemas/plugin-manifest.schema.json` (a copy of the #191 schema). The agent reads only this file to learn what the plugin is; it never runs plugin code for that |
| `src/lib.rs` | The plugin: `plugin()` declares it, `deckpilot_plugin_sdk::plugin!(plugin)` exports it |
| `src/tests.rs` | Native unit tests: definition equals manifest, schema validity, both actions against a mock host |
| `assets/icon-128.png` | The required 128×128 icon |

## Build

The crate is a `cdylib` for `wasm32-unknown-unknown`; it also builds as an
`rlib` so that its unit tests run natively. It is a member of the
`plugins/runtime` workspace, which pins the toolchain and the lock file.

```sh
cd plugins/runtime
rustup target add wasm32-unknown-unknown       # once
cargo test --locked -p hello-weather            # native unit tests
cargo build --locked -p hello-weather --target wasm32-unknown-unknown --profile guest
# -> target/wasm32-unknown-unknown/guest/hello_weather.wasm
```

The end-to-end tests in `plugins/runtime/crates/host/tests/sdk_e2e.rs`
build the same module (from `crates/test-fixtures/build.rs`) and run it in
the real sandboxed host under the real supervisor and broker.

## Writing a plugin

### 1. Declare the plugin and its actions

```rust
use deckpilot_plugin_sdk::prelude::*;

pub fn plugin() -> Plugin {
    Plugin::new("acme.hello-weather", "0.1.0")      // = manifest id, version
        .on_init(check_cities)                       // optional lifecycle hook
        .action(
            Action::new("greet", greet)              // = manifest action id
                .param(Param::text("name").required().max_length(64))
                .param(Param::choice("style", &["plain", "excited"]).default("plain")),
        )
}

deckpilot_plugin_sdk::plugin!(plugin);
```

The action ids and parameters must equal `contributes.actions` in
`manifest.json`. When the host loads the module it reports the SDK's
descriptor (`dp_describe`), and the supervisor refuses a module whose
action ids differ from the manifest's. Keep a unit test like
`definition_matches_the_manifest` in `src/tests.rs`.

Parameter kinds:

| SDK | Manifest `kind.type` | Read as |
| --- | --- | --- |
| `Param::text(k)` (`.max_length(n)`) | `text` (`maxLength`) | `String` |
| `Param::integer(k)` (`.min(a)`, `.max(b)`) | `integer` (`min`, `max`) | `i64` (or `f64`) |
| `Param::boolean(k)` | `boolean` | `bool` |
| `Param::choice(k, &[..])` | `choice` (`options`) | `String` |
| `Param::number(k)` | none in manifest version 1 | `f64` |
| `Param::secret(k)` | none in manifest version 1 | `SecretHandle` |

Parameters are optional unless `.required()`; `.default(v)` fills an absent
optional parameter. Before your handler runs, the SDK refuses unknown
parameters, wrong kinds, out-of-range values and options that are not
listed, with the shared code `invalid-parameters` and a message that names
the parameter (never its value).

### 2. Handle an action

```rust
fn greet(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let name: String = args.get("name")?;            // typed and checked
    let style: String = args.get("style")?;          // default filled in
    Ok(Output::message(format!("Hello, {name}.")).with("greeting", name))
}
```

`args.get::<Option<T>>(key)` reads an optional parameter without a
default. Output keys should be the manifest's `node.outputs[].key`; the
agent keeps only declared outputs.

### 3. Return typed errors

```rust
return Err(PluginError::new("weather-service-error", "HTTP 503").retryable());
```

A code is either a shared code (`deckpilot_plugin_sdk::codes`, for example
`invalid-parameters`, `not-permitted`, `throttled`) or one of your own,
declared in `contributes.errorCodes`. An undeclared or malformed code
becomes `invalid-result`. Do not panic: a panic aborts the module, the host
process exits and the supervisor restarts it with backoff (1, 2, 4 … 60 s;
five crashes in ten minutes disable the plugin).

### 4. Use capabilities through the broker

The module has no network, file system, clock or process access of its
own. Everything goes through `Context`, and the agent's permission broker
checks every call against what the user approved:

```rust
let response = ctx.fetch(Request::get(url).header("Accept", "application/json"))?;
```

* Declare a `network` permission with the **exact** host and methods in
  `manifest.json`. The user approves it at install and can revoke it at
  any time; a revocation applies to the next call.
* Without the grant the call fails with a `CapabilityError` whose message
  names the origin and the missing permission, for example
  `http.fetch to https://api.open-meteo.com was denied by the permission
  broker (origin not granted). Declare a network permission …`. With `?`
  it becomes the shared code `not-permitted`.
* A non-2xx status is a `Response`, not an error: map it to your own codes.
* Secrets are handles: `Request::get(url).bearer(&handle)` asks the agent
  to inject the secret of a credential slot; the plugin never sees it, and
  the broker sends it only to the slot's approved origins and schemes.
* `ctx.info(..)` and friends write to the plugin log (always allowed,
  rate limited, best effort). `ctx.storage_get/set/delete` use the
  plugin's own storage.

### 5. Lifecycle

`Plugin::on_init` runs once when the host loads the module (a failure
refuses the load); `Plugin::on_shutdown` runs once on a clean shutdown.
Neither has capabilities.

### 6. Test

Run actions natively with `deckpilot_plugin_sdk::testing::MockHost`:

```rust
let mut host = MockHost::deny_all();                       // nothing granted
let out = host.invoke(&plugin(), "current-temperature", json!({"city": "tokyo"}));
// -> Failed { code: "not-permitted", .. }
```

`MockHost::new(|request| ...)` scripts replies, and `host.requests()` shows
what the plugin asked for.

## Versioning

| Number | Where | This sample |
| --- | --- | --- |
| Plugin version | `manifest.json` `version`, `Plugin::new` | 0.1.0 |
| Plugin API level | `engines.pluginApi`, `deckpilot_plugin_sdk::API_LEVEL` | ^1.0 |
| SDK version | `engines.sdk`, `deckpilot_plugin_sdk::SDK_VERSION` | 0.1.0 |
| Guest ABI | `deckpilot_plugin_sdk::ABI_VERSION` (checked at compile time) | 1 |

## Not covered yet

Packaging and signing (`.dpplugin`, registry), install consent UI, and the
agent-side network executor are separate work; the end-to-end tests use a
stub executor and check the broker's decisions only. Manifest version 1 has
no parameter kinds for `number` or `secret`, so this sample does not use
them (the SDK test guest in `plugins/runtime/guest/sdk-test-guest` does).
