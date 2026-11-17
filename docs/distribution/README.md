# Distribution architecture

How DeckPilot is distributed through more than one channel (direct GitHub
download, Homebrew, WinGet, APT, Flathub, the Mac App Store and the
Microsoft Store) without any channel updating an installation it does not
own. This is the target design. The pull requests listed in
[section 10](#10-implementation-plan) implement it.

Companion documents:

- [Audit](audit.md): what the code, the pipeline and the published
  `v0.6.0-alpha.3` release do today, with `file:line` evidence. Gaps
  G1-G11 are referred to below.
- [Capability matrix](capability-matrix.md): what each channel allows
  (sandbox, agent, HID, tray, plugin host, JIT, credentials, file system,
  updates), the store blockers, and the credentials each channel would
  need, with sources.
- [`targets.toml`](targets.toml): the packaging matrix of
  [section 8](#8-packaging-matrix) in machine-readable form, for the
  validation in PR D.

Constraints that apply to everything below:

- The application identity does not change: bundle identifier
  `io.github.knittingcode.deckpilot`, agent signing identifier
  `io.github.knittingcode.deckpilot.agent`, the data directory name, and
  the credential store service `DeckPilot` (audit section 2 and 8).
- The pinned update URLs in `crates/deckpilot-update/src/lib.rs:33-46` keep
  their spelling byte for byte (audit section 3.1).
- GitHub Releases stays the canonical, immutable record of every release.

Contents:

1. [Distribution metadata](#1-distribution-metadata)
2. [Where the metadata lives](#2-where-the-metadata-lives)
3. [The update provider interface](#3-the-update-provider-interface)
4. [Providers](#4-providers)
5. [Cross-channel rejection rules](#5-cross-channel-rejection-rules)
6. [Release automation](#6-release-automation)
7. [Agent update lifecycle](#7-agent-update-lifecycle)
8. [Packaging matrix](#8-packaging-matrix)
9. [Updates UI per channel](#9-updates-ui-per-channel)
10. [Implementation plan](#10-implementation-plan)

## 1. Distribution metadata

Every build carries one immutable record describing itself:

| Field | Values | Meaning |
| --- | --- | --- |
| `distribution_channel` | `github`, `homebrew`, `winget`, `apt`, `flathub`, `mac_app_store`, `microsoft_store`, or `unknown` | Who delivered **this artifact** and therefore owns its installation. `unknown` is what a build without metadata reports (development builds; a release build must never ship it) |
| `update_provider` | `github_release`, `external_package_manager`, `store_managed`, `none` | Derived from `distribution_channel` by a fixed table (below); stored for display and checks, never chosen separately |
| `package_format` | `app` (macOS `.app` in `.dmg` / `.app.tar.gz`), `msi`, `nsis`, `deb`, `rpm`, `appimage`, `flatpak`, `mas_pkg`, `msix` | The installed format |
| `release_channel` | `stable`, `beta`, `alpha` | The class of **this build's own version** |
| `application_version` | SemVer, for example `0.6.0-alpha.3` | `CARGO_PKG_VERSION`, equal across app, agent and frontend (`deckpilot-dev check-versions`) |
| `platform` | `macos`, `windows`, `linux` | The compilation target OS |
| `architecture` | `aarch64`, `x86_64` | The compilation target architecture |

`update_provider` is a function of `distribution_channel`:

| `distribution_channel` | `update_provider` |
| --- | --- |
| `github` | `github_release` |
| `homebrew`, `winget`, `apt` | `external_package_manager` |
| `flathub`, `mac_app_store`, `microsoft_store` | `store_managed` |
| `unknown` | `none` |

`release_channel` maps onto the existing semantics of
`crates/deckpilot-update/src/channel.rs` without changing them:

| Build version | `release_channel` | `ReleaseKind` | Manifest | Default update `Channel` |
| --- | --- | --- | --- | --- |
| `X.Y.Z` | `stable` | `Stable` | `latest.json` | `Stable` |
| `X.Y.Z-beta.N`, `X.Y.Z-rc.N` | `beta` | `Prerelease` | `prerelease.json` | `Prerelease` |
| `X.Y.Z-alpha.N` | `alpha` | `Prerelease` | `prerelease.json` | `Prerelease` |

The update subscription stays the two-valued `Channel` (`Stable`,
`Prerelease`) that the user chooses (`policy.rs:28-39`). `release_channel`
describes the build; `Channel` describes what the user wants to receive.
The prerelease channel keeps accepting alpha, beta, rc and stable, and
`Channel::accepts` (`channel.rs:54-56`) stays the only version rule. An rc
counts as `beta` because it is offered to the same audience; that is a
label, not a new rule. A store's own beta track (TestFlight, Partner Center
package flights, Flathub beta, an APT `prerelease` suite) is chosen by the
store or package manager, never by this field.

`platform` and `architecture` are compile-time facts (`cfg!(target_os)`,
`cfg!(target_arch)`), which is what `manifest::current_target`
(`manifest.rs:141-148`) already uses. They must not be detected at run
time.

## 2. Where the metadata lives

### 2.1 Rule

The metadata is **baked in at build time, read-only, and inside the signed
bundle**. It is never derived from the operating system and never read from
user configuration or the environment in a release build.

Not allowed as a source, in a release build:

- probing the OS: `/.flatpak-info`, `FLATPAK_ID`, `/var/lib/dpkg`, the
  install path (`/Applications`, `/opt/homebrew/Caskroom`, `Program Files`,
  `WindowsApps`), MSIX package identity APIs, `receipt` files, the App
  Store receipt;
- the database, `update.preferences` or any setting;
- environment variables and command-line arguments.

Reason: each of these can be wrong (a `.deb` copied to another machine, a
user who sets a variable, a Homebrew path that is just a symlink), and an
attacker who can set one could turn a store build into a self-updater, or
silence updates of a GitHub build. A compiled constant is covered by the
macOS code signature, by Authenticode once Windows is signed, by the
minisign updater signature for GitHub packages, by the APT repository
signature, and by the store's signature.

### 2.2 Mechanism

A new crate, `crates/deckpilot-distribution` (PR B), owns the record:

- Its `build.rs` reads `DECKPILOT_DISTRIBUTION_CHANNEL` and, for channels
  with their own build, `DECKPILOT_PACKAGE_FORMAT` **at compile time**
  (`cargo:rerun-if-env-changed`). It fails the build on any value outside
  the enums and passes the value on with `cargo:rustc-env`. This is the
  same pattern as `DECKPILOT_UPDATER_PUBKEY` and `DECKPILOT_UPDATE_ENDPOINT`
  (`apps/desktop/src-tauri/src/updates.rs:40`, `53`).
- `pub const DISTRIBUTION: Distribution` and `pub fn current() ->
  &'static Distribution` expose it. `application_version` is
  `env!("CARGO_PKG_VERSION")`. `release_channel` is computed from it with
  `ReleaseTag::for_version` (`channel.rs:143-145`). `platform` and
  `architecture` come from `cfg!`.
- Both `deckpilot-desktop` and `deckpilot-agent` depend on it. The release
  workflow sets the variable for the agent build step
  (`release.yml:341-348`) and for the `tauri-action` step
  (`release.yml:389-410`). The agent reports its record in `GetStatus`, so
  the app can refuse an agent from another channel (lifecycle step 10).
- **Package format of GitHub builds.** One GitHub Linux build produces
  `.deb`, `.rpm` and AppImage from the same binary, and one Windows build
  produces MSI and NSIS. A compiled constant cannot tell them apart. For
  `distribution_channel = github`, `package_format` comes from Tauri's
  `tauri::utils::platform::bundle_type()`. The bundler patches that value
  into the binary per package, before signing
  (`tauri-bundler-2.10.4/src/bundle.rs:40-49`,
  `tauri-utils-2.10.2/src/platform.rs:351-378`), so it is also baked in and
  covered by the package's signature. On macOS it is always `app`. Channels
  with their own build (`apt`, `flathub`, `mac_app_store`,
  `microsoft_store`) compile the format in. If both are present they must
  agree; otherwise the provider is `none` (rule R3).
- A release build with `distribution_channel = unknown` gets provider
  `none`: updates "not available in this build", exactly as a build without
  an updater key behaves today (`updates.rs:89-91`). PR D adds a release
  step that reads the embedded record from each built binary and fails the
  build if it is `unknown` or differs from the matrix entry. It works like
  the existing "Embedded updater key matches" step
  (`release.yml:434-443`).
- Store and package-manager variants are built **without**
  `DECKPILOT_UPDATER_PUBKEY` and without `createUpdaterArtifacts`. The
  Mac App Store, MSIX and Flathub variants are also built without the
  `tauri-plugin-updater` dependency (a `self-update` Cargo feature of the
  desktop crate, on by default). A build that cannot verify or install an
  update cannot be talked into one.

### 2.3 Debug and test overrides

Overrides exist so that the UI and the providers can be tested for every
channel without building each variant:

- **Unit and flow tests** construct a `Distribution` value and pass it
  into the provider. `updates.rs` already injects its `Source` this way
  (`updates.rs:251-270`). No global state is overridden.
- **Debug builds** (`cfg(debug_assertions)`) may read
  `DECKPILOT_DISTRIBUTION_OVERRIDE` (for example `apt` or
  `mac_app_store`). The code that reads it sits behind
  `#[cfg(debug_assertions)]`, so it is **not compiled** into release builds
  (the variable name does not occur in the binary). The override is
  logged, and the UI marks it "Development override". This follows
  `apps/agent/src/service/security.rs:63`, where debug builds already
  switch the credential store scope.
- **The loopback update test build** keeps using the compile-time
  `DECKPILOT_UPDATE_ENDPOINT` (`updates.rs:47-63`), with
  `DECKPILOT_DISTRIBUTION_CHANNEL=github`.
- PR D's release check also asserts that the string
  `DECKPILOT_DISTRIBUTION_OVERRIDE` is absent from the release binaries.

## 3. The update provider interface

Pure logic in `crates/deckpilot-update/src/provider.rs` (PR B). The
Tauri-dependent implementation lives in the desktop crate (PR C).

```rust
/// What the UI shows and which operations exist.
pub enum ProviderKind { GithubRelease, ExternalPackageManager, StoreManaged, None }

pub struct ProviderCapabilities {
    /// The app can download and install updates itself.
    pub self_update: bool,
    /// UpdateMode::DownloadAndAsk / InstallWhenIdle are offered.
    pub automatic_updates: bool,
    /// The user may choose Stable or Prerelease in the app.
    pub channel_choice: bool,
    /// The app may look up a newer version (for a notice only, without self_update).
    pub version_check: bool,
}

pub enum ProviderStatus {
    Idle,
    Checking,
    Available(AvailableVersion),
    Downloading { downloaded: u64, total: Option<u64> },
    Ready(AvailableVersion),
    Installing,
    /// Updates come from elsewhere; `instruction` tells the user how.
    Managed { by: DistributionChannel, instruction: Option<String> },
    NotConfigured { reason: String },
    Failed { stage: Stage, message: String },
}

#[async_trait]
pub trait UpdateProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn capabilities(&self) -> ProviderCapabilities;
    /// Look for an update the channel accepts. Never downloads a package.
    async fn check(&self) -> Result<Option<AvailableVersion>, UpdateError>;
    /// The result of the last check.
    fn available_version(&self) -> Option<AvailableVersion>;
    /// Download and verify; GitHubRelease only, every other provider returns `Unsupported`.
    async fn prepare(&self, progress: &dyn Fn(u64, Option<u64>)) -> Result<(), UpdateError>;
    /// Replace the installation; GitHubRelease only. Runs the agent lifecycle (section 7).
    async fn install(&self) -> Result<(), UpdateError>;
    fn status(&self) -> ProviderStatus;
    /// Whether `capabilities().automatic_updates`; the background task asks this.
    fn supports_automatic_updates(&self) -> bool;
}
```

`AvailableVersion` is today's `UpdateInfo` (`updates.rs:104-110`): version,
notes, date. `Stage` is today's `Stage` (`updates.rs:374-390`). The phases
map one to one onto today's `Phase` (`updates.rs:93-102`), so
`UpdateState` (`updates.rs:141-153`) gains two fields, `distribution` and
`provider` (kind plus capabilities), and keeps every existing field.

The provider is chosen **once at start-up** from `deckpilot_distribution::current()`
and stored in Tauri state in place of today's direct use of
`Source::release` (`updates.rs:276-295`). It cannot change while the app
runs.

## 4. Providers

### 4.1 GitHubRelease

The existing implementation, moved behind the interface **unchanged**:

- endpoints, constants and channel selection as in `updates.rs:65-72`,
  `275-311` and `channel.rs`;
- `version_comparator` with `Channel::accepts` (`updates.rs:321-328`);
- minisign verification against the compiled key and the signed version
  (`requireSignedVersion`, `tauri.conf.json:67`);
- the install sequence and its failure handling (`updates.rs:646-716`),
  extended by the lifecycle in [section 7](#7-agent-update-lifecycle);
- **added** (G10): before `updater.check()`, the manifest is fetched once
  with the same client and validated with `Manifest::validate_artifacts`
  (`manifest.rs:96-101`), and the entry for
  `{current_target}-{package_format}` must exist. An untrusted URL fails
  the check before Tauri sees the manifest. This matches what the pipeline
  already enforces (`tools/dev/src/release.rs:47`). Old clients are
  unaffected, and the pinned prefix keeps both working.

Capabilities: `self_update`, `automatic_updates`, `channel_choice` and
`version_check` are all true, as today.

### 4.2 ExternalPackageManager

For `apt` (and the reserved `homebrew`, `winget`):

- **Never** constructs a Tauri updater, never downloads a package, never
  replaces a file. `prepare` and `install` return `Unsupported`.
- Shows the update command for its channel: `sudo apt update && sudo apt
  upgrade` (APT), `brew upgrade --cask deckpilot` (Homebrew),
  `winget upgrade KnittingCode.DeckPilot` (WinGet; the identifier is
  decided when the manifest is first submitted).
- `version_check`: optional and **off by default**. When the user turns it
  on, `check` reads the GitHub manifest of the user's `Channel`, which
  needs no key, and only reports "version X is available from your package
  manager". The repository may lag behind GitHub, so the wording never
  says the package manager already has it.
- Capabilities: `self_update = false`, `automatic_updates = false`,
  `channel_choice = false` (APT suites, cask names and WinGet identifiers
  select the track), `version_check` per setting.

### 4.3 StoreManaged

For `flathub`, `mac_app_store` and `microsoft_store`:

- No network check at all. Store policies discourage or forbid prompting
  users to update outside the store, and the store already notifies.
  `check` returns `Ok(None)` without I/O.
- `status` is `Managed { by, instruction }`, with instructions such as
  "Updates are installed by the App Store", "… by the Microsoft Store", or
  "… by `flatpak update` or your software center".
- The Tauri updater plugin is not compiled into these builds (section 2.2).
- Capabilities: all false.

### 4.4 None

`distribution_channel = unknown` (development builds), or a GitHub build
without an updater key. It reports `NotConfigured`, which the UI shows as
"Updates are not available in this build", as today
(`updates-card.tsx:69-70`).

## 5. Cross-channel rejection rules

Each rule is enforced in code and covered by a test in the PR named.

| # | Rule | Enforced by | PR |
| --- | --- | --- | --- |
| R1 | Only `distribution_channel = github` may construct the Tauri updater. Every other channel's provider has no code path to it, and its build has no updater key and, for stores, no updater plugin | Provider selection; build configuration | B, C, D |
| R2 | A store or package-manager build never accepts a GitHub-updater update, even if a GitHub manifest is reachable and the user asks: `prepare`/`install` return `Unsupported` | `ExternalPackageManager`, `StoreManaged` | C |
| R3 | A GitHub build self-updates only if its `package_format` (from `bundle_type()`) is one the manifest offers, the manifest has the exact `{os}-{arch}-{format}` key, and a compiled format, if any, agrees. Otherwise the provider is `None` with a reason | `GitHubRelease` construction | C |
| R4 | A GitHub update installs the **same format** it was installed as: never a `.deb` over an AppImage, never NSIS over MSI | Tauri's format keys (audit 3.5) plus R3's exact-key check | C |
| R5 | Never an older version, never a pre-release on the stable channel | `Channel::accepts` as `version_comparator` (unchanged) | C |
| R6 | The app refuses to work with an agent from another `distribution_channel` or another version, and restarts the agent from its own bundle | Lifecycle step 10; `AgentCompatibility` | E |
| R7 | The GitHub `.deb` is never republished into the APT repository; APT publishes only the `apt` variant, verified by its embedded record | Downstream `distribute-apt` job | D, G |
| R8 | Changing channels is a reinstall by the user, never an in-app action. Whichever variant is installed decides. (A user who installs the GitHub `.deb` over the APT one has a self-updating install until `apt upgrade` replaces it with the APT variant. This is documented, not prevented: preventing it would mean probing the OS) | Documentation | F |
| R9 | No release-build input (environment, arguments, settings, database) can change the record | Section 2; release binary check | B, D |

## 6. Release automation

### 6.1 Principles

- GitHub Releases stays canonical. Stage A builds and verifies a draft,
  and stage B publishes it once (audit section 4). Published releases are
  immutable (`immutable: true` on `v0.6.0-alpha.3`).
- **Any artifact that must be byte-identical to what was verified is built
  in stage A** and attached to the draft, covered by `SHA256SUMS` and
  described in `release-meta.json`. For now that means the APT `.deb`
  variant. Downstream jobs only republish such files and write channel
  metadata.
- Channels that build and sign on their own (Flathub builds from source;
  the Mac App Store and MSIX variants are signed with other identities)
  build **from the release's commit** recorded in `release-meta.json`, in
  their own jobs. Their outputs never become release assets.
- Downstream jobs run in a separate workflow, `distribute.yml`, with
  `permissions: contents: read` on this repository. They are triggered on
  `release: published` and by hand, and each runs in its own protected
  environment (`distribute-homebrew`, `distribute-winget`, `distribute-apt`,
  `distribute-flathub`, `distribute-mas`, `distribute-msstore`), holding
  only that channel's credential. A failed downstream job cannot modify the
  release: it has no write permission, and the release is immutable.
  `check-workflow-gate.py` is extended to fail if `distribute.yml` gains
  `contents: write` or calls `gh release upload|edit|delete`.
- Each job is independent: one channel's failure blocks no other, and
  re-running a job is idempotent.

### 6.2 `release-meta.json` extension (PR D)

Additive, so existing readers keep working:

```json
{
  "tag": "v0.7.0",
  "version": "0.7.0",
  "channel": "stable",
  "commit": "…",
  "manifest": "latest.json",
  "updater_signature": "minisign",
  "platforms": { "aarch64-apple-darwin": { "codesign": "developer-id", "notarized": true } },
  "artifacts": [
    {
      "name": "DeckPilot_0.7.0_aarch64.dmg",
      "sha256": "…",
      "target": "aarch64-apple-darwin",
      "distribution_channel": "github",
      "package_format": "app",
      "update_provider": "github_release"
    },
    {
      "name": "DeckPilot_0.7.0_amd64_apt.deb",
      "sha256": "…",
      "target": "x86_64-unknown-linux-gnu",
      "distribution_channel": "apt",
      "package_format": "deb",
      "update_provider": "external_package_manager"
    }
  ]
}
```

`artifacts[].distribution_channel` and `package_format` are read from each
built binary's embedded record, not typed in. `check-assets.sh` gains the
`_apt.deb` name (and its absence from the updater manifest).

### 6.3 Common verification step

Every downstream job starts with the same script,
`tools/distribution/verify-release-input.sh <tag>` (PR G). It:

1. reads the release through the API. It must be published, not a draft,
   `immutable: true`, with the pre-release flag matching the tag;
2. downloads `SHA256SUMS` and `release-meta.json`, and checks that the tag
   still points at `release-meta.json`'s `commit`;
3. downloads only the assets the job needs and verifies them against
   `SHA256SUMS` and `release-meta.json`'s `artifacts[]`, including
   `distribution_channel`;
4. writes `input.json` (tag, version, `release_channel`, commit, the
   verified asset paths and hashes) for the channel step.

### 6.4 Downstream jobs

Named here, implemented later (PR G as dry runs first). Nothing in this PR
publishes.

| Job | Inputs (verified) | Output | Credential (future) |
| --- | --- | --- | --- |
| `distribute-homebrew` | `input.json`; both DMG names and SHA-256s | A cask update (`version`, `sha256` per arch, `auto_updates true`) as a PR to the tap or `Homebrew/homebrew-cask`; stable releases only | GitHub token for the fork |
| `distribute-winget` | `input.json`; MSI name, SHA-256, `ProductCode` (read from the MSI) | WinGet manifests (version, installer, locale) as a PR to `microsoft/winget-pkgs`; pre-releases under a separate identifier | GitHub token for the fork |
| `distribute-apt` | `input.json`; `DeckPilot_<v>_amd64_apt.deb` | Repository files: `pool/`, `dists/<suite>/main/binary-amd64/Packages`, `Release`, `InRelease`, `Release.gpg`; suite `stable` or `prerelease`; pushed to the repository host | Repository GPG key; host deploy credential |
| `distribute-flathub` | `input.json`; the commit; generated `cargo-sources.json` and `node-sources.json` | A PR to `flathub/io.github.knittingcode.deckpilot` updating the manifest's `tag`/`commit` | GitHub account on the Flathub repository |
| `distribute-mas` | `input.json`; the commit | MAS variant built from that commit, signed with Apple Distribution, uploaded to App Store Connect; never submitted for review by the job | Apple Distribution, Mac Installer Distribution, provisioning profiles, App Store Connect API key |
| `distribute-msstore` | `input.json`; the MSI URL (MSI listing), or the commit (MSIX) | A Partner Center submission draft with the versioned MSI URL, or an MSIX upload; never committed for certification by the job | Partner Center API credentials; Authenticode for the MSI listing |

Each job also uploads its outputs as a workflow artifact and attests them
(`actions/attest-build-provenance`), so what was submitted can be traced
to the verified input.

## 7. Agent update lifecycle

This applies to the GitHubRelease provider, which replaces the bundle
itself. For other channels the store or package manager does steps 2-7,
and DeckPilot handles the result from step 8 on (section 7.3).

### 7.1 The eleven steps

| # | Step | Today | Target |
| --- | --- | --- | --- |
| 1 | **Detect an eligible update.** The provider is `github_release`; the channel accepts the version (R5); the manifest validates (G10); the exact format key exists (R3) | Check plus `version_comparator` (`updates.rs:508-578`) | + R3, G10 |
| 2 | **Download and verify.** minisign signature and signed version; nothing partial is kept | `updates.rs:580-630` | Unchanged |
| 3 | **Preflight.** Workflows: by policy, wait (automatic mode retries later) or, on an explicit user choice, cancel and record them as interrupted. Never end them silently. Automatic mode: deck idle for 10 minutes. The install location is writable | Workflows checked (`updates.rs:656-665`); idle (`policy.rs:99-123`) | + an explicit "cancel workflows and install" choice using `restart_agent`'s existing confirmation (`commands/agent.rs:48-67`) |
| 4 | **Record intent.** Write `<data dir>/update-intent.json`: from and to version, `distribution_channel`, the database schema version, the time. Data dir only, never the install dir | None | New (PR E) |
| 5 | **Quiesce the agent.** `PrepareForUpdate`: the agent stops accepting new runs, finishes the current request, stops plugin hosts, ends the work session, blanks and releases every device, closes the database, removes the token and exits. Old agents: protocol 1 `Shutdown` | `agent.rs:127-143`, `service.rs:603-634` | + refuse new `RunWorkflow` after `PrepareForUpdate` |
| 6 | **Prove it stopped. No two agents.** Wait until the IPC endpoint is gone **and** the instance lock (`agent.lock`) can be taken. The endpoint closes before the lock is released (`instance.rs:8-12`). Briefly hold the lock to prove it, then release it. Time out at 15 s and abort (nothing installed) | Endpoint only (`agent.rs:145-156`) | + lock probe (PR E) |
| 7 | **Install.** The Tauri updater replaces the bundle: macOS swaps the `.app` and restores it on failure; Windows hands over to the MSI or NSIS installer and exits; Linux runs `pkexec dpkg -i`, or replaces the AppImage. The data dir is never touched | `updates.rs:691-715` | Unchanged |
| 8 | **Restart.** The app restarts; the new app starts the new agent from its own bundle (`agent_executable`, never `DECKPILOT_AGENT_BIN` in release builds, G6). The instance lock guarantees one agent; a second start exits | `updates.rs:633-637`, `main.rs:127-136`, `main.rs:243-257` | + env override debug-only |
| 9 | **Migrate data.** The new agent opens the database: integrity check, verified encrypted backup, forward migrations in one transaction, verification (`sqlite.rs:261-370`); `plugins.db` likewise with its own migrations | As described | Unchanged; the intent file records the backup path from `migration_history` |
| 10 | **Verify compatibility.** The app checks the agent's version (`compatibility::check`), protocol, and the agent's `distribution_channel` against its own (R6); devices are re-enumerated | Version and protocol (`agent.rs:80-104`) | + distribution field in `GetStatus` (PR E) |
| 11 | **Commit or recover.** Success: delete the intent, log it to `updates.log`. Failure: see 7.2 | Failure before install: previous agent restarted (`updates.rs:701-712`) | + start-up recovery from the intent file |

Implemented by PR E: step 4 (`deckpilot_update::intent`, written by the
install sequence in `updates.rs`), the lock probe of step 6
(`deckpilot_ipc::instance`, also used by "restart agent"), step 8's
debug-only `DECKPILOT_AGENT_BIN` (`deckpilot_distribution::agent_executable_override`),
step 10's distribution field (`AgentStatus::distribution`) and channel
check (`AgentCompatibility::OtherDistribution`), and step 11's start-up
recovery (`updates::recover`). Step 5 needed no change: the agent's run
loop ends with `PrepareForUpdate`, so no run can start after it. Still
open: the explicit "cancel workflows and install" choice of step 3 (the
installation keeps refusing while workflows run), the version-marker check
of section 7.3, recording the backup path from `migration_history` in the
intent (the advice names the backup by its schema instead), and a restore
command for pre-migration backups (G7).

### 7.2 Recovery and rollback

Rollback is safe only where no data was migrated. The evidence:
migrations are forward-only (`sqlite.rs:27-85`, no down-migrations), an
older build refuses a newer schema before writing anything
(`sqlite.rs:193-199`, `263-270`), the 0.5.0 agent is measured to refuse a
0.6.0 database and leave it byte-identical (`docs/compatibility.md:75`),
and `Channel::accepts` never offers an older version (`channel.rs:54-56`).

| Failure point | State | Recovery | Safe? |
| --- | --- | --- | --- |
| Steps 1-3 | Nothing changed | Report; retry later | Yes |
| Step 5/6 (agent will not stop) | Old version, old agent running | Abort the install; the agent keeps running | Yes (today's behavior, `updates.rs:670-689`) |
| Step 7 (install fails) | Old version installed (macOS restores, Windows Installer rolls back, `dpkg` keeps the old package on error) | Restart the old agent; the update stays ready | Yes; same schema |
| Step 7, macOS cannot restore the old `.app` | Neither bundle at the path | Message names the kept copy (`updates.rs:431-435`); the data dir is intact | Yes for data; needs manual action |
| Step 8, new agent fails **before** migrating | New binaries, old schema | Reinstalling the previous version is safe (the schema matches) | Yes |
| Step 9, migration fails | The transaction rolled back; the database stays at the old schema; the backup exists | The new agent reports the error; reinstalling the previous version is safe | Yes |
| After step 9 succeeded, a later problem | New schema | **Binary rollback is not safe**: the old build refuses the database. Options: stay and wait for a fix release (always offered); or restore the pre-migration backup from `backups/` and reinstall the old version (manual; no restore command exists, G7). Changes made after the upgrade are lost | No, without a restore |
| Interrupted at any point (power loss) | The intent file remains | At the next start the app reads it and compares the installed version, the running agent and the schema. It then either finishes (restarts the agent) or shows what happened and the options above | Depends on the step, as above |

Policy: DeckPilot never performs an automatic binary rollback after a
migration, and never installs an older version. A restore command for
pre-migration backups is a separate feature (G7). Until it exists, this
document and `docs/updates.md` describe the manual path.

### 7.3 Updates done by a store or package manager

A store may replace the bundle while the agent runs. On macOS the App Store
generally quits the app first; MSIX may defer or force-close; `apt` replaces
files under a running process (all **Unverified** for DeckPilot). Then:

- the running agent keeps working from its loaded image until it is
  restarted; on Linux its executable may be a deleted inode;
- the next app start finds `AgentOutdated` (`compatibility.rs:39-56`) and
  restarts the agent, waiting for workflows as in step 3. This exists
  today;
- PR E adds a check in the agent: once a minute it compares its own
  executable's version marker with its running version, and if they differ
  it tells connected apps ("restart required"). It never restarts itself
  during a workflow;
- steps 9-11 apply unchanged.

User data stays outside every install directory in every channel (audit
section 8; capability matrix 2.9). That is why a package manager may
replace the bundle without touching configuration.

## 8. Packaging matrix

Machine-readable in [`targets.toml`](targets.toml). "Today" rows ship in
`v0.6.0-alpha.3`; the others are planned, and their capability verdicts are
in the [capability matrix](capability-matrix.md).

| Target | OS / arch | Format | Channel | Signing identity | Entitlements / sandbox | Update provider | Agent install | Plugin host install | Data dir | Status |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `github-macos-arm64` | macOS 13+ / aarch64 | `.app` in `.dmg`, `.app.tar.gz` | `github` | Developer ID Application (`PUB9L83UMZ`), notarized, stapled | Hardened runtime; no entitlements except the plugin host's one | `github_release` | `externalBin` in `Contents/MacOS`; LaunchAgent via `auto_launch` | `Contents/MacOS/deckpilot-plugin-host`, signed before bundling, the host alone with `allow-unsigned-executable-memory` (`allow-jit` is not enough for wasmtime 50: capability matrix, [E2]); bundled plugins in `Contents/Resources/bundled` | `~/Library/Application Support/io.github.knittingcode.deckpilot` | Today |
| `github-macos-x64` | macOS / x86_64 | as above | `github` | as above | as above | `github_release` | as above | as above | as above | Today |
| `github-windows-msi` | Windows 10+ / x86_64 | MSI | `github` | **None** (Authenticode not provisioned) | None | `github_release` | `externalBin` next to the app; HKCU Run | `externalBin` next to the agent; AppContainer + Job Object by the supervisor; bundled plugins in `bundled\` of the installation directory | `%APPDATA%\io.github.knittingcode.deckpilot` | Today |
| `github-windows-nsis` | Windows / x86_64 | NSIS `-setup.exe` | `github` | **None** | None | `github_release` | as MSI | as MSI | as MSI | Today |
| `github-linux-deb` | Linux / x86_64 | `.deb` | `github` | None (minisign `.sig` for updates) | None; udev rule installed | `github_release` (`pkexec dpkg -i`) | `/usr/bin/deckpilot-agent`; XDG autostart | `/usr/bin/deckpilot-plugin-host`; Landlock + seccomp; bundled plugins in `/usr/lib/DeckPilot/bundled` | `~/.local/share/io.github.knittingcode.deckpilot` | Today |
| `github-linux-appimage` | Linux / x86_64 | AppImage | `github` | None (minisign `.sig`) | None; udev rule manual | `github_release` | Inside the AppImage; autostart runs the AppImage with `--agent` | Inside the AppImage (`usr/bin`, bundled plugins in `usr/lib/DeckPilot/bundled`) | as `.deb` | Today |
| `github-linux-rpm` | Linux / x86_64 | `.rpm` | `github` | None | None; udev rule installed | `github_release` | `/usr/bin` | as `.deb` | as `.deb` | Stable releases only |
| `homebrew-cask` | macOS / both | GitHub `.dmg` | `github` (phase 1) | Developer ID (from GitHub) | as GitHub | `github_release`; cask `auto_updates true` | as GitHub | as GitHub | as GitHub | Planned |
| `winget-msi` | Windows / x86_64 | GitHub MSI | `github` (phase 1) | Authenticode advised | as GitHub | `github_release` | as GitHub | as GitHub | as GitHub | Planned |
| `msstore-msi-listing` | Windows / x86_64 | GitHub MSI or NSIS | `github` | Authenticode from a Trusted Root CA (required) | as GitHub | `github_release` | as GitHub | as GitHub | as GitHub | Planned, needs Authenticode |
| `apt-deb` | Linux / x86_64 (arm64 later) | `.deb` variant `_apt.deb` | `apt` | Repository GPG (InRelease) | None; udev rule installed | `external_package_manager` | `/usr/bin`; XDG autostart | as `github-linux-deb` | as `.deb` | Planned |
| `flathub` | Linux / x86_64, aarch64 | Flatpak | `flathub` | Flathub's key | Flatpak: `--device=all`, `--share=network`, `--talk-name=org.kde.StatusNotifierWatcher`, `--talk-name=org.freedesktop.secrets`; host udev rule by the user | `store_managed` | `/app/bin/deckpilot-agent`; Background portal autostart | `/app/bin`; nested Landlock + seccomp (Unverified) | `~/.var/app/io.github.knittingcode.deckpilot/data/…` | Feasibility |
| `mac-app-store` | macOS 13+ / universal | `.pkg` (sandboxed `.app`) | `mac_app_store` | Apple Distribution + Mac Installer Distribution, MAS provisioning | App Sandbox; `device.usb`, `network.client`, keychain access group; helper inherits | `store_managed` (no updater compiled) | `Contents/Library/LoginItems/DeckPilot Agent.app` via `SMAppService` (opt-in) | Not shipped (plugins off; XPC design Unverified) | Container `~/Library/Containers/io.github.knittingcode.deckpilot/Data/…` | Feasibility; blockers in the capability matrix, section 3 |
| `microsoft-store-msix` | Windows / x86_64 | MSIX (full trust) | `microsoft_store` | Signed by the Store; Partner Center publisher | `runFullTrust`; virtualized AppData and HKCU | `store_managed` (no updater compiled) | In the package; `windows.startupTask` | In the package; AppContainer child (Unverified) | Redirected per-package AppData | Feasibility |

Capabilities common to all targets: raw HID to Stream Decks, tray,
credential store, IPC between app and agent in the user's data dir.
Where a target needs an adaptation, it is in the capability matrix.

## 9. Updates UI per channel

What `updates-card.tsx` and `update-window.tsx` show (PR F), driven only by
`UpdateState.provider` and `UpdateState.distribution`, never by guessing:

| Element | GitHub (`github_release`) | APT (`external_package_manager`) | Homebrew / WinGet dedicated (reserved) | Flathub, Mac App Store, Microsoft Store (`store_managed`) | No updates (`none`) |
| --- | --- | --- | --- | --- | --- |
| Current version, channel label | Shown | Shown | Shown | Shown | Shown |
| "Installed from" line | "GitHub Releases (`.dmg`, MSI, …)" | "APT repository" | "Homebrew" / "WinGet" | "Flathub" / "Mac App Store" / "Microsoft Store" | "Development build" or "No update key" |
| Check for updates | Shown | Shown only if the version notice is on | as APT | **Hidden** | Disabled with today's hint |
| Available version and notes | Shown | Notice only: "Version X is available; update with your package manager" | as APT | Hidden | Hidden |
| Download / Install and restart / progress | Shown | **Hidden** | Hidden | Hidden | Hidden |
| Update command (copyable) | Hidden | `sudo apt update && sudo apt upgrade` | `brew upgrade --cask deckpilot` / `winget upgrade …` | Hidden | Hidden |
| Mode (notify, download-and-ask, install-when-idle) | Shown | **Hidden** | Hidden | Hidden | Hidden |
| Stable / prerelease channel choice | Shown | Hidden (the repository suite decides) | Hidden | Hidden | Hidden |
| Managed notice | Hidden | Hidden | Hidden | "Updates are installed by {store}" | Hidden |
| Update failure messages (`updates.rs:407-464`) | Shown | n/a | n/a | n/a | n/a |
| "Restart agent" after an external update | When `AgentOutdated` | When `AgentOutdated` | When `AgentOutdated` | When `AgentOutdated` | When `AgentOutdated` |

Wording rules: never "Download from GitHub" in a store build; never offer
to switch channels in-app (R8); failure texts that say "install it from
the releases page" (`updates.rs:427-452`) appear only for `github_release`.

## 10. Implementation plan

Each PR is reviewable on its own, changes behavior only where it says so,
and avoids `apps/desktop/src/plugins`, `apps/agent/src/plugins` and
`plugins/`, which the marketplace work owns.

| PR | Scope | Owns (files) | Behavior change |
| --- | --- | --- | --- |
| **A** (this) | Audit, architecture, capability matrix, `targets.toml` | `docs/distribution/*`, `docs/README.md` (link) | None |
| **B** Distribution metadata and provider abstraction | `deckpilot-distribution` crate (build-time record, validation, debug override, tests); `provider.rs` in `deckpilot-update` (trait, kinds, capabilities, the channel→provider table, R1/R2/R9 as pure functions); `targets.toml` parsed in a test so the docs and the code agree | `crates/deckpilot-distribution/**` (new), `crates/deckpilot-update/src/provider.rs`, `crates/deckpilot-update/src/lib.rs` (module line), root `Cargo.toml` (member), `Cargo.lock` (refreshed after any version bump on main) | None: nothing uses it yet |
| **C** GitHub updater integration | `GitHubRelease`, `ExternalPackageManager`, `StoreManaged` and `None` providers in the desktop crate; `UpdateState` gains `distribution` and `provider`; R3, R4, G10 client-side manifest check; `self-update` Cargo feature | `apps/desktop/src-tauri/src/updates.rs`, `updates_tests.rs`, new `apps/desktop/src-tauri/src/update_provider.rs`, `apps/desktop/src-tauri/src/main.rs` (wiring), `apps/desktop/src-tauri/Cargo.toml` | GitHub builds: none apart from the extra manifest check. Builds without the record: `None` |
| **D** Packaging-matrix validation | Set `DECKPILOT_DISTRIBUTION_CHANNEL=github` in the release and packaging builds; check each binary's embedded record and the absence of the override; `release-meta.json` `artifacts[]`; macOS bundle and AppImage in `packaging.yml`; `check-workflow-gate.py` rule for `distribute.yml`; `targets.toml` versus the built assets | `.github/workflows/release.yml`, `.github/workflows/packaging.yml`, `tools/ci/check-packages.sh`, `tools/ci/test-check-packages.sh`, `tools/release-pipeline/release-meta.sh`, `check-assets.sh`, `test.sh`, `check-workflow-gate.py`, `docs/releasing.md` | Release metadata gains fields; no user-visible change |
| **E** Agent update lifecycle | Steps 3-6, 8, 10, 11 and 7.3; `update-intent.json` and start-up recovery; lock probe; refusing runs after `PrepareForUpdate`; `distribution` in `GetStatus` as an optional field with `#[serde(default)]` (`AgentStatus`, `crates/deckpilot-ipc/src/protocol.rs:1091-1093`, does not deny unknown fields, so older clients ignore it; a protocol bump is needed only if a request is added); `DECKPILOT_AGENT_BIN` debug-only; autostart strategy selected by the record (GitHub, APT, Homebrew and WinGet keep `auto_launch`; store strategies are stubs that report "unsupported") | `apps/agent/src/service.rs`, `apps/agent/src/instance.rs`, `apps/agent/src/autostart.rs`, `crates/deckpilot-ipc/src/protocol.rs`, `apps/desktop/src-tauri/src/agent.rs`, `apps/desktop/src-tauri/src/commands/agent.rs`, `apps/desktop/src-tauri/src/updates.rs` (install sequence only, after C), `docs/updates.md`, `docs/background-agent.md`, `docs/compatibility.md` | Safer installs; same flow for users |
| **F** Distribution-aware update UI | Section 9; "Installed from"; copyable commands; managed notice; the version-notice setting for external package managers | `apps/desktop/src/components/updates-card.tsx`, `apps/desktop/src/windows/update-window.tsx`, `apps/desktop/src/lib/api.ts` (types), `apps/desktop/src/windows/feature-windows.test.tsx`, a new `updates-card.test.tsx` | UI only; the GitHub build looks the same except for the "Installed from" line |
| **G** Store feasibility | Prototypes for the Unverified cells (MAS: sandboxed hidapi open of a Stream Deck, SMAppService helper, Keychain group; MSIX: hidapi and an AppContainer child from full trust; Flatpak: hidraw with `--device=all`, nested Landlock and seccomp, Background portal); a draft Flatpak manifest; `distribute.yml` with the verification step and **dry-run** jobs that build outputs but publish nothing | `experiments/distribution/**` (new), `packaging/flatpak/**` (new), `packaging/apt/**` (new), `.github/workflows/distribute.yml` (new, dry run only), `tools/distribution/**` (new), updates to `docs/distribution/capability-matrix.md` with results | None for users |

Ordering: B, then C and D in parallel (C needs B's types; D needs B's
build-time variable), then E (after C, because both touch the install
sequence in `updates.rs`), then F (after C's `UpdateState` fields), and G at
any point after B. Real publication to any channel (credentials, the first
submission, the APT host) is a separate, explicitly authorized step after
G. It is not part of these PRs.
