# First-party plugins

DeckPilot's own plugins (`deckpilot.*`), built by the application's
release build and shipped **inside the installers** as app components
(`docs/marketplace/module-migration.md` §4). They are also submitted to
`knittingcode/deckpilot-plugins` by pinned commit for registry updates;
that is a separate step, not part of this directory.

Experimental, like all of the plugin marketplace. The plugins of
`bundled.json` are bundled. The `migration`-class ones are the only
implementation of the former built-in `github`, `gitlab`, `jira`, `git`,
`docker`, `workspace` and `projects` modules (removed in WP-C5 and WP-C6):
their legacy action ids are unavailable with the reason until the plugin
serves. Among them `deckpilot.github` (class `migration`,
[github/README.md](github/README.md)), to which a plugin-enabled agent
routes the legacy `github.*` action types once a computer's migration is
cut over
(`docs/marketplace/module-migration.md` §6 and §9, P9),
`deckpilot.gitlab` (class `migration`, [gitlab/README.md](gitlab/README.md))
and `deckpilot.jira` (class `migration`, [jira/README.md](jira/README.md)),
routed the same way through the alias blocks `gitlab.*` and `jira.*`, for
GitLab.com or the server the user had entered and allows (P11), and
`deckpilot.notifications` (class `default`, the first of the default set,
[notifications/README.md](notifications/README.md)), seeded on every
computer and routed through the alias block `notifications.*` (P10), and
`deckpilot.workflows` (class `default`), which owns the Workflows screens.
Git, Docker, Workspace and Projects (plugin API 1.9) are routed since
WP-C1. The published release
pipeline does not package plugins at all (see "Packaging" below).

## Layout

| Path | What |
| --- | --- |
| `bundled.json` | The plugins a release bundles: `{"path": "<dir>", "class": "default" \| "migration"}` |
| `released-pins.json` | The pins of the published releases, per platform: the id, version and the size and sha256 of every file of each bundled plugin (see "Versions") |
| `docker/` | `deckpilot.docker`, class `migration` (plugin API 1.9): the replacement of the former built-in `docker` module over `tool.run` and resource handles; [docker/README.md](docker/README.md) lists the differences; checked by its unit tests and `apps/agent/tests/plugins_docker.rs` |
| `git/` | `deckpilot.git`, class `migration` (plugin API 1.9): the replacement of the former built-in `git` module over `tool.run` and resource handles; [git/README.md](git/README.md) lists the differences; checked by its unit tests and `apps/agent/tests/plugins_git.rs` |
| `github/` | `deckpilot.github`, class `migration`: the replacement of the former built-in `github` module, checked against its frozen contract (`crates/deckpilot-modules/legacy/`) by `crates/deckpilot-modules/tests/contracts.rs` and `apps/agent/tests/plugins_github.rs` |
| `gitlab/` | `deckpilot.gitlab`, class `migration`: the replacement of the former built-in `gitlab` module, for GitLab.com or a self-managed GitLab the user enters and allows; checked against its frozen contract by `crates/deckpilot-modules/tests/contracts.rs` and `apps/agent/tests/plugins_gitlab.rs` |
| `jira/` | `deckpilot.jira`, class `migration`: the replacement of the former built-in `jira` module, for the Jira Cloud site or Data Center the user enters and allows (it has no host of its own); checked against its frozen contract by `crates/deckpilot-modules/tests/contracts.rs` and `apps/agent/tests/plugins_jira.rs` |
| `support/` | `deckpilot-first-party-support`: a library the plugins for a user-entered server (`gitlab`, `jira`, `github` for GitHub Enterprise Server) share (URLs without a URL parser, the time limit counted with `Date` headers, text helpers). Compiled into each plugin's module; never bundled by itself |
| `notifications/` | `deckpilot.notifications`, class `default`: the replacement of the built-in `notifications` module, checked against it by `crates/deckpilot-modules/tests/contracts.rs` and `apps/agent/tests/plugins_notifications.rs` |
| `<name>/` | One plugin: `Cargo.toml` (a member of the `plugins/runtime` workspace), `src/`, `manifest.json` (id `deckpilot.<name>`), `LICENSE`, its icons (and plugin API 1.8 visual assets, PNG or JPEG) in `assets/` and its translations in `locales/<tag>.json` (plugin API 1.6) |
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
  deckpilot.<name>/assets/icon-128.png   the icons and visual assets (PNG, JPEG) the manifest names
  deckpilot.<name>/locales/<tag>.json    the translations the manifest declares
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

