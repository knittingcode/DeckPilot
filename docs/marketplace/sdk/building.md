# Building, packaging, testing

## Project layout

A plugin is a Rust crate of crate type `cdylib` (the module) and `rlib`
(so its unit tests run natively), a member of the `plugins/runtime`
workspace (its `Cargo.lock`, toolchain pin and `guest` profile):

```text
plugins/examples/developer-tools/
├── Cargo.toml          # [lib] crate-type = ["cdylib", "rlib"], workspace = "../../runtime"
├── manifest.json       # manifest.md
├── LICENSE             # required in every package
├── README.md
├── assets/icon-128.png # icons.128: 128×128 PNG, at most 256 KiB
└── src/lib.rs          # plugin() + deckpilot_plugin_sdk::plugin!(plugin)
```

Add the crate to `members` and `default-members` of
`plugins/runtime/Cargo.toml`. The SDK is not published; depend on it with
`deckpilot-plugin-sdk.workspace = true`. Dependencies must build for
`wasm32-unknown-unknown` without WASI or JavaScript glue (no `getrandom`
with a JS backend, no `wasm-bindgen`): the package check refuses any
import other than `deckpilot_v1.host_call`.

## Build

```sh
rustup target add wasm32-unknown-unknown   # once, for the pinned toolchain (rust-toolchain.toml)
plugins/examples/build.sh                  # both examples
plugins/examples/build.sh developer-tools  # one
```

`plugins/examples/build.sh [--target-dir DIR] [--out DIR] [PACKAGE...]`
runs, in `plugins/runtime`,

```sh
cargo build --locked -p hello-world -p developer-tools \
  --target wasm32-unknown-unknown --profile guest
```

with the checkout and the Cargo home remapped
(`--remap-path-prefix <plugins>=/deckpilot/plugins`,
`<CARGO_HOME>=/cargo`, through `CARGO_ENCODED_RUSTFLAGS`) and `RUSTFLAGS`
and compiler wrappers cleared, copies each module to `--out` (default
`plugins/runtime/target/examples/<crate>.wasm`) and prints its sha256.
The plain `cargo build` above (without the script) gives a working module
too, with your checkout's paths in its panic locations.

The `guest` profile (`plugins/runtime/Cargo.toml`): `opt-level = "s"`,
`lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`.
Hello World is about 190 KB, Developer Tools about 300 KB (the limit is
4 MiB for `plugin.wasm`, `deckpilot_plugin_package::wasm::MAX_MODULE_BYTES`).

## Reproducible builds

The same source, toolchain and lock file give **byte-identical modules on
the same platform**, whatever the checkout path, user or target
directory: the toolchain is pinned, dependencies are locked, symbols are
stripped, paths are remapped and no environment flags leak in.

CI checks it: the `reproducible` job of
`.github/workflows/plugin-runtime.yml` builds both examples twice, each
time into a new empty target directory, on Ubuntu, macOS and Windows, and
fails unless the sha256 of each module is the same both times. It prints
the hashes.

**Builds on different operating systems are not byte-identical** (a known
finding; the cause, somewhere in the per-platform host toolchain, is not
analysed here). **Ubuntu (`ubuntu-latest`) is the reference
platform**: a published package's `plugin.wasm` should be the Ubuntu
build, and a verifier should rebuild on Ubuntu. For example, when the
examples were added, two clean builds on macOS arm64 (toolchain 1.99.1)
gave, both times:

```text
7eb0d27a88844b71c2a728e745b5d5a7dd539fa67c084a44d20133e617a70728  hello_world.wasm
4a359883daf8b4c0ebc0d822ec386d41888e2d2cbe3267c13d4ac87138cba381  developer_tools.wasm
```

Any change to an example, the SDK or the lock file changes them; the
`reproducible` CI job prints the current hashes for each platform.

## Package

```sh
cargo run --locked -p deckpilot-plugin-registry --features publish --bin deckpilot-registry -- \
  package --plugin-dir plugins/examples/developer-tools \
  --module plugins/runtime/target/examples/developer_tools.wasm \
  --out developer-tools-0.1.0.dpplugin
# acme.developer-tools 0.1.0 packaged to developer-tools-0.1.0.dpplugin (sha256 …, … bytes)
```

