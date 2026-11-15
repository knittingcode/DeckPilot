# The marketplace end to end through the app

The automated version of [local-testing.md](local-testing.md), through the
configuration app instead of the agent's IPC alone. One scenario
(`apps/desktop/src-tauri/tests/support/marketplace.rs`), two drivers:

| Driver | Test | Platforms | What runs |
| --- | --- | --- | --- |
| UI | `tests/marketplace_gui.rs` | Linux, Windows | the built app (debug, `VITE_DECKPILOT_PLUGINS=1`) driven over WebDriver by `tauri-driver` (WebKitWebDriver on Linux, msedgedriver on Windows); the app starts the agent itself |
| Command layer | `src/marketplace_e2e.rs` | macOS, Linux, Windows | the app's commands through Tauri's mock runtime, with the frontend's argument names; the test starts the agent |

**The macOS GUI is not driven**: there is no WebDriver for WKWebView. On
macOS the scenario runs at the command layer only: the same commands, the
same agent, IPC, storage and sandbox, without the webview.

Both use a real agent built with `plugins,test-hooks`, the real sandboxed
plugin host, the sample plugin (`plugins/examples/hello-weather`, packaged
with a setting and a credential slot added), and:

- an isolated data directory (`DECKPILOT_DATA_DIR` in a temporary
  directory) with a **password-protected** configuration, so no keychain
  or credential store item is created;
- a registry made with the publishing tool (`deckpilot-registry`, called
  as a library with the CLI's arguments) and throwaway keys, served over
  HTTPS by a local server whose throwaway CA only the agent under test
  trusts (`DECKPILOT_TEST_REGISTRY_CA`);
- canned HTTP for `api.open-meteo.com` (`DECKPILOT_TEST_PLUGIN_HTTP`): the
  plugin never reaches the network;
- a simulated Stream Deck MK.2 in the agent (`--mock-devices mk2`). The app
  starts the agent without arguments; with the test hooks the agent also
  reads them from `DECKPILOT_TEST_AGENT_ARGS`.

Each step records how it was done: **UI**, **app IPC** (the command the
frontend would call, invoked in the app's webview with
`window.__TAURI_INTERNALS__.invoke`, or through the mock runtime), **agent
IPC** (the simulated keys) or **fixture** (written or read from outside by
the test). The report is `<artifacts>/<driver>-steps.txt`.

## Steps and how the UI test does them

| # | Step | UI test |
| --- | --- | --- |
| 1 | Start the app with an isolated data dir; it starts the agent | UI: the password typed in the unlock window |
| 2 | Open Settings › Extensions › Plugins | UI |
| 3 | Discover Hello Weather | UI |
| 4 | Inspect its details | UI |
| 5 | Install 1.0.0 with consent | UI |
| 6 | Grant permissions (enable, consent dialog) | UI |
| 7 | Configure | UI (Configure: the setting chosen and saved, the credential's secret stored); an invalid value is refused (app IPC); the plugin's storage is written into `plugins.db` (fixture) because the sample never writes storage |
| 8 | Assign the action to a key | app IPC (`create_action`, `save_layout`): drag and drop in the Layout Designer is not driven |
| 9 | Execute the action | UI (Run in Installed) and agent IPC (key presses on the simulated MK.2) |
| 10 | Execute it in a workflow | app IPC (`save_workflow`, `start_workflow`) |
| 11 | Verify the results | app IPC, the canned HTTP log |
| 12 | Restart the agent and the app | UI (Settings › Background Agent › Stop agent; the app is closed and started again, which starts the agent) |
| 13 | Verify persistence | app IPC, agent IPC, `plugins.db` |
| 14 | Update to 1.1.0 (adds `notifications`) | UI (Updates › Check for updates › Update, consent dialog) |
| 15 | Verify state migration (storage, settings, credential, grants) | app IPC, `plugins.db` |
| 16 | Roll back | UI |
| 17 | Revoke the network permission | UI |
| 18 | Verify the denied operation | UI (Run is unavailable, with the reason), app IPC (`run_action` fails), agent IPC (the key fails); nothing reaches the canned HTTP |
| 19 | Disable | UI |
| 20 | Uninstall | UI (the confirmation, keeping neither settings nor credentials) |
| 21 | Verify cleanup: storage, credentials, settings gone, `plugin_known` kept | `plugins.db` read with the derived key, app IPC |
| 22 | Profiles, workflows and the encrypted configuration intact | app IPC and the SQLCipher database opened with its key |

The CI workflow is `.github/workflows/desktop-e2e.yml` (not a required
check); it uploads the step report, screenshots, the agent's and
tauri-driver's logs as artifacts.

## Running it locally

Build the artifacts (from the repository root):

```sh
cargo build -p deckpilot-agent --features plugins,test-hooks --bin deckpilot-agent
(cd plugins/runtime &&
  cargo build --locked -p deckpilot-plugin-host --bin deckpilot-plugin-host &&
  cargo build --locked -p hello-weather --target wasm32-unknown-unknown --profile guest)
mkdir -p /tmp/dp-e2e && cp target/debug/deckpilot-agent /tmp/dp-e2e/
export DECKPILOT_E2E_AGENT_BIN=/tmp/dp-e2e/deckpilot-agent
export DECKPILOT_E2E_PLUGIN_HOST=$PWD/plugins/runtime/target/debug/deckpilot-plugin-host
export DECKPILOT_E2E_WASM_DIR=$PWD/plugins/runtime/target/wasm32-unknown-unknown/guest
```

The command layer (any platform):

```sh
cargo test -p deckpilot-desktop --bin deckpilot-desktop marketplace_e2e -- --ignored --nocapture
```

The UI (Linux, Windows): build the app with the plugins page and copy it
out of `target/` (building the tests replaces `target/debug/deckpilot-desktop`
with a build that expects the dev server), install `tauri-driver` (and on
Linux `webkit2gtk-driver`; on Windows an msedgedriver matching the WebView2
runtime, named by `DECKPILOT_E2E_NATIVE_DRIVER`):

```sh
(cd apps/desktop && VITE_DECKPILOT_PLUGINS=1 pnpm tauri build --debug --no-bundle)
cp target/debug/deckpilot-desktop /tmp/dp-e2e/
cargo install tauri-driver --locked --version 2.1.0
export DECKPILOT_E2E_APP=/tmp/dp-e2e/deckpilot-desktop
cargo test -p deckpilot-desktop --test marketplace_gui -- --ignored --nocapture   # under xvfb-run on a headless Linux
```

`DECKPILOT_E2E_ARTIFACTS` sets where screenshots, logs and the step report
go; `DECKPILOT_E2E_KEEP=1` keeps the temporary directory. The test stops
the agent it started, also when it fails.
