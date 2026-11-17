# Plugin artifact contract (MVP)

Status: **decided** (owner-delegated, 2026-11-14). Applies to the
experimental plugin marketplace (`docs/marketplace/architecture.md` on
`docs/plugin-marketplace-design`, #191). Nothing here is enabled in a
release build. Where packages are published, and that they never ship
with an application release: [distribution.md](distribution.md).

## Decision

The executable artifact of a DeckPilot plugin is **one WebAssembly module
implementing plugin ABI 1** (`plugins/runtime/crates/abi`), shipped in the
package as **`plugin.wasm`**, and declared in `manifest.json` as

```json
"runtime": { "kind": "wasm", "abi": 1 }
```

The agent loads that module, unchanged, into a sandboxed
`deckpilot-plugin-host` process (wasmtime, no WASI) through the supervisor.
Nothing in the agent ever interprets plugin code.

## Why

#191 proposed TypeScript compiled to `plugin.js`, run by a QuickJS-NG engine
module the app ships. The runtime that exists (#202 spikes, `plugins/runtime`)
is engine-agnostic: the host loads any ABI 1 module, and the only working SDK
is the Rust SDK (`deckpilot-plugin-sdk`), whose sample `hello-weather` builds
to such a module. QuickJS-NG in wasm is not built yet (runtime README,
"QuickJS"). Making the module itself the artifact:

- keeps one runtime contract for every source language: the host, the
  supervisor, the broker and the ABI never learn what language a plugin was
  written in;
- needs no engine shipped with the app and no engine/bundle version matrix;
- lets the package be checked statically before install (below).

## JavaScript plugins (future)

A JavaScript or TypeScript plugin will be built by the SDK into a single ABI 1
module that contains QuickJS-NG and the bundle (the engine guest described in
the runtime README). It is then an ordinary `plugin.wasm` with
`"runtime": {"kind": "wasm", "abi": 1}`; the runtime contract does not change
and there is deliberately no `"kind": "js"`.

`plugin.js` packages (the #191 contract: no `runtime`, `plugin.js` in the
package) are **not accepted**. The package reader refuses them with
`PackageError::UnsupportedRuntime`, whose message names this contract; the
publishing tool refuses to build or add them. No format is ever converted or
reinterpreted silently.

## Manifest

| Field | Rule |
| --- | --- |
| `runtime` | Optional in the schema, so manifests written for #191 still parse and validate. `kind` is `wasm` (the only kind), `abi` is `1` (anything else is a validation error, `runtime.abi`). Unknown fields are refused |
| absent `runtime` | The #191 JavaScript contract; `Manifest::runtime_contract()` is `LegacyJs` and packages of it are refused |
| `entry` | Informational source path (`src/lib.rs`); the agent never reads it |

The schema (`crates/deckpilot-plugin-manifest/schema/` and its copy in
`plugins/schemas/`) gained `runtime`; the change is additive.

## Package

Required files: `manifest.json`, `plugin.wasm`, `LICENSE`, plus the files the
manifest names (icons, key icons, screenshots, README, CHANGELOG). Nothing
else. `plugin.wasm` is at most 4 MiB (the package limits stay as in #191:
5 MiB compressed, 5 MiB per file, 10 MiB unpacked).

At verify time (`deckpilot_plugin_package::verify_package`, after the size and
sha256 pins and the manifest checks) the module is validated with `wasmparser`
without being compiled or run (`deckpilot_plugin_package::wasm`):

- a valid core module under the WebAssembly 2.0 feature set (no components,
  threads, shared or 64-bit memory, multi-memory, GC or exceptions);
- the only import allowed is `deckpilot_v1.host_call: (i32, i32) -> i64`
  (no WASI, no other host function);
- required exports: `memory` (its single memory), `dp_abi_version: () -> i32`,
  `dp_alloc: (i32) -> i32`, `dp_invoke: (i32, i32) -> i64`; the optional
  `dp_describe: () -> i64`, `dp_init: () -> i32`, `dp_shutdown: () -> ()`,
  `dp_features: () -> i64` and `dp_state: (i32, i32) -> i64` must have
  exactly these types;
- at most one table.

Everything the module reaches outside itself goes through that one import
as a capability request (`plugins/runtime/crates/abi/src/capability.rs`):
`http.fetch` (with an optional credential handle), `notify`, `log`,
`storage.get`, `storage.set`, `storage.delete`, `storage.keys`,
`storage.usage`, `credentials.has`, `settings.get` and `url.open`. The
last five are compatible ABI 1 additions; a module that does not use them
is unchanged.
A secret never crosses the import: the module names a credential slot and
the agent injects its value (agent-integration.md, "Plugin data").

A start function is allowed: the host runs it under the invocation deadline
and fuel limits, as it does today. The host repeats the import and export
checks when it loads the module, the supervisor compares the module's
descriptor with the manifest's action ids (`HostConfig::expect_actions`), and
the agent re-verifies the installed files against their integrity record and
re-validates the module before every load.

## Building a package

```sh
cd plugins/runtime
cargo build --locked -p hello-weather --target wasm32-unknown-unknown --profile guest
cd ../..
cargo run -p deckpilot-plugin-registry --features publish --bin deckpilot-registry -- \
  package --plugin-dir plugins/examples/hello-weather \
  --module plugins/runtime/target/wasm32-unknown-unknown/guest/hello_weather.wasm \
  --out hello-weather.dpplugin
```

`package` takes `manifest.json`, `LICENSE` and the named files from the plugin
directory and the module as `plugin.wasm`, and runs every reader check on the
result (deterministic output). `add-version` then publishes it into a registry
working directory (`crates/deckpilot-plugin-registry/PUBLISHING.md`).
