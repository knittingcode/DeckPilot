# Distribution contract: application and plugins

Status: **decided** for the separation. The registry repository
[`knittingcode/deckpilot-plugins`](https://github.com/knittingcode/deckpilot-plugins)
exists, with its workflows and Pages site; its environments' secrets and
the production registry keys are **pending** the owner's approval of the
trust configuration (U-10, U-12), so no signed registry is live yet.
Applies to the experimental plugin marketplace; nothing here changes what
an application release contains today.

DeckPilot ships through two channels that never share a release, a key, a
secret or an environment.

| | Application | Plugins |
| --- | --- | --- |
| Repository | `knittingcode/deckpilot` (formerly `knittingcode/DeckPilot`) | `knittingcode/deckpilot-plugins` |
| What it holds | application source and application releases | the static plugin registry and the independently distributed plugin packages |
| Served from | GitHub Releases of `knittingcode/deckpilot` (URLs keep the `knittingcode/DeckPilot` spelling installed clients require) | registry metadata: GitHub Pages, <https://knittingcode.github.io/deckpilot-plugins/v1/>; packages: GitHub Releases of `knittingcode/deckpilot-plugins` |
| Signed with | the application updater key (minisign, `TAURI_SIGNING_PRIVATE_KEY`) and the Apple Developer ID | the registry roles: root keys (offline), index key and timestamp key (`crates/deckpilot-plugin-registry/PUBLISHING.md`) |
| Signing environments | `production-release` | `registry-publish` (release tags `*--v*`: index and timestamp keys) and `registry-timestamp` (`main` only: the timestamp key, daily refresh), in `knittingcode/deckpilot-plugins` only |
| Trusted because | the updater signature, against the public key built into the app | the registry signatures, against the root keys built into the app (`DECKPILOT_REGISTRY_ROOT_PUBKEYS`, a repository variable read at build time; [agent-integration.md](agent-integration.md#trusted-root-keys)) |
| Lifecycle | application versions, tags `v*` of this repository | plugin versions and registry snapshots, on their own schedule |

## `knittingcode/deckpilot`: the application only

An application release (`.github/workflows/release.yml`,
[docs/releasing.md](../releasing.md)) contains exactly:

- the installers: `.dmg`, `.AppImage`, `.deb`, `.rpm` (stable releases),
  `.msi`, NSIS `-setup.exe`;
- the updater archives (`DeckPilot_<arch>.app.tar.gz` and the packages
  above) with their updater signatures (`.sig`);
- the update manifest (`latest.json`, or `prerelease.json` for a
  pre-release);
- `release-meta.json` (commit, channel, signing record), the release notes
  and `SHA256SUMS`.

Nothing else. In particular never a plugin package (`*.dpplugin`), never
registry metadata (`roots.json`, key sets, index snapshots,
`timestamp.json`, details, `*.minisig`) and never a plugin build output.
The update manifest names, for each platform, that platform's application
updater package only.

The plugin host (`deckpilot-plugin-host`) is a component of the
application: when it is bundled, it ships *inside* the installers (like the
agent), signed and updated with the application. It is never a release
asset of its own and never an independently installable package.

### macOS: the plugin host's entitlement

The host is the only DeckPilot executable with an entitlement, and only
in packages built with `DECKPILOT_RELEASE_PLUGINS=1`:
`com.apple.security.cs.allow-unsigned-executable-memory`, from
[`apps/desktop/src-tauri/entitlements/plugin-host.entitlements`](../../apps/desktop/src-tauri/entitlements/plugin-host.entitlements),
on `Contents/MacOS/deckpilot-plugin-host` alone. The application and the
agent keep the hardened runtime with **no** entitlements, and a package
without the variable is exactly what it was (no host, no entitlement).

- **Why.** wasmtime 50.0.1 (Cranelift) maps the compiled plugin code with
  `mmap` and makes it executable with `mprotect`, without `MAP_JIT`. The
  GitHub build is signed with the hardened runtime, which kills such a
  process (`SIGKILL`, `CODESIGNING`, "Invalid Page") the moment it runs a
  guest, without an entitlement and also with
  `com.apple.security.cs.allow-jit`, which covers only `MAP_JIT` memory.
  `allow-unsigned-executable-memory` is the entitlement under which it runs
  (experiment E2 of the distribution feasibility study, PR #263,
  `experiments/distribution/macos-sandbox-plugin-host`, and
  `plugins/runtime/scripts/macos-hardened-runtime.sh`, which CI runs on
  macOS).
- **The host's own sandbox is unchanged.** It still confines itself with
  `sandbox_init` and the deny-default profile
  (`plugins/runtime/crates/host/src/sandbox/macos.rs`) before it reads a
  plugin byte; the entitlement only lets wasmtime make its own pages
  executable. Under the hardened runtime with the entitlement the host's
  process, sandbox and SDK tests pass against the signed binary: guests
  run, files, the network and `exec` stay denied
  (`macos-hardened-runtime.sh`).
- **Why not `allow-jit`.** It is narrower (only `MAP_JIT` regions may be
  executable) but wasmtime has no configuration or feature that allocates
  its code with `MAP_JIT`, and its own mapping cannot be turned into one
  (`mmap` with `MAP_JIT | MAP_FIXED` over it fails with `EINVAL`). It
  works only with a custom loader in the host: compile with
  `Engine::precompile_module`, copy the image into a `MAP_JIT` region the
  host maps itself, load it with the `unsafe` `Module::deserialize_raw`,
  and switch the thread's write protection
  (`pthread_jit_write_protect_np`) in a `CustomCodeMemory`. A probe of
  that passed on arm64 (guests run, traps work, `sandbox_init` still
  denies files), but on arm64 a `MAP_JIT` region cannot be `mprotect`ed,
  so the whole image (data included) is executable and W^X is only per
  thread, any code in the process can switch it, and the Intel build could
  not be tested here (Rosetta does not enforce the hardened runtime). That
  is new `unsafe` code in the process that holds untrusted code, for a
  marginal gain against an attacker who already runs code in it; not done.
  wasmtime's Pulley interpreter would need no entitlement at all, at the
  cost of interpreted plugins; not evaluated beyond that.
- **How it is applied.** The Tauri bundler signs every external binary
  with the entitlements of the whole app, so on macOS the host is not an
  external binary: `deckpilot-dev package` stages it at
  `apps/desktop/src-tauri/binaries/macos/deckpilot-plugin-host`, signs it
  with the app's identity (`APPLE_SIGNING_IDENTITY`), the hardened
  runtime, the identifier `io.github.knittingcode.deckpilot.plugin-host`
  and exactly that file (`tools/ci/sign-macos-plugin-host.sh`), and
  `tauri.plugins.macos.conf.json` copies it into `Contents/MacOS` as it is
  (`bundle.macOS.files`). The bundler then signs the app around it (no
  `--deep`, so the host's signature stays) and notarizes the whole.
- **Enforcement.** `tools/ci/check-macos-entitlements.sh` allows exactly
  that entitlement dictionary on exactly `Contents/MacOS/deckpilot-plugin-host`
  and none on any other Mach-O file (the app, the agent, a second copy of
  the host anywhere); any other or additional entitlement on the host
  fails. `.github/scripts/verify-macos-app.sh` (release) and
  `deckpilot-dev package` (after bundling, when signing) run it;
  `tools/ci/test-check-macos-entitlements.sh` tests it (Packaging
  workflow, macOS). No Tauri configuration may set
  `bundle.macOS.entitlements` (`deckpilot-dev` unit test).

Registry and plugin signing never happen in this repository's workflows,
in any environment: no `deckpilot-registry` signing or publishing command,
no registry secret, no `registry-*` environment, and never with the
application's signing secrets or the `production-release` environment.
This repository's CI only *tests* the registry tooling, with throwaway keys
made inside the tests.

## `knittingcode/deckpilot-plugins`: the registry and the packages

- **GitHub Pages** serves the signed registry metadata under
  <https://knittingcode.github.io/deckpilot-plugins/v1/> (`site/v1/` of
  `PUBLISHING.md`). The agent trusts it **only through its signatures**,
  verified against the root keys built into the application, never because
  of where it is hosted: a compromised or impersonated Pages site can at
  most withhold updates (which the timestamp's expiry turns into a visible
  `Stale` state), not change what clients accept.
- **GitHub Releases** of that repository hold the signed plugin packages
  (`<id>-<version>.dpplugin`), pinned by sha256 and size in the signed
  index; one release per plugin version.
- Its workflows sign only in its own environments, with its own keys:
  `publish.yml` signs the index and the timestamp in `registry-publish`
  (release tags `*--v*`), and `refresh-timestamp.yml` re-signs the
  timestamp daily in `registry-timestamp` (`main` only, the timestamp key
  only). The secrets of both and the production registry keys are pending
  the owner's approval of the trust configuration (U-10 key ceremony,
  U-12). Until then the tooling has been used only with throwaway keys in
  tests.

## Independent lifecycles, separate keys

- A plugin is published, yanked or revoked without an application release,
  and an application release never waits for, contains or re-signs a
  plugin.
- The application updater key never signs registry metadata or a plugin
  package, and no registry key ever signs an application update. Losing or
  rotating one never affects the other.
- No secret and no environment is shared between the two repositories'
  release workflows.
- The application embeds only *public* registry keys (the root role). They
  reach the build as the **repository variable**
  `DECKPILOT_REGISTRY_ROOT_PUBKEYS` of `knittingcode/deckpilot` (never a
  secret: nothing in this repository can sign with them), holding the
  content of `keys/roots.json` of the registry repository. `release.yml`
  passes it to the agent build only when it is set; the build checks it
  and fails on a malformed value; unset, nothing is embedded and the
  application uses no registry. A release build with embedded roots
  cannot be given other roots at run time (`--plugin-registry-roots`
  overrides them in debug and test builds only). Steps for the owner:
  [agent-integration.md](agent-integration.md#enabling-the-public-registry-in-release-builds).
- The embedded roots are only the anchor: a routine root rotation is a
  key set signed by the old and the new roots, which clients follow
  without an application release. Only the loss or compromise of the
  embedded roots needs one, with new roots (`PUBLISHING.md`, "Root loss
  or compromise").

## Enforcement in this repository

Run by *Release pipeline tests* (`.github/workflows/release-tests.yml`,
job `gate`) on every pull request that touches a workflow or
`tools/release-pipeline/` (not a required check):

- `tools/release-pipeline/check-assets.sh` (stage A's last check and stage
  B's first): the release's assets must be exactly the application's
  allowlist above; a plugin package or registry file is refused by name,
  anything else as "not an application asset".
- `tools/release-pipeline/publish.sh` and `verify-published.sh`: each
  platform's update in the manifest must be that platform's application
  updater package of this tag; a URL to a plugin package, registry
  metadata, a signature, a checksum file or another platform's package is
  refused. Tests: `tools/release-pipeline/test-publication.sh`.
- `tools/release-pipeline/check-release-separation.py`: no workflow
  publishes plugin packages or registry metadata, runs a registry signing
  or publishing command, signs a plugin package, or uses a registry secret
  or environment, and none mixes them with the application's signing
  secrets or `production-release`. Tests:
  `tools/release-pipeline/test-release-separation.sh` with the fixtures in
  `tools/release-pipeline/fixtures/release-separation/`.
