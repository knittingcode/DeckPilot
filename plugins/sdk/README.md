# DeckPilot plugin SDKs

Where to start writing a DeckPilot plugin. **Experimental**: plugins are
off in every DeckPilot release, and nothing here is published (no crate,
no npm package, no registry release).

| | Where | Status |
| --- | --- | --- |
| Rust SDK `deckpilot-plugin-sdk` 0.1.0 (plugin API 1.0, guest ABI 1) | [`plugins/runtime/guest/plugin-sdk`](../runtime/guest/plugin-sdk) | Works: builds an ABI 1 `plugin.wasm` that the agent runs in its sandboxed host. Every ABI 1 capability has a typed wrapper |
| JavaScript SDK `@deckpilot/plugin-sdk` 0.1.0 | [`plugins/sdk-js`](../sdk-js) | Type declarations only. **JavaScript to wasm (QuickJS) is not available yet**: no JavaScript plugin can be built or run |
| Manifest schema | [`plugins/schemas/plugin-manifest.schema.json`](../schemas/plugin-manifest.schema.json) | Manifest version 1 |
| Examples | [`hello-world`](../examples/hello-world), [`developer-tools`](../examples/developer-tools), [`hello-weather`](../examples/hello-weather) | Built, packaged and run through the real agent in CI |

Documentation: [`docs/marketplace/sdk/`](../../docs/marketplace/sdk/README.md)
(manifest, actions, permissions, capabilities, lifecycle, errors and
versions, building and testing).

```sh
rustup target add wasm32-unknown-unknown
plugins/examples/build.sh                     # both examples, reproducibly, with sha256
(cd plugins/runtime && cargo test --locked -p hello-world -p developer-tools)
```