(the same with `hello-world` / `hello_world.wasm`). `package` takes
`manifest.json`, `LICENSE` and every file the manifest names (icons, key
icons, screenshots, `README.md`, `CHANGELOG.md`) from the plugin
directory and the module as `plugin.wasm`, then runs every reader check
on the result: manifest validation, the size limits (5 MiB compressed,
5 MiB per file, 10 MiB unpacked) and the static module check (only the
`deckpilot_v1.host_call` import, the required exports,
[artifact-contract.md](../artifact-contract.md)). The output is
deterministic: the same inputs give the same bytes (the example e2e test
packages each example twice and compares).

**Packaging is not publishing.** Packages are published only through the
separate registry repository `knittingcode/deckpilot-plugins`, never with
an application release, and never from this repository's workflows
([distribution.md](../distribution.md)). Do not attach a `.dpplugin` to a
DeckPilot release: the release checks refuse it.

## Testing

### Native unit tests

`cargo test --locked -p <your-plugin>` in `plugins/runtime`. The SDK's
`testing` module runs actions without wasm and without an agent:

```rust
use deckpilot_plugin_sdk::testing::{MockHost, manifest_mismatches};

#[test]
fn definition_matches_the_manifest() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!("../manifest.json")).unwrap();
    assert_eq!(manifest_mismatches(&plugin(), &manifest), Vec::<String>::new());
}

#[test]
fn greet_needs_no_capability() {
    let mut host = MockHost::deny_all();                 // nothing granted
    let out = host.invoke(&plugin(), "greet", serde_json::json!({"name": "Ada"}));
    assert!(matches!(out, ActionOutcome::Ok { .. }));
    assert!(host.requests().is_empty());
}
```

- `MockHost::new(|request| CallResult)` scripts the agent's replies per
  `CapabilityRequest`; `allow_all()` answers everything with `null` (an
  empty 200 for fetches); `deny_all()` denies everything like an ungranted
  permission.
- `host.invoke(&plugin, action, json_params)` runs one invocation exactly
  as the host would (init, parameter checks, handler, result checks).
- `host.requests()` lists what the plugin asked for, in order: assert the
  URL, method, headers, that a credential is a handle (`auth`), what was
  stored and published.
- `host.context()` gives a `Context` for testing helpers directly.
- `manifest_mismatches(&plugin, &manifest)` lists every difference between
  code and manifest.

Developer Tools' `src/tests.rs` scripts a whole agent (settings, storage in
memory, credential state, canned GitHub answers) in about 40 lines
(`github_host`).

### Schema check

Validate `manifest.json` against `plugins/schemas/plugin-manifest.schema.json`
with the `jsonschema` dev-dependency (`manifest_is_valid_against_the_schema`
in both examples). `deckpilot-registry package` runs the full validation.

### In the real agent and host

`apps/agent/tests/plugins_examples.rs` installs both examples from a local
registry signed with throwaway keys, enables them with exactly their
declared permissions and invokes every action through a real agent and
the sandboxed host, with canned HTTP responses (never the real GitHub API):

```sh
cargo test -p deckpilot-agent --features plugins --test plugins_examples --locked -- --nocapture
```

It builds the host and the modules itself, or takes
`DECKPILOT_E2E_PLUGIN_HOST` and `DECKPILOT_E2E_WASM_DIR`. Copy it for your
own plugin's end-to-end test; [local-testing.md](../local-testing.md) is
the same flow by hand with the app (replace the sample's paths with your
plugin's directory and module).

## Checklist

- [ ] `manifest.json` validates; `manifest_mismatches` is empty.
- [ ] Every permission is one you call, with a justification a user
      understands; nothing broader than needed (exact hosts, only the
      methods you use).
- [ ] Every own error code is declared; no panics on input.
- [ ] Output keys are declared in `node.outputs`; texts stay under 16 KiB.
- [ ] The `getPlugin` answer stays small: at most about 16 actions, 8
      outputs and 8 parameters each, short descriptions
      ([actions.md](actions.md#keep-the-definitions-small); the IPC limit
      is 64 KiB per message).
- [ ] `plugins/examples/build.sh` twice gives the same sha256.
- [ ] `LICENSE` and a 128×128 PNG icon are present; `package` succeeds.
