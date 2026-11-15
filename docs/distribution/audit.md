# Distribution audit

What DeckPilot's packaging, signing, update and data handling do today, as
read from the code on `main` (at `21fc2de2`, after #229/#230), and from the
published `v0.6.0-alpha.3` release. Each finding cites the file and line it
comes from. Where the code and older design documents disagree, the code
is what is described here.

This is the input to the [target architecture](README.md) and the
[capability matrix](capability-matrix.md).

Contents:

1. [Summary](#1-summary)
2. [Bundle and application identity](#2-bundle-and-application-identity)
3. [The update client](#3-the-update-client)
4. [Release pipeline](#4-release-pipeline)
5. [The published v0.6.0-alpha.3 release](#5-the-published-v060-alpha3-release)
6. [Signing](#6-signing)
7. [The agent's lifecycle](#7-the-agents-lifecycle)
8. [Storage, data locations and upgrades](#8-storage-data-locations-and-upgrades)
9. [Plugin host](#9-plugin-host)
10. [Compatibility checks](#10-compatibility-checks)
11. [Runtime overrides that exist today](#11-runtime-overrides-that-exist-today)
12. [What works, what needs architectural change](#12-what-works-what-needs-architectural-change)

## 1. Summary

- There is exactly one distribution channel: **direct download from GitHub
  Releases**, updated in place by the signed Tauri updater. Nothing in the
  code knows how it was installed; every build assumes it owns its
  installation and may replace it.
- The Tauri updater already distinguishes package formats: the bundler
  patches the bundle type (`deb`, `rpm`, `appimage`, `msi`, `nsis`) into
  the binary and the updater looks for `{os}-{arch}-{bundle_type}` in the
  manifest first. That is a format, not a channel: a `.deb` installed from
  an APT repository would be indistinguishable from one downloaded from
  GitHub, and **would update itself from GitHub through `pkexec dpkg -i`**.
  This is the core gap the architecture closes.
- The update channel model (`stable` / `prerelease`, alpha, beta and rc in
  one prerelease channel) and the manifest naming (`latest.json` /
  `prerelease.json`) are sound and can be reused unchanged.
- Releases are canonical and immutable: `v0.6.0-alpha.3` is published with
  GitHub's `immutable: true`, and the pipeline publishes a fully verified
  draft in one step.
- macOS builds are Developer ID signed (team `PUB9L83UMZ`), hardened
  runtime, notarized and stapled. Windows and Linux packages carry no code
  signature; their only integrity protection is the minisign updater
  signature (for updates) and `SHA256SUMS` (for downloads).
- The plugin host is **not packaged**. The agent's `plugins` feature is off
  in release builds, the host lives in a separate Cargo workspace, and the
  macOS packaging check forbids entitlements, which a JIT-ing host will
  need.
- Schema migrations are **forward-only**, preceded by a verified encrypted
  backup; an older build refuses a newer database. Downgrading the app
  without restoring that backup is therefore not possible, and there is no
  restore command in the app.

## 2. Bundle and application identity

| Item | Value | Source |
| --- | --- | --- |
| Product name | `DeckPilot` | `apps/desktop/src-tauri/tauri.conf.json:3` |
| Version | `0.6.0-alpha.3` | `apps/desktop/src-tauri/tauri.conf.json:4` |
| Bundle identifier (app) | `io.github.knittingcode.deckpilot` | `apps/desktop/src-tauri/tauri.conf.json:5`, `crates/deckpilot-ipc/src/paths.rs:10` |
| Code signing identifier (agent) | `io.github.knittingcode.deckpilot.agent`, from an `Info.plist` linked into `__TEXT,__info_plist` | `apps/agent/Info.plist`, `apps/agent/build.rs:13-20` |
| Bundle targets | `"all"` (macOS `.app` + `.dmg`; Windows MSI + NSIS; Linux `.deb`, `.rpm`, AppImage) | `tauri.conf.json:34` |
| Hardened runtime | on, no entitlements | `tauri.conf.json:38-40`; enforced by `.github/scripts/verify-macos-app.sh:26-28` |
| udev rule | `/usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules` in `.deb` and `.rpm` only (not AppImage) | `tauri.conf.json:41-52`, `packaging/linux/70-deckpilot-streamdeck.rules` |
| Agent packaging | Tauri `externalBin` `binaries/deckpilot-agent`, added only by the release configuration | `apps/desktop/src-tauri/tauri.release.conf.json:4` |
| Updater artifacts | `createUpdaterArtifacts: true`, release configuration only | `tauri.release.conf.json:5` |
| Updater endpoint | `https://github.com/knittingcode/DeckPilot/releases/latest/download/latest.json` | `tauri.conf.json:64-66` |
| Updater public key | empty in the file; injected at build time | `tauri.conf.json:63`, `.github/workflows/release.yml:354-362` |
| Windows install mode | `passive` | `tauri.conf.json:68-70` |

Notes:

- `tauri.conf.json` alone builds no agent and no updater artifacts; only
  `--config src-tauri/tauri.release.conf.json` makes a shippable bundle
  (`release.yml:407`, `packaging.yml` "Build the .deb").
- The bundle identifier is also the data directory name
  (`paths.rs:66-77`). **Changing it would orphan every user's data and
  Keychain items.** Store channels must keep it (see the
  [capability matrix](capability-matrix.md#identifiers) for where a store
  forces a different one).

## 3. The update client

### 3.1 Constants (`crates/deckpilot-update/src/lib.rs`)

| Constant | Value | Line |
| --- | --- | --- |
| `REPOSITORY` | `knittingcode/DeckPilot` | 33 |
| `STABLE_ENDPOINT` | `https://github.com/knittingcode/DeckPilot/releases/latest/download/latest.json` | 37-38 |
| `RELEASES_API` | `https://api.github.com/repos/knittingcode/DeckPilot/releases?per_page=30` | 42-43 |
| `DOWNLOAD_PREFIX` | `https://github.com/knittingcode/DeckPilot/releases/download/` | 46 |

The former spelling `DeckPilot` is deliberate (`lib.rs:24-30`): the
repository is now `knittingcode/deckpilot` and GitHub serves both, but
installed builds up to `0.6.0-alpha.3` expect the old spelling. The
constants are pinned byte for byte by
`crates/deckpilot-update/tests/pipeline_prefix.rs:41-57`, the pipeline's
copy by `tools/release-pipeline/common.sh:24` and
`pipeline_prefix.rs:31-39`, and the Tauri endpoint by
`pipeline_prefix.rs:59-72`. **No distribution PR may change these
strings.**

### 3.2 Channels (`crates/deckpilot-update/src/channel.rs`)

- Two channels only: `Stable` (default) and `Prerelease` (`channel.rs:34-42`).
  Alpha, beta and rc are all `Prerelease` (`channel.rs:127-133`); there is
  no separate beta channel.
- `Channel::accepts` (`channel.rs:54-56`): always strictly newer; the stable
  channel never accepts a pre-release. This is passed to the Tauri updater
  as its `version_comparator` (`apps/desktop/src-tauri/src/updates.rs:327`),
  so whatever a manifest announces, a downgrade or a pre-release on stable
  is refused.
- Manifest names: `latest.json` for stable tags, `prerelease.json` for
  pre-release tags (`channel.rs:28-31`, `83-89`). Stable reads the
  `latest` alias; prerelease reads GitHub's release list, picks the newest
  published release that the channel accepts, ignoring drafts and releases
  whose pre-release flag disagrees with the tag (`channel.rs:230-244`),
  and builds the manifest URL from the **validated tag** and
  `DOWNLOAD_PREFIX` (`channel.rs:186-193`).
- Tag grammar: `^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$`
  (`channel.rs:12`, `common.sh:11`). The compatibility baseline tag
  `v0.5.0-2` is _not_ a release tag under it (`channel.rs:301-303`).
- MSI version mapping alpha.N → `.N`, beta.N → `.20000+N`, rc.N →
  `.40000+N` (`channel.rs:147-182`).

### 3.3 Manifest validation (`crates/deckpilot-update/src/manifest.rs`)

- `validate_artifacts` (`manifest.rs:96-101`, `130-139`) requires every
  artifact URL to be a release download of `REPOSITORY` (owner and name
  compared case-insensitively since #227, `manifest.rs:121-128`), without
  `..`, `?`, `#` or `\`, and a well-formed minisign signature.
- **Finding:** in this tree that check is called only by the release
  tooling (`tools/dev/src/release.rs:47`, whose comment says "as the client
  checks them"). The desktop client (`apps/desktop/src-tauri/src/updates.rs`)
  never calls `Manifest::parse`, `update_for` or `validate_artifacts`
  (`git log -S validate_artifacts -- apps/desktop` is empty): it hands the
  manifest URL to the Tauri updater, which downloads whatever URL the
  manifest names and accepts it only if the minisign signature verifies
  against the compiled-in key and the signed version matches
  (`updates.rs:313-333`, `392-402`). The pinned prefix still matters for
  the prerelease manifest URL (`channel.rs:186-188`) and for the pipeline.
  The statement in `pipeline_prefix.rs:3-4` and #227 that installed builds
  "compare every package URL case-sensitively" is not reproduced by the
  current client code; keeping the pinned prefix is harmless either way.
  PR C (below) must either add the URL check to the client before
  `updater.check()`, or correct those comments. Marked **Unverified** until
  then.

### 3.4 The updater flow (`apps/desktop/src-tauri/src/updates.rs`)

- Not configured (no download ever) unless the build compiled in
  `DECKPILOT_UPDATER_PUBKEY` (`updates.rs:40`, `89-91`, `277-279`); the
  background task does not start then (`updates.rs:785-788`).
- Build-time-only loopback test endpoint `DECKPILOT_UPDATE_ENDPOINT`,
  accepted only as `http://127.0.0.1:<port>…` (`updates.rs:47-63`). This is
  the precedent for a safe build-time override: `option_env!`, validated,
  never read from the runtime environment.
- Phases `Idle → Checking → Idle | Downloading → Ready → Installing`, one
  operation at a time under one lock (`updates.rs:93-102`, `213-238`).
- Modes `NotifyOnly` (default), `DownloadAndAsk`, `InstallWhenIdle`
  (10 minutes without device input) (`crates/deckpilot-update/src/policy.rs:10-26`,
  `108-123`). Preferences live in the encrypted database under
  `update.preferences` (`policy.rs:8`, `updates.rs:163-179`).
- Install sequence (`updates.rs:646-716`): refuse while workflows run
  (`updates.rs:656-665`); `PrepareForUpdate` to the agent, falling back to
  protocol 1 `Shutdown` for old agents (`apps/desktop/src-tauri/src/agent.rs:127-143`);
  wait up to 15 s for it to stop (`updates.rs:681-689`); `update.install`;
  on failure restart the previous agent and keep the update ready
  (`updates.rs:691-714`); on success `app.restart()` (`updates.rs:633-637`),
  and the restarted app starts the new agent (`apps/desktop/src-tauri/src/main.rs:127-136`).
- Failures are explained per updater error, including signature failures
  ("Do not install DeckPilot from another source"), package-manager
  failures (`DebInstallFailed`, `PackageInstallFailed`) and the macOS
  "previous app not restored" case (`updates.rs:407-464`), and logged to
  `logs/updates.log` (`updates.rs:469-494`).
- The UI (`apps/desktop/src/components/updates-card.tsx`,
  `apps/desktop/src/windows/update-window.tsx`) shows check, download,
  install-and-restart, mode and channel; when `configured` is false it
  disables the check with "Updates are not available in this build"
  (`updates-card.tsx:69-70`, `115`). There is no notion of "updated by
  someone else".

### 3.5 Package format selection by the Tauri updater

- `tauri-utils 2.10.2` keeps a static `__TAURI_BUNDLE_TYPE` that the
  bundler (`tauri-bundler 2.10.4`, `src/bundle.rs:40-49`) patches per
  package to `DEB`, `RPM`, `APP` (AppImage), `MSI` or `NSS`; on macOS it is
  always `App` (`tauri-utils/src/platform.rs:351-378`).
- `tauri-plugin-updater 2.14.1` looks up `{os}-{arch}-{bundle_type}` before
  `{os}-{arch}` (`updater.rs:46-47`, `260`, `722`). The published manifest
  has both forms (section 5), so a `.deb` install updates with the `.deb`
  (installed with `pkexec dpkg -i`), an AppImage with the AppImage, MSI and
  NSIS with their own installers.
- This is the right precedent for "metadata baked into the bundle", but it
  answers _which file format_, never _who owns the installation_.

## 4. Release pipeline

`.github/workflows/release.yml` (735 lines), stage A then stage B
(`release.yml:1-18`):

| Job | What it does | Lines |
| --- | --- | --- |
| `validate-tag` | Tag grammar, commit on protected `main`, versions, required checks (`tools/release-pipeline/validate-tag.sh`, `required-checks.txt`) | 83-123 |
| `environment-gate` | `production-release` exists, deploys `v*` only (`check-environment.sh`) | 125-142 |
| `prepare` | Updater key pair matches `.github/updater.pub`; release notes; macOS signing configuration complete, or the release is blocked unless `DECKPILOT_ALLOW_UNSIGNED_MACOS` | 144-217 |
| `packaging` | Reusable `packaging.yml` (unsigned installers, throwaway updater key) | 224-236 |
| `compatibility` | Previous release vs this commit (`tools/compat/matrix.sh`) | 238-258 |
| `draft` | One draft release for the tagged commit (`create-draft.sh`) | 260-286 |
| `build` (×4) | `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc`; builds and stages the agent as `externalBin`, writes `tauri.updater.conf.json` (pubkey, MSI version), `tauri-action` with `--config tauri.release.conf.json`; Linux pre-releases only `--bundles deb,appimage` (RPM refuses `-`); checks the embedded updater key; notarizes and staples the DMG; verifies signatures; writes a signing record; attests provenance | 287-553 |
| `manifest` | Assembles `latest.json` or `prerelease.json` once from the per-target fragments (`tools/ci/assemble-latest-json.sh`) with the pinned prefix | 555-588 |
| `verify` | Downloads the draft, verifies every updater signature (`deckpilot-dev verify-release`), every platform and package type in the manifest, writes `release-meta.json` (`release-meta.sh`) and `SHA256SUMS`, final asset check (`check-assets.sh --final`) | 590-686 |
| `publish-gate` | Environment still protected | 688-708 |
| `publish` | `publish.sh`: re-checks the draft's real assets, publishes in one update (pre-release or latest), then `verify-published.sh` downloads every public asset and compares with `SHA256SUMS` | 710-735 |

Properties that the distribution work must keep:

- **Fail closed, publish once.** Publication is a single update of a
  verified draft (`tools/release-pipeline/publish.sh:1-25`); a failed stage
  B never changes a published release (`release.yml:14-17`).
- **Secrets only in `production-release` jobs** (`prepare`, `build`,
  `publish`), checked by `tools/release-pipeline/check-workflow-gate.py`
  and `check-secret-isolation.py`.
- **Asset set is closed.** `check-assets.sh:26-40` requires the exact
  installer, updater archive and manifest names and rejects the other
  channel's manifest. A channel-specific artifact added later must be added
  there.
- **`release-meta.json`** records `tag`, `version`, `channel`, `commit`,
  `manifest`, `updater_signature` and per-target `codesign` / `notarized`
  (`release-meta.sh:60-66`). It has no distribution channel or package
  format yet.
- `tools/release-pipeline/check-release-separation.py` named in the task is
  **not on `main`**; nothing here depends on it.

Other workflows:

- `packaging.yml`: on PRs touching packaging paths, nightly, and called by
  the release. Builds Linux `.deb` (+ `.rpm` for final versions) and
  Windows NSIS + MSI with the release configuration and a throwaway updater
  key, then `tools/ci/check-packages.sh` (names, versions, app, bundled
  agent, udev rule, updater signatures) (`packaging.yml:1-22`, `104-160`,
  `162-233`). **It does not build macOS or the AppImage**, and signs
  nothing.
- `update-continuity.yml`: manual, on a release tag, in
  `production-release`; two Developer ID signed (not notarized) builds,
  loopback update, Keychain access survives (`update-continuity.yml:1-19`).
- `compatibility.yml`: `tools/compat/matrix.sh` against the previous
  release's real binaries; baseline `v0.5.0-2`
  (`tools/release-pipeline/compat-baseline`, `tools/compat/matrix.sh:17`).
- `signing-check.yml` ("Updater signing keys match", a required check) and
  `apple-signing-check.yml` (manual, `production-release`).

## 5. The published v0.6.0-alpha.3 release

Read with `gh release view v0.6.0-alpha.3 -R knittingcode/deckpilot` and
`gh api repos/knittingcode/deckpilot/releases/tags/v0.6.0-alpha.3` on
2026-11-15: published 2026-11-14T20:52:06Z, `prerelease: true`,
`immutable: true`, commit `b0b23de9bd1183352cffc059a373d0ba3f550841`.
17 assets:

| Asset | Bytes | Role |
| --- | --- | --- |
| `DeckPilot_0.6.0-alpha.3_aarch64.dmg` | 15455368 | macOS arm64 installer (notarized, stapled) |
| `DeckPilot_0.6.0-alpha.3_x64.dmg` | 16259431 | macOS x86_64 installer |
| `DeckPilot_aarch64.app.tar.gz` + `.sig` | 15953394 | macOS arm64 updater archive |
| `DeckPilot_x64.app.tar.gz` + `.sig` | 16618149 | macOS x86_64 updater archive |
| `DeckPilot_0.6.0-alpha.3_x64_en-US.msi` + `.sig` | 17231872 | Windows MSI, installer and updater |
| `DeckPilot_0.6.0-alpha.3_x64-setup.exe` + `.sig` | 12327720 | Windows NSIS, installer and updater |
| `DeckPilot_0.6.0-alpha.3_amd64.deb` + `.sig` | 18167924 | Linux `.deb`, installer and updater |
| `DeckPilot_0.6.0-alpha.3_amd64.AppImage` + `.sig` | 95070712 | Linux AppImage, installer and updater |
| `prerelease.json` | 16870 | Update manifest |
| `release-meta.json` | 609 | Signing record |
| `SHA256SUMS` | 1588 | Checksums of the 16 other assets |

`prerelease.json` has `version`, `notes`, `pub_date` and ten platform keys:
`darwin-aarch64`, `darwin-aarch64-app`, `darwin-x86_64`,
`darwin-x86_64-app`, `windows-x86_64` (→ MSI), `windows-x86_64-msi`,
`windows-x86_64-nsis`, `linux-x86_64` (→ AppImage),
`linux-x86_64-appimage`, `linux-x86_64-deb`. Every URL starts with the
pinned `https://github.com/knittingcode/DeckPilot/releases/download/`.

`release-meta.json`: `channel: prerelease`, `manifest: prerelease.json`,
`updater_signature: minisign`; `aarch64-apple-darwin` and
`x86_64-apple-darwin` `developer-id` / notarized; `x86_64-pc-windows-msvc`
and `x86_64-unknown-linux-gnu` `unsigned` / `null`.

Not shipped: no `.rpm` (pre-release), no Linux or Windows arm64, no
Flatpak, no MSIX, no store build, no plugin host.

`docs/releasing.md:705-726` ("Status") still says no release has been
published; that section is out of date since `v0.6.0-alpha.3`.

## 6. Signing

| What | Status | Evidence |
| --- | --- | --- |
| macOS Developer ID Application, team `PUB9L83UMZ` | Provisioned in `production-release`; app and agent signed with hardened runtime | `release.yml:33-43`, `363-388`, `462-484`; `docs/releasing.md:717`; `release-meta.json` |
| macOS notarization and stapling | App notarized and stapled by the bundler; DMG notarized and stapled by the workflow | `release.yml:444-461` |
| Fixed agent signing identifier | `io.github.knittingcode.deckpilot.agent`, CI-checked | `apps/agent/Info.plist`, `.github/workflows/ci.yml:133-140` |
| Entitlements | None, enforced | `.github/scripts/verify-macos-app.sh:26-28` |
| Tauri updater (minisign, Ed25519) | Public key pinned in `.github/updater.pub`; repository variable `DECKPILOT_UPDATER_PUBKEY` must equal it; private key in `production-release`; key pair proven in `prepare`; embedded key checked in the binary | `signing-check.yml:1-20`, `release.yml:161-180`, `434-443` |
| Windows Authenticode | **Not provisioned**: installers unsigned, SmartScreen warns | `docs/releasing.md:199-205`; `release-meta.json` |
| Linux `.deb` / AppImage | **No code signature, no GPG**; only the minisign `.sig` used by the updater, and `SHA256SUMS` | `release-meta.sh:53-56`; release assets |
| Build provenance | GitHub artifact attestation | `release.yml` "Attest build provenance" |

The Keychain item with the database key is bound to the agent's code
identity (`apps/agent/Info.plist` comment). Any channel that re-signs the
agent with a different identity (Mac App Store: Apple Distribution) will
**not** be able to read Keychain items written by the Developer ID build
without a migration step; see the [capability matrix](capability-matrix.md).

## 7. The agent's lifecycle

- **Single instance per data directory**: an OS file lock on
  `<data dir>/agent.lock`, released by the OS on death
  (`apps/agent/src/instance.rs:15-43`); a starting agent waits up to 5 s for
  a stopping one (`instance.rs:8-12`, `45-61`); a second agent exits with
  success (`apps/agent/src/main.rs:243-257`).
- **IPC**: protocol version 10, minimum 1, minimum compatible agent 4
  (`crates/deckpilot-ipc/src/protocol.rs:80`, `173`, `178`); the client
  negotiates down (`crates/deckpilot-ipc/src/client.rs:130-150`). Endpoint
  derived from the data directory (`paths.rs:98-101`), token file readable
  by the user only (`paths.rs:60-62`).
- **Shutdown**: `Shutdown`, `PrepareForUpdate` and tray Quit stop the
  service loop (`apps/agent/src/service.rs:733-741`); `stop` stops plugin
  hosts, ends the work session, interrupts executions, broadcasts
  `ShuttingDown`, blanks and closes every device within `CLOSE_TIMEOUT`,
  waits for the server to close the endpoint and removes the token
  (`service.rs:603-634`). SIGTERM and Ctrl+C do the same
  (`service.rs:1743-1763`). `PrepareForUpdate` itself does not wait for
  workflows: the app refuses to install while `running_executions > 0`
  (`updates.rs:656-665`) and the restart command asks before interrupting
  (`apps/desktop/src-tauri/src/commands/agent.rs:48-67`).
- **Who starts the agent**: the app at its start if none runs
  (`main.rs:127-136`), detached in its own process group / `DETACHED_PROCESS`
  (`agent.rs:296-322`); the login entry; the user. Inside an AppImage the
  AppImage itself is re-executed with `--agent` so the mount stays alive
  (`agent.rs:252-293`).
- **Where the agent executable is found**: `DECKPILOT_AGENT_BIN`, or
  `deckpilot-agent` next to the app executable (`agent.rs:242-251`).
- **Autostart**: per-user only, via `auto_launch`: a `LaunchAgent` plist on
  macOS, an XDG autostart entry on Linux, `HKCU\…\Run` on Windows
  (`apps/agent/src/autostart.rs:1-3`, `41-56`); AppImage entries point at
  the AppImage file and are repaired (`autostart.rs:19-28`, `58-66`).
  **None of these is permitted or effective inside the Mac App Store,
  MSIX or Flatpak sandboxes**: each needs its platform mechanism
  (`SMAppService`, `windows.startupTask`, the XDG Background portal).
- **Devices**: raw HID through `hidapi 2.6.7` (`crates/deckpilot-hid`),
  hidraw on Linux, which the udev rule opens to the logged-in user
  (`uaccess`).
- **Tray**: `tray-icon` in the agent (`apps/agent/Cargo.toml:78`,
  `apps/agent/src/tray.rs`).
- **Version coupling**: app and agent must be the same version; any
  mismatch is reported (`crates/deckpilot-update/src/compatibility.rs:39-56`)
  and the app offers to restart the agent (`agent.rs:80-104`).

## 8. Storage, data locations and upgrades

| Item | Location | Source |
| --- | --- | --- |
| Data directory | `DECKPILOT_DATA_DIR`, else `dirs::data_dir()/io.github.knittingcode.deckpilot` (`~/Library/Application Support/…`, `~/.local/share/…`, `%APPDATA%\…`) | `crates/deckpilot-ipc/src/paths.rs:14`, `66-77` |
| Database | `<data dir>/deckpilot.db` (SQLCipher) | `paths.rs:87-96` |
| Lock, token, logs | `agent.lock`, `agent.token`, `logs/` | `paths.rs:87-96` |
| Pre-migration backups | `<data dir>/backups/`, newest 5 kept | `crates/deckpilot-storage/src/sqlite.rs:106`, `1100-1104`, `1185-1205` |
| Plugin side store | `<data dir>/plugins/plugins.db`, own SQLCipher file, own migrations (`schema_migrations`) | `crates/deckpilot-plugin-store/src/layout.rs:5`, `26`; `db.rs:1-3`, `25`, `95`, `133-177` |
| Credential store service | `DeckPilot` (production), `DeckPilot Development` (debug builds or any custom data dir), `DeckPilot Test …` (tests) | `crates/deckpilot-keystore/src/lib.rs:110-130`; `apps/agent/src/service/security.rs:59-68` |

No user data is written inside an install directory. That holds for every
format today and must hold for every channel.

Migrations (`sqlite.rs:27-85`, `167-330`):

- Schema version = number of migrations, **9** on `main`
  (`sqlite.rs:252-254`; entries at `sqlite.rs:27-85`).
- A database with a higher `user_version` is refused before anything is
  written, and again inside the migration transaction (`sqlite.rs:193-199`,
  `263-270`, `287-293`). Measured across releases: the 0.5.0 agent refuses
  a 0.6.0 database and leaves it byte-identical (`docs/compatibility.md:75`).
- Before upgrading an existing database: integrity check, then an
  **encrypted, verified backup** with the same key (`sqlite.rs:274-279`,
  `372-400`). All migrations run in one `IMMEDIATE` transaction; the result
  is recorded as unverified, integrity-checked after commit, and re-checked
  on every start until it passes (`sqlite.rs:305-370`).
- Migrations are **forward-only**: there are no down-migrations, and the app
  has no "restore backup" command (only the files in `backups/`; see
  `docs/security.md:307-309`).

Consequence for rollback, used in the
[agent update lifecycle](README.md#7-agent-update-lifecycle): re-installing
the previous binaries is safe **only if** the update did not migrate the
database (same schema version), or after the pre-migration backup has been
restored. The updater never offers an older version
(`channel.rs:54-56`), and no channel may.

## 9. Plugin host

- The agent's `plugins` feature is **off** in release builds; the release
  builds `cargo build --release --locked -p deckpilot-agent` with default
  features (`apps/agent/Cargo.toml:26-45`, `release.yml:341-348`), checked
  by `apps/agent/tests/plugins_feature.rs`.
- The host (`deckpilot-plugin-host`) lives in the standalone workspace
  `plugins/runtime/` which the root workspace does not build
  (`plugins/runtime/README.md:7-10`). It is **not** in `externalBin`, not
  signed, not notarized and in no release asset.
- It sandboxes itself: macOS `sandbox_init` with a deny-default SBPL profile
  (`plugins/runtime/crates/host/src/sandbox/macos.rs:1-54`), Linux Landlock
  plus a seccomp-bpf allowlist (`sandbox/linux.rs:1-110`), Windows
  AppContainer plus Job Object applied by the supervisor
  (`plugins/runtime/crates/supervisor/src/windows.rs`). The agent requires
  a `full` sandbox report (`apps/agent/src/plugins/mod.rs:105-118`).
- It runs wasmtime `=50.0.1` with Cranelift (`plugins/runtime/Cargo.toml:55`,
  `crates/host/src/engine.rs:48`, `186-198`): executable memory at run time.
  A signed, hardened-runtime build with `com.apple.security.cs.allow-jit`,
  notarization, and the Pulley interpreter are listed as **not verified**
  (`plugins/runtime/README.md:174-175`,
  `experiments/marketplace-spikes/runtime/RESULTS.md:233-235`, `317-318`).
- The agent finds the host through `--plugin-host`, `DECKPILOT_PLUGIN_HOST`,
  or next to the agent (`apps/agent/src/plugins/mod.rs:44-63`,
  `apps/agent/src/main.rs:35-36`).
- `docs/marketplace/distribution.md` is **not on `main`**; the marketplace
  documents present are `agent-integration.md`, `artifact-contract.md` and
  `local-testing.md`.

## 10. Compatibility checks

- `tools/compat/matrix.sh` builds the agent and CLI of the baseline tag
  (default `v0.5.0-2`) and of this commit, and runs each agent against each
  client, the old agent against a new database, and the upgrade of the old
  database (`tools/compat/matrix.sh:1-18`).
- `docs/compatibility.md` records the measured matrix.
- `deckpilot-dev check-versions` keeps app, agent and frontend versions
  equal (`ci.yml:117-119`).

## 11. Runtime overrides that exist today

| Variable | Read in release builds | Effect | Source |
| --- | --- | --- | --- |
| `DECKPILOT_DATA_DIR` | yes | Other data directory; switches the credential store to the development service | `paths.rs:66-72`, `security.rs:59-66` |
| `DECKPILOT_AGENT_BIN` | **yes** | The app starts this executable as "the agent" | `agent.rs:42`, `242-246` |
| `DECKPILOT_PLUGIN_HOST`, `DECKPILOT_ENABLE_PLUGINS`, registry variables | only with the `plugins` feature (off in releases) | Host path, registry | `apps/agent/src/plugins/mod.rs:44-54`, `87-100` |
| `DECKPILOT_UPDATER_PUBKEY` | compile time only | Updater key | `updates.rs:40` |
| `DECKPILOT_UPDATE_ENDPOINT` | compile time only, loopback only | Test endpoint | `updates.rs:47-63` |

The compile-time pattern (`option_env!` plus validation) is what the
distribution metadata adopts. `DECKPILOT_AGENT_BIN` in release builds
weakens the "restart a compatible agent" step, because any executable can
be started as the agent; PR F gates it to debug builds.

## 12. What works, what needs architectural change

Works and is kept as is:

- Channel semantics and manifest names (`channel.rs`), the version
  comparator, the pinned URL constants, the tag grammar and MSI mapping.
- The signed Tauri updater as the GitHub provider, including the per-format
  manifest keys.
- The two-stage, fail-closed, publish-once pipeline; immutable releases;
  `SHA256SUMS` and `release-meta.json` as the verified record.
- Single-instance lock, `PrepareForUpdate`, refusing to install while
  workflows run, restarting the previous agent on failure, version-coupled
  app and agent.
- User data outside install directories; forward-only migrations with a
  verified encrypted backup; refusing newer databases.

Needs architectural change:

| # | Gap | Where | Addressed by |
| --- | --- | --- | --- |
| G1 | No build knows its distribution channel; every build self-updates from GitHub | `updates.rs` (whole file) | Distribution metadata + `UpdateProvider` (PR B, C) |
| G2 | A `.deb` from an APT repository would self-update via `pkexec dpkg -i` | Tauri updater `linux-x86_64-deb` key | Channel-specific build; cross-channel rejection (PR B, C, D) |
| G3 | `release-meta.json` has no channel or format per artifact | `release-meta.sh:60-66` | PR D |
| G4 | Packaging check covers Linux `.deb`/`.rpm` and Windows only; no macOS bundle, AppImage, Flatpak, MSIX or MAS validation | `packaging.yml` | PR D |
| G5 | Autostart uses `LaunchAgent` / Run key / XDG file, invalid in sandboxes | `autostart.rs:41-56` | Per-channel autostart strategy (PR E, G) |
| G6 | Agent located through a runtime env var in release builds | `agent.rs:242-251` | PR E |
| G7 | No restore of the pre-migration backup; rollback across a migration impossible | `sqlite.rs` | PR E (documented policy; restore command later) |
| G8 | Plugin host not packaged; entitlements forbidden; JIT under hardened runtime unverified | `verify-macos-app.sh:26-28`, `plugins/runtime/README.md:174` | Marketplace distribution work; packaging matrix rows (PR D) |
| G9 | The UI assumes it can always check, download and install | `updates-card.tsx` | PR F |
| G10 | Client-side artifact URL check absent despite comments saying otherwise | section 3.3 | PR C |
| G11 | Windows unsigned; Linux unsigned (no GPG) | `release-meta.json` | Credentials (not created here) |
