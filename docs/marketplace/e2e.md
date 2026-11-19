# The marketplace end to end through the app

The automated version of [local-testing.md](local-testing.md), through the
configuration app instead of the agent's IPC alone. One scenario
(`apps/desktop/src-tauri/tests/support/marketplace.rs`), two drivers:

| Driver | Test | Platforms | What runs |
| --- | --- | --- | --- |
| UI | `tests/marketplace_gui.rs` | Linux, Windows | the built app (debug, `VITE_DECKPILOT_PLUGINS=1`) driven over WebDriver by `tauri-driver` (WebKitWebDriver on Linux, msedgedriver on Windows); the app starts the agent itself |
| Command layer | `src/marketplace_e2e.rs` | macOS, Linux, Windows | the app's commands through Tauri's mock runtime, with the frontend's argument names; the test starts the agent |

**The macOS GUI is not driven here**: there is no WebDriver for WKWebView. On
macOS the scenario runs at the command layer only: the same commands, the
same agent, IPC, storage and sandbox, without the webview. The *published*
macOS and Windows apps are driven from outside through the operating
system's accessibility layer by the release smoke test
([release-validation.md](../release-validation.md#6-release-smoke-the-published-release)).

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
| 1 | Start the app with an isolated data dir; it starts the agent | Linux: UI (the password typed in the unlock window). Windows: the test starts the agent and unlocks it over its IPC before the app starts (fixture, agent IPC), because a second WebView2 window of the app does not load under msedgedriver; the app then opens no prompt |
| 2 | Open the Marketplace (toolbar › Marketplace), after checking the device editor (the default view) | UI |
| 3 | Discover Hello Weather | UI |
| 4 | Inspect its details | UI |
| 5 | Install 1.0.0 with consent | UI |
| 6 | Grant permissions (enable, consent dialog) | UI |
| 7 | Configure | UI (Configure: the setting chosen and saved, the credential's secret stored); an invalid value is refused (app IPC); the plugin's storage is written into `plugins.db` (fixture) because the sample never writes storage |
| 8 | Assign the action to a key | app IPC (`create_action`, `save_layout`): drag and drop in the Layout Designer is not driven |
| 9 | Execute the action | UI (Run in Installed) and agent IPC (key presses on the simulated MK.2) |
| 10 | Execute it in a workflow | app IPC (`save_workflow`, `start_workflow`) |
| 11 | Verify the results | app IPC, the canned HTTP log |
| 12 | Restart the agent and the app | Linux: UI (toolbar › Settings opens the settings window, the test switches to it: Background Agent › Stop agent; the app is closed and started again, which starts the agent). Windows: the settings window's `stop_agent` over app IPC, as a second WebView2 window does not load under msedgedriver; the test then starts and unlocks the agent as in step 1 |
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

## Update failures and protections

A second command-layer scenario
(`plugin_updates_fail_safely_and_need_consent_through_the_app_commands` in
`src/marketplace_e2e.rs`, steps in `tests/support/marketplace_lifecycle.rs`)
uses the same fixture with versions of the sample made for it: 1.0.0;
1.1.0, which adds `notifications` and broadens the network grant to `POST`;
1.2.0, which declares an action its module does not implement. It runs
wherever the command layer runs (macOS, Linux, Windows); the GUI is not
driven. Its report is `<artifacts>/commands-update-failures-steps.txt`.

| # | Step | Expected |
| --- | --- | --- |
| 1 | Install 1.0.0 with consent, enable, configure, bind to keys | runs from the simulated MK.2 |
| 2 | Update while the registry's timestamp signature fails | `registryUnavailable`, nothing downloaded |
| 3 | Update with a package that is not the pinned one | `invalidPackage` |
| 4 | Update whose download is cut off | `registryUnavailable` |
| 5 | The agent killed mid-update (`DECKPILOT_TEST_PLUGIN_CRASH=update:files_unpacked`), restarted | recovery stages 1.1.0, which still waits for consent; 1.0.0 runs; declined |
| 6 | Update to 1.1.0 without consent, then with consent to part of it | staged, `consentRequired`; 1.0.0 runs, nothing granted |
| 7 | Restart, decline | still staged after the restart; declined: files gone, nothing granted |
| 8 | Update with consent to everything it asks for | applied in one step, data kept |
| 9 | Update to 1.0.0, install 1.0.0 again, rollback after the registry revoked 1.0.0 | `invalidState`, `alreadyInstalled`, `revoked` |
| 10 | Serve the older, validly signed site | refused (`rollback`), seq kept |
| 11 | Update to 1.2.0 (module refused by the host) | `broken`, 1.1.0 stays active and runs |
| 12 | The registry revokes 1.1.0; uninstall | quarantined, nothing runs or fetches, cannot be enabled; uninstall removes storage, settings, credentials and files; profiles and the encrypted configuration intact |

After each failed or refused step the test checks that the running version
is still enabled with its grants, settings, credential and storage, runs
from its key, that the version directories on disk are exactly the
expected ones and that the store's journal and staging are empty.

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

## The live registry (smoke test)

Three more command-layer tests in `src/marketplace_e2e.rs` check the
production trust chain. They are ignored like the others and, in addition,
do nothing unless `DECKPILOT_LIVE_REGISTRY=1`, so `--ignored` alone (as in
CI) never reaches the network. Run them by hand, e.g. after publishing a
plugin or before a release; they cover macOS, Linux and Windows alike (the
command layer), the GUI is not driven.

| Test | Agent | Registry | Expected |
| --- | --- | --- | --- |
| `live_registry_installs_and_runs_a_plugin_with_the_embedded_roots` | production roots embedded, none given at run time | the agent's default, `https://knittingcode.github.io/deckpilot-plugins/`, packages from the GitHub releases of knittingcode/deckpilot-plugins | `deckpilot.memory` 0.1.0 discovered, installed with consent to `device:state`, enabled, configured, run from keys of the simulated MK.2 and uninstalled |
| `live_registry_is_refused_by_an_agent_trusting_other_roots` | no embedded roots, throwaway roots given (`DECKPILOT_PLUGIN_REGISTRY_ROOTS`) | the live one, as above | refused on its signatures (`keys/1.json`), nothing listed, downloaded or installed |
| `live_production_roots_refuse_a_registry_signed_with_other_keys` | production roots embedded, none given | the local throwaway registry of the scenario (no network) | refused the same way |

The live test passes no registry URL, roots, CA or canned HTTP: the agent
uses its embedded roots (the log says `registry root keys roots=embedded`),
the default registry URL and the system's TLS roots, and the plugin runs in
the real sandboxed host. The data directory is a temporary one with a
password-protected configuration, as in the scenario. Its step report is
`<artifacts>/commands-live-steps.txt`; the agent's log
(`agent-stderr-1.log`) has the verification
(`plugin registry verified: a new index registry=https://knittingcode.github.io/deckpilot-plugins/`)
and the download
(`plugin package downloaded; ... url=https://github.com/knittingcode/deckpilot-plugins/releases/download/...`).

Build the agent once more with the production roots embedded (the content
of `keys/roots.json` of knittingcode/deckpilot-plugins, also the repository
variable `DECKPILOT_REGISTRY_ROOT_PUBKEYS`), and copy it apart from the one
without:

```sh
gh api repos/knittingcode/deckpilot-plugins/contents/keys/roots.json --jq .content \
  | base64 -d > /tmp/dp-e2e/production-roots.json
mkdir -p /tmp/dp-e2e/live
DECKPILOT_REGISTRY_ROOT_PUBKEYS=/tmp/dp-e2e/production-roots.json \
  cargo build -p deckpilot-agent --features plugins,test-hooks --bin deckpilot-agent
cp target/debug/deckpilot-agent /tmp/dp-e2e/live/
export DECKPILOT_E2E_LIVE_AGENT_BIN=/tmp/dp-e2e/live/deckpilot-agent
```

(Build the one without embedded roots, `DECKPILOT_E2E_AGENT_BIN` above,
with the variable unset.) Then, with the plugin host and the sample as
above:

```sh
DECKPILOT_LIVE_REGISTRY=1 cargo test -p deckpilot-desktop --bin deckpilot-desktop \
  marketplace_e2e::live -- --ignored --nocapture --test-threads 1
```
