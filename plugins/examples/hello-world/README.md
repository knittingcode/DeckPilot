# Hello World

The smallest DeckPilot plugin: one action, one parameter, no permissions.
Built with the Rust plugin SDK (`deckpilot-plugin-sdk`, plugin API 1.0,
guest ABI 1). Experimental, like all of the plugin marketplace; not
published.

| Action | Parameters | Outputs | Capabilities |
| --- | --- | --- | --- |
| `greet` | `name` (text, required, at most 64 characters) | `greeting` | none |

`greet` answers `Hello, <name>!`. It asks the agent for nothing, so the
manifest's `permissions` is empty and the plugin is enabled without
granting anything.

## Files

| File | Purpose |
| --- | --- |
| `manifest.json` | Id `acme.hello-world`, version 0.1.0, the action, its parameter and output |
| `src/lib.rs` | `plugin()` declares it, `deckpilot_plugin_sdk::plugin!(plugin)` exports it; unit tests at the end |
| `assets/icon-128.png` | The icon, drawn by `../make-icon.py` |
| `LICENSE` | Apache-2.0, packaged with the plugin |

## Build, test, package

```sh
rustup target add wasm32-unknown-unknown                      # once
(cd plugins/runtime && cargo test --locked -p hello-world)    # native tests
plugins/examples/build.sh hello-world                         # -> plugins/runtime/target/examples/hello_world.wasm
cargo run --locked -p deckpilot-plugin-registry --features publish --bin deckpilot-registry -- \
  package --plugin-dir plugins/examples/hello-world \
  --module plugins/runtime/target/examples/hello_world.wasm --out hello-world-0.1.0.dpplugin
```

`apps/agent/tests/plugins_examples.rs` packages it, installs it from a
local registry, enables it and runs `greet` through the real agent and the
sandboxed host.

The SDK documentation is in [`docs/marketplace/sdk/`](../../../docs/marketplace/sdk/README.md);
[`../developer-tools`](../developer-tools) shows every capability.