## Versions

A released version of a bundled plugin never changes. An installation
replaces its installed copy only with a **higher** bundled version
(`apps/agent/src/plugins/bundled.rs`), and checks the installed copy
against the pins compiled into the agent at every load: a plugin rebuilt
with other files under a version an earlier release shipped (a changed
SDK or support crate is enough) is **broken** on every installation
upgraded from that release. So every change to a bundled plugin's files
gets a new version, in `manifest.json`, `Cargo.toml`, the plugin's entry in
`plugins/runtime/Cargo.lock` and the plugin's README ("Versions"): a patch
version for a rebuild or a fix, a minor version for new actions,
settings, permissions or a higher plugin API (0.2.0 of `deckpilot.github`
for GitHub Enterprise).

`released-pins.json` records what each release shipped, per platform
(`macos`, `linux`, `windows`; `build.sh` is reproducible on one platform,
and both macOS apps are built on the same runner). It does not depend on
where the checkout is: Cargo hashes a member outside the workspace root
(`../first-party/...` of `plugins/runtime`) by its absolute path into the
code, so `build.sh` builds in a copy of the workspace rooted at `plugins/`,
and a release's pins recorded on a developer's Mac match the runner's.
Format 1:

```json
{ "releasedPinsVersion": 1,
  "releases": [ { "release": "0.9.0", "platform": "macos", "source": "<where the pins come from>",
                  "plugins": [ { "id": "deckpilot.<name>", "version": "X.Y.Z",
                                 "files": { "<path>": { "sha256": "<hex>", "size": 0 } } } ] } ] }
```

`tools/release-pipeline/check-released-pins.sh <out>/bundled/pins.json`
fails when a built plugin has the id and version of one a release recorded
for this platform, with other files (another file set, size or sha256),
and names the files; a new version, or the same files, pass. It runs in
`check-bundled-plugins.sh` (so in the release and *Packaging* builds and
`deckpilot-dev package`) and in the bundle dry run of the *Plugins in the
agent* workflow, and its cases are in
`tools/release-pipeline/test-bundled-plugins.sh`. Only `macos` has
recorded pins so far (0.9.0, from the published app); a platform without
any passes with a note, and the macOS checks catch a source change for
every platform.

Release preparation appends the release's pins
([docs/releasing.md](../../docs/releasing.md), "Version management"):

```sh
plugins/first-party/build.sh --out target/bundled-plugins
tools/release-pipeline/check-released-pins.sh --record X.Y.Z target/bundled-plugins/bundled/pins.json \
  --platform macos --source "plugins/first-party/build.sh at the release commit"
```

`--record` runs the same check first, changes nothing when the same pins
are recorded again and refuses other pins for a release and platform
already recorded.


`deckpilot-dev package` with `DECKPILOT_RELEASE_PLUGINS=1` builds the
plugins, the agent with the `plugins` feature and their pins, and the
plugin host from `plugins/runtime`, then bundles with
`apps/desktop/src-tauri/tauri.plugins.conf.json` on top of
`tauri.release.conf.json`: the host as an external binary next to the
agent, `bundled/` as a resource. On macOS the host is signed first with
its one entitlement and copied into `Contents/MacOS` instead
(`tauri.plugins.macos.conf.json`; [docs/releasing.md](../../docs/releasing.md),
"Packaging the plugins"). Without the variable (the default) the
package has neither. The release workflow (`.github/workflows/release.yml`)
and the *Packaging* workflow's package checks build every installer this
way, and `tools/ci/check-packages.sh` requires the host and exactly the
plugins of `bundled.json` in each.
