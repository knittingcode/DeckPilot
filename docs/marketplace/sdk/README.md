# DeckPilot plugin SDK

Status: **experimental.** Plugins are off in every DeckPilot release build
so far (the agent needs the `plugins` cargo feature, which turns them on,
[agent-integration.md](../agent-integration.md)). Plugin API 1.1, guest
ABI 1 and the manifest version 1 described here are what `main` implements
today; they can still change before plugins ship. Nothing here is
published: not the SDK crate, not the JavaScript package, not the example
plugins.

A DeckPilot plugin is **one WebAssembly module implementing guest ABI 1**,
shipped as `plugin.wasm` next to a `manifest.json` in a signed package
([artifact-contract.md](../artifact-contract.md)). The agent runs it in a
separate, OS-sandboxed host process with no WASI: the module has no file
system, network, clock, randomness, environment or process of its own.
Everything it reaches outside itself is a capability request that the
agent's permission broker checks against what the user approved, on every
call.

Today the only SDK that builds such a module is the **Rust SDK**,
`deckpilot-plugin-sdk` (`plugins/runtime/guest/plugin-sdk`, crate version
0.1.0). The JavaScript SDK (`plugins/sdk-js`) is type declarations only:
**JavaScript to wasm (QuickJS) is not available yet**, so no JavaScript
plugin can be built or run ([JavaScript](#javascript)).

## Pages

| Page | Contents |
| --- | --- |
| [manifest.md](manifest.md) | Every `manifest.json` field, with the rules the schema and the agent apply |
| [actions.md](actions.md) | Declaring actions in code and in the manifest: parameters, defaults, outputs, results |
| [permissions.md](permissions.md) | Permission kinds, what each grants in ABI 1, justifications, consent and revocation |
| [capabilities.md](capabilities.md) | HTTP, credentials as handles, settings, variables, storage, notifications, log: the `Context` API |
| [dialogs.md](dialogs.md) | Standard dialogs (plugin API 1.5): confirm, alert, form and selection, described as data and drawn by DeckPilot |
| [localization.md](localization.md) | Translations (plugin API 1.6): `locales/<tag>.json`, `t:` manifest texts, the fallback chain, `ctx.locale()` and `ctx.t(...)` |
| [lifecycle.md](lifecycle.md) | What runs when: init and shutdown hooks, hosts, restarts; what ABI 1 does not have |
| [errors-and-versions.md](errors-and-versions.md) | Error codes and how they reach the user; ABI, API level, SDK and `engines` versions |
| [building.md](building.md) | Building, reproducible builds, packaging, testing, local installation, size limits |

## Quick start

```sh
rustup target add wasm32-unknown-unknown     # once, for the pinned toolchain
cd plugins/runtime
cargo test --locked -p hello-world           # native unit tests
cd ../..
plugins/examples/build.sh hello-world        # -> plugins/runtime/target/examples/hello_world.wasm
```

The smallest plugin (abridged from `plugins/examples/hello-world/src/lib.rs`):

```rust
use deckpilot_plugin_sdk::prelude::*;

pub fn plugin() -> Plugin {
    Plugin::new("acme.hello-world", "0.1.0").action(
        Action::new("greet", greet).param(Param::text("name").required().max_length(64)),
    )
}

deckpilot_plugin_sdk::plugin!(plugin);

fn greet(_ctx: &mut Context<'_>, args: &Args<'_>) -> ActionResult {
    let name: String = args.get("name")?;
    let greeting = format!("Hello, {}!", name.trim());
    Ok(Output::message(greeting.clone()).with("greeting", greeting))
}
```

plus a `manifest.json` that declares the same id, version and action
([manifest.md](manifest.md)), a `LICENSE` and a 128×128 PNG icon. Start
from one of the examples:

| Example | Shows | Permissions |
| --- | --- | --- |
| [`plugins/examples/hello-world`](../../../plugins/examples/hello-world) | The minimum: one pure action with one parameter | none |
| [`plugins/examples/developer-tools`](../../../plugins/examples/developer-tools) | Pure utilities, brokered HTTPS to one origin, a credential slot used only as a handle, settings, storage, own variables, notifications, typed errors | `network:api.github.com` (GET), `credentials:githubToken`, `notifications` |
| [`plugins/examples/hello-weather`](../../../plugins/examples/hello-weather) | A pure action and one brokered request; an init hook | `network:api.open-meteo.com` (GET) |

## SDK coverage

Every capability of ABI 1 on `main`, its typed wrapper in
`deckpilot-plugin-sdk` (`src/context.rs` unless noted), where it is
documented, and what tests it. Every wrapper has a doc example that runs
under `cargo test` (against `testing::MockHost`).

| Capability (op) | Wrapper | Doc | Tests |
| --- | --- | --- | --- |
| `http.fetch` | `Context::fetch(Request)`, `Context::http_get`; `Request::{get, post, put, patch, delete, head, header, body, json}`; `Response::{is_success, header, text, json}` | [capabilities.md](capabilities.md#http) | SDK doc test; `developer-tools` unit tests; host `sdk_e2e.rs` (hello-weather); agent `plugins_e2e.rs`, `plugins_examples.rs` (canned HTTP) |
| `http.fetch` with `auth` (credential handle) | `SecretHandle::slot`, `Request::credential(handle, AuthScheme)`, `Request::bearer` | [capabilities.md](capabilities.md#credentials) | SDK unit test `secret_handles_are_keys_only`; agent `plugins_e2e.rs` (bearer, header, query, redaction); `plugins_examples.rs` (bearer on the wire, never in the answer) |
| `credentials.has` | `Context::has_credential(&SecretHandle) -> bool` | [capabilities.md](capabilities.md#credentials) | SDK doc and unit tests; `developer-tools` unit tests; agent `plugins_e2e.rs` |
| `settings.get` | `Context::settings() -> Settings` (`get::<T>`, `text`, `integer`, `boolean`, `contains`, `as_map`, `to_json`) | [capabilities.md](capabilities.md#settings) | SDK doc and unit tests; agent `plugins_e2e.rs` (defaults, validation); `plugins_examples.rs` (`defaultRepo`) |
| `variables.publish`, `variables.clear` | `Context::publish_variable`, `clear_variable`, `clear_variables` | [capabilities.md](capabilities.md#variables) | SDK doc test; agent `plugins_e2e.rs` (types, undeclared ids, disable); `plugins_examples.rs` (`openPrs`, `buildStatus` resolved) |
| `storage.get`, `storage.set`, `storage.delete` | `Context::storage_get`, `storage_get_as::<T>`, `storage_set`, `storage_delete` | [capabilities.md](capabilities.md#storage) | SDK doc and unit tests; agent `plugins_e2e.rs` (isolation, quota, revocation); `plugins_examples.rs` (`last-values`) |
| `storage.keys`, `storage.usage` | `Context::storage_keys`, `Context::storage_usage -> StorageUsage` | [capabilities.md](capabilities.md#storage) | SDK doc and unit tests; agent `plugins_e2e.rs`; `plugins_examples.rs` (`storageBytes`) |
| `notify` | `Context::notify(title, body)` | [capabilities.md](capabilities.md#notifications) | SDK doc test; `developer-tools` unit test (`build_status_publishes_stores_and_notifies_on_change`) |
| `time.now` (API 1.1) | `Context::now() -> Now` (`unix_ms`, `utc_offset_minutes`, `unix_seconds`, `local_seconds`, `local_minutes_of_day`, `local_weekday`, `millis_since`) | [capabilities.md](capabilities.md#time) | SDK doc test; broker `time_random.rs`; host `sdk_e2e.rs`; agent unit tests (`plugins::ambient`) and `plugins_e2e.rs` |
| `random.bytes` (API 1.1) | `Context::random_bytes(n)`, `random_u64`, `random_below(bound)` | [capabilities.md](capabilities.md#randomness) | broker `time_random.rs` (byte budget); host `sdk_e2e.rs`; agent unit tests (`plugins::ambient`) and `plugins_e2e.rs` |
| `log` | `Context::log(LogLevel, ..)`, `debug`, `info`, `warn`, `error` | [capabilities.md](capabilities.md#log) | SDK doc test and `logging_is_best_effort_and_bounded` |
| Exports (`dp_abi_version`, `dp_alloc`, `dp_invoke`, `dp_describe`, `dp_features`) | `plugin!` | [actions.md](actions.md) | host `sdk_e2e.rs`, `engine.rs` (`network_kinds_reach_only_guests_that_read_them`); every agent e2e |
| Network kind of a failed `http.fetch` | `CapabilityError::network_kind -> Option<NetworkErrorKind>`; `?` reports its code | [capabilities.md](capabilities.md#errors) | SDK unit test `network_failures_keep_the_built_in_codes`; agent `plugins_e2e.rs` (canned DNS, connection, TLS, blocked address and redirect failures) |
| `dp_state` (API 1.2) | `Action::state(StateHandler)`, `KeyState` builder (`label`, `detail`, `indicator`, `progress`, `toggle`, `accent`, `glyph`, `badge`, `series`), `StateOutcome` | [actions.md](actions.md#key-state) | ABI `state.rs`; broker `key_state.rs`; host `engine.rs` (`key_state_output_is_bounded_and_typed`) and `sdk_e2e.rs` (read only, budgets, crashes); SDK doc test and `tests/runtime.rs` |
| `dp_init`, `dp_shutdown` | `Plugin::on_init`, `Plugin::on_shutdown` | [lifecycle.md](lifecycle.md) | SDK `tests/runtime.rs`; host `lifecycle.rs`, `sdk_e2e.rs` |
| Testing | `testing::MockHost` (`new`, `allow_all`, `deny_all`, `invoke`, `context`, `requests`), `testing::manifest_mismatches` | [building.md](building.md#testing) | SDK doc tests and `tests/runtime.rs`; used by every example |

## JavaScript

`plugins/sdk-js` (`@deckpilot/plugin-sdk`, `private: true`, version 0.1.0,
never published) holds the type declarations of the #191 design. **No
DeckPilot host can run JavaScript plugins: compiling JavaScript to an
ABI 1 module (QuickJS-NG inside the module) is not available yet**
(`plugins/runtime/README.md`, "QuickJS"). The declarations describe the
design, not ABI 1; [its README](../../../plugins/sdk-js/README.md) lists
where they differ. Write plugins in Rust until the engine guest exists.
