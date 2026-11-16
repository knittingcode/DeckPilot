# First-party plugins

DeckPilot's own plugins (`deckpilot.*`), built by the application's
release build and shipped **inside the installers** as app components
(`docs/marketplace/module-migration.md` §4). They are also submitted to
`knittingcode/deckpilot-plugins` by pinned commit for registry updates;
that is a separate step, not part of this directory.

Experimental, like all of the plugin marketplace. Three plugins are bundled:
`deckpilot.github` (class `migration`, [github/README.md](github/README.md)),
to which a plugin-enabled agent routes the built-in `github.*` action
types once a computer's migration is cut over
(`docs/marketplace/module-migration.md` §6 and §9, P9),
`deckpilot.gitlab` (class `migration`, [gitlab/README.md](gitlab/README.md)),
built and pinned but not routed until its cut-over (P11), and
`deckpilot.notifications` (class `default`, the first of the default set,
[notifications/README.md](notifications/README.md)), seeded on every
computer and routed through the alias block `notifications.*` (P10). The published release
pipeline does not package plugins at all (see "Packaging" below).

## Layout

| Path | What |
| --- | --- |
| `bundled.json` | The plugins a release bundles: `{"path": "<dir>", "class": "default" \| "migration"}` |
| `github/` | `deckpilot.github`, class `migration`: the replacement of the built-in `github` module, checked against it by `crates/deckpilot-modules/tests/contracts.rs` and `apps/agent/tests/plugins_github.rs` |
| `gitlab/` | `deckpilot.gitlab`, class `migration`: the replacement of the built-in `gitlab` module, for GitLab.com or a self-managed GitLab the user enters and allows; checked against the built-in by `crates/deckpilot-modules/tests/contracts.rs` and `apps/agent/tests/plugins_gitlab.rs` |
| `support/` | `deckpilot-first-party-support`: a library the plugins for a user-entered server share (URLs without a URL parser, the time limit counted with `Date` headers, text helpers). Compiled into each plugin's module; never bundled by itself |
| `notifications/` | `deckpilot.notifications`, class `default`: the replacement of the built-in `notifications` module, checked against it by `crates/deckpilot-modules/tests/contracts.rs` and `apps/agent/tests/plugins_notifications.rs` |
| `<name>/` | One plugin: `Cargo.toml` (a member of the `plugins/runtime` workspace), `src/`, `manifest.json` (id `deckpilot.<name>`), `LICENSE` and its icons in `assets/` |
| `build.sh` | Builds the listed plugins to wasm (`wasm32-unknown-unknown`, `guest` profile, reproducible on one platform) and stages them with their pins |
| `test-fixtures/` | **Test fixtures, never shipped**: `example-default` (`deckpilot.example-default`, class `default`, one action, one low-tier permission), `legacy-fixture` (`deckpilot.legacy-fixture`, class `migration`: the replacement of the agent's fixture legacy module in the module migration tests, with a low-, a medium- and a high-tier permission) and their own `bundled.json`, used only with `build.sh --test-fixtures` |

## What the build produces

```sh
plugins/first-party/build.sh --out target/bundled-plugins            # the release set
plugins/first-party/build.sh --out target/bundle --test-fixtures     # tests, dry runs
```

```text
<out>/bundled/
  pins.json                              id, version, class, size and sha256 of every file
  deckpilot.<name>/manifest.json
  deckpilot.<name>/plugin.wasm
  deckpilot.<name>/LICENSE
  deckpilot.<name>/assets/icon-128.png   the icons the manifest names, nothing else
```

Never a `.dpplugin`, never an archive of its own, never a signature: a
bundled plugin is trusted by the **pins compiled into the agent**
(option B of §4.4), plus the app bundle's code seal on macOS (option A).

## How the pins are trusted

1. `build.sh` writes `bundled/pins.json`.
2. The agent is compiled with `DECKPILOT_BUNDLED_PINS=<out>/bundled/pins.json`:
   `crates/deckpilot-plugin-bundled/build.rs` copies the file into the
   build and `include_bytes!` embeds it. Without the variable (every build
   today) nothing is pinned and nothing is seeded.
3. `tools/release-pipeline/check-bundled-plugins.sh <out>/bundled <agent>`
   checks the staged files against `pins.json` and `pins.json` against
   the agent's `--print-bundled-pins`.
4. At start-up or unlock the agent seeds the bundled plugins from the
   application's resources (`apps/agent/src/plugins/bundled.rs`), checking
   every file against the compiled pins, never against the `pins.json`
   next to them, and checks the installed copy again at every load.

## Packaging

`deckpilot-dev package` with `DECKPILOT_RELEASE_PLUGINS=1` builds the
plugins, the agent with the `plugins` feature and their pins, and the
plugin host from `plugins/runtime`, then bundles with
`apps/desktop/src-tauri/tauri.plugins.conf.json` on top of
`tauri.release.conf.json`: the host as an external binary next to the
agent, `bundled/` as a resource. On macOS the host is signed first with
its one entitlement and copied into `Contents/MacOS` instead
(`tauri.plugins.macos.conf.json`; [docs/releasing.md](../../docs/releasing.md),
"Packaging the plugins"). Without the variable (the default) the
package is exactly what it was. The published release workflow
(`.github/workflows/release.yml`) is unchanged and packages neither the
host nor bundled plugins until the owner enables the marketplace.
