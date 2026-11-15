# Distribution capability matrix

Whether DeckPilot, as it is built today ([audit](audit.md)), can work in each
distribution channel, and what would have to change. It is a planning
document. **Nothing here claims that a store would accept DeckPilot**: no
channel other than direct GitHub download has been tried, and store review
outcomes cannot be known in advance.

Verdicts:

| Verdict | Meaning |
| --- | --- |
| **Supported** | Works with today's design; the platform documents it, and for direct GitHub it has been shipped |
| **Requires adaptation** | Possible according to the platform's documentation, but DeckPilot's code, packaging or configuration must change |
| **Blocked** | The platform's documented rules or technical limits rule out today's design; a different design would be needed, and might still be refused |
| **Unverified** | Not established: no document settles it and DeckPilot has not tested it. Needs a prototype (PR G) |

A verdict of Supported or Requires adaptation describes the platform's
documented rules. It says nothing about DeckPilot passing review. Where a
verdict rests on our reading of a rule rather than on the rule's wording,
the cell says "reading". Sources are listed in [section 6](#6-sources) and
cited as [S1], [S2] and so on.

Channels:

| Channel | `distribution_channel` | Artifact | Who updates |
| --- | --- | --- | --- |
| Direct GitHub download | `github` | `.dmg` / `.app.tar.gz`, MSI, NSIS, `.deb`, AppImage, `.rpm` (stable only) | The signed Tauri updater (today) |
| Homebrew Cask | `homebrew` (reserved), see [section 4](#4-homebrew-cask-and-winget) | The GitHub `.dmg` | The app itself (`auto_updates true`) |
| WinGet | `winget` (reserved), see [section 4](#4-homebrew-cask-and-winget) | The GitHub MSI | The app itself; `winget upgrade` reinstalls |
| APT repository | `apt` | A dedicated `.deb` variant | `apt upgrade` |
| Flathub | `flathub` | Flatpak built by Flathub from source | `flatpak update` |
| Mac App Store | `mac_app_store` | Sandboxed `.app` in a `.pkg`, Apple Distribution signed | The App Store |
| Microsoft Store (MSIX) | `microsoft_store` | MSIX, full trust | The Store |
| Microsoft Store (MSI/EXE listing) | `github` | The GitHub MSI or NSIS, Authenticode signed | The app itself |

## 1. Matrix

Short form; the reasons are in [section 2](#2-per-capability-detail).

| Capability | GitHub | Homebrew Cask | WinGet | APT | Flathub | Mac App Store | MS Store (MSIX) | MS Store (MSI listing) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| App sandbox and entitlements | Supported | Supported | Supported | Supported | Requires adaptation | Requires adaptation | Supported (full trust) | Supported |
| Separate background agent | Supported | Supported | Supported | Supported | Requires adaptation | Requires adaptation | Requires adaptation | Supported |
| Autostart at login | Supported | Supported | Supported | Supported | Requires adaptation | Requires adaptation | Requires adaptation | Supported |
| Raw USB HID (hidapi) | Supported | Supported | Supported | Supported | Requires adaptation | **Unverified** | **Unverified** | Supported |
| Tray / menu bar | Supported | Supported | Supported | Supported | Requires adaptation | Supported (reading) | Supported (reading) | Supported |
| Spawning the plugin host | Requires adaptation | Requires adaptation | Requires adaptation | Requires adaptation | Requires adaptation | Requires adaptation | **Unverified** | Requires adaptation |
| Plugin host's own OS sandbox | Supported (verified on CI runners, not bundled) | as GitHub | as GitHub | as GitHub | **Unverified** | **Blocked** as designed | **Unverified** | as GitHub |
| JIT / executable memory (wasmtime) | Requires adaptation | Requires adaptation | Supported | Supported | Supported (reading) | Supported (reading), **Unverified** | Supported | Supported |
| Downloaded plugin code | Supported | Supported | Supported | Supported | Supported (reading) | **Blocked** (policy risk) | **Unverified** (policy) | Unverified (policy) |
| Credential store | Supported | Supported | Supported | Supported | Requires adaptation | Requires adaptation | Supported (reading), Unverified | Supported |
| File system / data location | Supported | Supported | Supported | Supported | Requires adaptation | Requires adaptation | Requires adaptation | Supported |
| Store's own update mechanism | n/a (Tauri updater) | Supported | Supported | Supported | Supported | Supported | Supported | n/a (self-update allowed) |
| Self-update by the Tauri updater | Supported | Supported (`auto_updates`) | Supported | **Blocked** (must not) | **Blocked** | **Blocked** | **Blocked** | Supported |
| udev rule for Stream Decks | `.deb`/`.rpm` only; AppImage manual | n/a | n/a | Supported | **Blocked** (host rule needed) | n/a | n/a | n/a |
| Code signing in place today | macOS yes; Windows, Linux no | via GitHub | via GitHub (unsigned) | no | Flathub signs | no | no | no |

### Identifiers

The bundle identifier `io.github.knittingcode.deckpilot` and the agent's
signing identifier `io.github.knittingcode.deckpilot.agent` must not change
(they name the data directory and bind the Keychain item; audit section 2).

| Channel | Identity the channel uses | Fits the current identifier? |
| --- | --- | --- |
| GitHub, Homebrew, WinGet, APT | The bundle identifier, the MSI `UpgradeCode`, the Debian package name | Yes (unchanged) |
| Flathub | Application ID, reverse DNS, verified by login of the `io.github` owner [S20] | `io.github.knittingcode.deckpilot` is a valid `io.github.*` ID (Unverified until submitted) |
| Mac App Store | Bundle ID registered in the developer account | Yes, if registered under team `PUB9L83UMZ` (Unverified) |
| Microsoft Store (MSIX) | Package identity name and publisher assigned by Partner Center | Different package family name; the app's own identifier constant can stay, but the virtualized data location differs (section 2.9) |

## 2. Per capability detail

### 2.1 App sandbox and entitlements

- **Mac App Store.** Apps must be sandboxed (App Review Guideline 2.4.5(i))
  [S1]. Every executable is signed with `com.apple.security.app-sandbox`.
  Helpers inherit the sandbox (`com.apple.security.inherit`) [S4]. Today's
  bundle has no entitlements at all, and CI enforces that
  (`.github/scripts/verify-macos-app.sh:26-28`). A MAS variant needs its own
  entitlements files and its own verification script. **Requires
  adaptation.**
- **Direct GitHub (macOS).** Developer ID plus hardened runtime, no sandbox
  required. This is what ships today. **Supported.**
- **MSIX.** A full-trust desktop app declares `runFullTrust` and runs at
  medium integrity level, not in an AppContainer [S11]. **Supported**,
  with the file system and registry virtualization described in 2.9.
- **Flathub.** Every app runs in the Flatpak sandbox. Permissions in
  `finish-args` are reviewed by Flathub [S16, S17]. DeckPilot needs at
  least `--device=all` (2.4), the tray name (2.5), Secret Service access
  (2.8) and network access. **Requires adaptation.**
- **Homebrew, WinGet, APT.** No sandbox. **Supported.**

### 2.2 Separate background agent

The agent is a second executable that runs without the configuration window
(audit section 7).

- **Mac App Store.** A helper that runs on its own is a login item
  registered with `SMAppService.loginItem(identifier:)` (macOS 13+). It must
  be a helper **app bundle** in `Contents/Library/LoginItems` [S5, S6]. Today
  the agent is a bare executable in `Contents/MacOS` (Tauri `externalBin`).
  A MAS variant must wrap it as `DeckPilot Agent.app`. Its bundle identifier
  `io.github.knittingcode.deckpilot.agent` already exists in
  `apps/agent/Info.plist`. **Requires adaptation.**
- **MSIX.** The full-trust package can contain several executables. Starting
  the agent at login is a `windows.startupTask` extension [S12]. Starting it
  from the app as a detached process is ordinary Win32 (reading).
  **Requires adaptation** (the startup task).
- **Flathub.** The agent runs in the same sandbox. It is kept alive with the
  Background portal (2.3). Starting it from the app works inside the
  sandbox (reading). **Requires adaptation.**
- **Others.** As today. **Supported.**

### 2.3 Autostart

Today: `auto_launch` with a `LaunchAgent` plist on macOS, an XDG autostart
file on Linux and the `HKCU` Run key on Windows
(`apps/agent/src/autostart.rs:41-56`).

- **Mac App Store.** Writing `~/Library/LaunchAgents` is outside the
  sandbox. `SMAppService` is the supported mechanism, and running at login
  needs the user's consent (Guideline 2.4.5(iii)) [S1, S5]. Autostart must
  be an opt-in setting. **Requires adaptation.**
- **MSIX.** HKCU writes by a packaged app go to a private per-package copy
  [S14], so the Run key would have no effect. Use
  `uap5:Extension Category="windows.startupTask"` with
  `Executable="deckpilot-agent.exe"` and
  `EntryPoint="Windows.FullTrustApplication"`; the user can disable it in
  Settings [S12]. **Requires adaptation.**
- **Flathub.** `org.freedesktop.portal.Background.RequestBackground` with
  `autostart: true` and a `commandline` [S18]. **Requires adaptation.**
- **Direct GitHub, Homebrew, WinGet, APT, MSI listing.** As today.
  **Supported.** For APT the entry points at `/usr/bin/deckpilot-agent`,
  the same as the GitHub `.deb` (`autostart.rs:86-92`).

### 2.4 Raw USB HID

The agent opens Stream Decks with `hidapi` (IOHIDManager on macOS, hidraw on
Linux, SetupDi/HID on Windows).

- **Mac App Store.** `com.apple.security.device.usb` is documented as giving
  access to "USB device access APIs" [S2]. Apple documents no HID-specific
  entitlement. A developer reported a TCC denial of `IOHIDDeviceOpen` with
  the USB entitlement set, without a documented fix [S9]. Whether hidapi
  can open a Stream Deck from a sandboxed process is **Unverified** and is
  the first prototype in PR G. It may also need the Input Monitoring
  permission. If it fails, the Mac App Store is **Blocked** for DeckPilot,
  because the agent's whole job is talking to the device.
- **MSIX (full trust).** The `humaninterfacedevice` capability gates the
  WinRT `Windows.Devices.HumanInterfaceDevice` API, which full-trust Win32
  code does not use [S13]. hidapi through SetupDi and `CreateFile` should
  work as in an unpackaged process (reading). **Unverified.**
- **Flathub.** Flatpak has no hidraw device option. `--device=all` is the
  only way to reach `/dev/hidraw*` (`--device=usb` covers `/dev/bus/usb`
  only) [S16]. StreamController, a Stream Deck app on Flathub, ships with
  `--device=all` [S21]. Even then, the hidraw nodes are owned by root
  unless a **host** udev rule gives the user access. A Flatpak cannot
  install one, so users must install
  `packaging/linux/70-deckpilot-streamdeck.rules` themselves.
  **Requires adaptation** (permission, documentation, first-run check).
- **APT.** The `.deb` installs the udev rule, as the GitHub `.deb` does.
  **Supported.**
- **AppImage.** No udev rule is installed (audit section 2). It works only
  after a manual install of the rule. Supported, as today.

### 2.5 Tray / menu bar

- **Flathub.** `tray-icon` uses StatusNotifierItem over D-Bus, which needs
  `--talk-name=org.kde.StatusNotifierWatcher` [S22]. **Requires
  adaptation.**
- **Mac App Store.** Nothing restricts a menu bar item (reading).
  **Supported (reading).**
- **MSIX.** The notification area is ordinary Win32 (reading).
  **Supported (reading).**
- **Others.** **Supported.**

### 2.6 Spawning the plugin host and its OS sandbox

The plugin host is **not bundled today** (audit section 9). Every channel
therefore needs packaging work before plugins can run ("Requires
adaptation" in the matrix). What remains in question is whether the host's
own sandbox can be applied inside each channel's sandbox.

- **Direct GitHub (and Homebrew, WinGet, APT, MSI listing).** The host
  applies `sandbox_init` (macOS), Landlock and seccomp (Linux), or the
  supervisor applies an AppContainer plus a Job Object (Windows). This is
  verified in CI on GitHub's runners only (`plugins/runtime/README.md:164-186`).
  Packaging adds: a second `externalBin`, signing with the same identity,
  and on macOS the JIT entitlement (2.7). **Supported (technically) once
  bundled.**
- **Mac App Store. Blocked as designed.** A child of a sandboxed process
  always inherits the parent's sandbox, and "a process is not allowed to
  change its sandbox" [S4, S10]. Apple's guidance is that custom
  `sandbox_init` profiles are unsupported and helpers should be XPC
  services [S10]. The host's self-applied deny-default profile
  (`plugins/runtime/crates/host/src/sandbox/macos.rs`) would fail or be
  redundant, and the agent requires a `full` sandbox report
  (`apps/agent/src/plugins/mod.rs:105-118`), so plugins would fail closed.
  An alternative design runs the host as an XPC service with its own
  minimal App Sandbox entitlements (no network, no files, no USB).
  **Unverified**, and it would need a new sandbox report kind.
- **Flathub.** Flatpak's seccomp filter is a blocklist. It does not block
  `seccomp`, `prctl` or the Landlock syscalls, and bubblewrap already sets
  `no_new_privs` [S19]. Stacking the host's seccomp filter and Landlock
  inside the Flatpak sandbox should therefore work if the host kernel has
  Landlock (reading). **Unverified.**
- **MSIX.** Nested Job Objects are supported since Windows 8 [S15]. No
  document was found on a full-trust packaged process creating an
  AppContainer profile and launching a child in it. **Unverified.**

### 2.7 JIT and executable memory

wasmtime 50 with Cranelift writes machine code and makes it executable
(`plugins/runtime/crates/host/src/engine.rs:186-198`).

- **Direct GitHub / Homebrew (macOS).** Notarization requires the hardened
  runtime, which forbids executable writable memory unless the binary has
  `com.apple.security.cs.allow-jit` (with `MAP_JIT`) [S3] or
  `com.apple.security.cs.allow-unsigned-executable-memory`. Whether
  wasmtime 50 needs one, and which, is **not verified**
  (`plugins/runtime/README.md:174-175`). The entitlement must be granted to
  the plugin host binary **only**, and `verify-macos-app.sh` changed to
  allow exactly that. **Requires adaptation.** Alternative: wasmtime's
  Pulley interpreter, which needs no executable memory, at a performance
  cost that has not been measured.
- **Mac App Store.** The hardened runtime is not required for App Store
  apps [S3]. Apple engineers state that the Mac App Store does not restrict
  JIT [S7]. **Supported (reading), Unverified** for DeckPilot.
- **MSIX full trust.** The `codeGeneration` capability only restricts
  AppContainer apps [S13]. **Supported.** The host inside its own
  AppContainer already runs Cranelift code in CI.
- **Flathub.** Flatpak has no W^X policy (reading). **Supported
  (reading).**
- **Windows, Linux direct.** No restriction. **Supported.**

### 2.8 Credential store

The database key is kept under the service `DeckPilot`
(`crates/deckpilot-keystore/src/lib.rs:110-127`). On macOS it is bound to
the agent's code identity.

- **Mac App Store.** The Keychain is available. Sharing items between the
  app and the agent helper needs `keychain-access-groups` with the team
  prefix [S8]. Items written by the Developer ID build are bound to that
  build's designated requirement. A build signed with Apple Distribution
  has a different requirement, so it cannot read them without a prompt, or
  at all (reading). Users moving from the GitHub build to the MAS build
  must re-enter a password or import an encrypted export. **Requires
  adaptation.**
- **MSIX full trust.** Credential Manager (DPAPI) is available to
  medium-IL processes (reading). **Supported (reading), Unverified.**
- **Flathub.** The keyring crate talks to the Secret Service over D-Bus,
  which needs `--talk-name=org.freedesktop.secrets`. The alternative, the
  Secret portal, gives a per-app master secret instead of the Secret
  Service API [S23] and would need a different `KeyStore` backend.
  **Requires adaptation.**
- **Others.** As today. **Supported.**

### 2.9 File system and data location

Data lives in `dirs::data_dir()/io.github.knittingcode.deckpilot`
(`crates/deckpilot-ipc/src/paths.rs:66-77`), never in the install
directory.

- **Mac App Store.** `dirs::data_dir()` resolves inside the container:
  `~/Library/Containers/io.github.knittingcode.deckpilot/Data/Library/Application Support/…`.
  The app and the agent helper must share it, either through an App Group
  container or by being in one container (reading). Existing GitHub users'
  data is not visible to the sandboxed build without a migration.
  **Requires adaptation.**
- **MSIX.** The package directory is read-only. New files a packaged app
  writes to AppData are redirected to a per-package location [S14], and
  that location is removed when the package is uninstalled (reading).
  **Requires adaptation**: document it, and have the export reminder
  mention it.
- **Flathub.** `XDG_DATA_HOME` is `~/.var/app/io.github.knittingcode.deckpilot/data`.
  Data written by the `.deb` or AppImage is not visible unless the manifest
  grants `--filesystem=xdg-data/io.github.knittingcode.deckpilot` (reading;
  Flathub review decides). **Requires adaptation.**
- **Others.** **Supported.**

### 2.10 Updates and the store's own mechanism

- **Mac App Store.** Guideline 2.4.5(vii): apps must be updated through the
  Mac App Store; "other update mechanisms are not allowed" [S1]. The
  Tauri updater must be **compiled out**, not merely disabled. **Blocked
  for self-update; StoreManaged.**
- **Downloaded plugins on the Mac App Store.** Guidelines 2.4.5(iv) and
  2.5.2 bar downloading or executing code that adds or changes features.
  Guideline 4.7 permits some plug-in models under extra rules [S1].
  User-installed WebAssembly plugins are DeckPilot's most likely review
  problem there. **Blocked (policy risk)** until the marketplace design
  addresses it.
- **MSIX.** The package directory is read-only and owned by the
  deployment stack, so the Tauri updater cannot replace files (reading of
  [S14]). Store policy 10.2.2 forbids changing functionality through
  dynamically downloaded code [S24]. **Blocked for self-update;
  StoreManaged.**
- **Microsoft Store MSI/EXE listing.** Policy 10.2.9 lists unpackaged
  `.msi`/`.exe` apps that are Authenticode signed by a certificate from the
  Microsoft Trusted Root Program. They need a versioned HTTPS URL whose
  binary never changes, and a silent install [S24, S25]. Such apps
  update themselves (Microsoft documentation as seen in search results,
  not checked against the page text) [S26]. **Supported** if Windows
  signing is provisioned. Because the binary is the GitHub MSI, this
  listing is the `github` channel.
- **Flathub.** Updates arrive through `flatpak update`. `/app` is
  read-only. Flathub builds from source with no network access during the
  build [S16]. No Flathub document was found that forbids self-updaters in
  so many words. A community report says they are not accepted [S27], and
  they could not work anyway because `/app` is read-only. **Blocked for
  self-update; StoreManaged.**
- **Homebrew Cask.** The `auto_updates true` stanza is meant for apps with
  their own "Check for Updates"; `brew upgrade` then leaves them alone
  unless `--greedy` is given [S28]. Since Homebrew 5.0, casks must pass
  Gatekeeper; `--no-quarantine` is deprecated and casks that fail Gatekeeper
  are to be disabled from September 2026 [S29, S30]. The notarized
  Developer ID DMG passes. **Supported.**
- **WinGet.** A manifest in `microsoft/winget-pkgs` points at a stable
  publisher HTTPS URL. The installer is scanned and checked for URL
  reputation; it must install silently [S31]. Signing is not required by
  that document but lowers the false-positive risk. `winget upgrade` runs
  the newer installer; the MSI's upgrade logic handles a version the app
  already updated itself to (reading). **Supported.**
- **APT.** Users add a repository key under `/usr/share/keyrings` and a
  `signed-by` source [S32]. `dpkg` owns `/usr/bin/deckpilot-desktop` and
  `/usr/bin/deckpilot-agent`. A self-update through
  `pkexec dpkg -i <github .deb>` would put files under dpkg's control that
  the repository did not publish, and break `apt`'s view of the version.
  The APT variant **must not** self-update. **Blocked for self-update;
  ExternalPackageManager.**

## 3. Store blockers, in order of severity

1. **Mac App Store: raw HID from the sandbox is unverified** (2.4). If
   hidapi cannot open the device in a sandboxed process, the channel is
   out, whatever else is done.
2. **Mac App Store: user-installed plugin code** (2.10, Guidelines 2.4.5(iv),
   2.5.2, 4.7). This is a policy risk that a prototype cannot settle; only
   review can. Shipping the MAS build with plugins disabled is the only
   design without it.
3. **Mac App Store: the plugin host's self-applied sandbox is impossible
   inside App Sandbox** (2.6). This needs an XPC-service design and a new
   sandbox report.
4. **Mac App Store: Keychain and data continuity** with existing GitHub
   installs (2.8, 2.9).
5. **Flathub: hidraw needs `--device=all` and a host udev rule** (2.4). It
   is accepted for a comparable app [S21], but it is still a reviewer
   decision.
6. **MSIX: AppContainer child from a packaged full-trust process, and
   hidapi under MSIX**, are unverified (2.4, 2.6). The Microsoft Store
   MSI listing avoids both but needs Authenticode.
7. **APT: needs its own `.deb` build** so that the updater is off. The
   GitHub `.deb` must never be republished to the repository as is (audit
   G2).
8. **All stores: Windows Authenticode is not provisioned** (audit G11). It
   is required for the Microsoft Store MSI listing [S25] and strongly
   advisable for WinGet.

## 4. Homebrew Cask and WinGet

Both channels re-host the GitHub artifact unchanged, so the installed bytes
are those of the `github` channel. Homebrew documents `auto_updates true`
for exactly this case [S28]. WinGet re-runs the publisher's installer, and
the MSI's own upgrade rules apply (reading).

**Decision for the first phase:** the cask and the WinGet manifest point at
the GitHub assets, the installed app reports `distribution_channel = github`,
and it keeps its signed self-update. No second macOS or Windows build per
release is needed.

`homebrew` and `winget` stay reserved values. A later dedicated build
(for example, if Homebrew maintainers or users ask for `brew upgrade` to be
authoritative) would carry them, use the `ExternalPackageManager` provider,
and show `brew upgrade --cask deckpilot` or `winget upgrade <id>`. Such a
build needs its own notarized DMG or signed MSI attached to the release in
stage A (see [README.md, section 6](README.md#6-release-automation)).

## 5. Future signing and publishing credentials

None of these exist or are created by this work. Each is listed so that a
later PR knows what to ask the owner for. Secrets would live in a
channel-specific protected environment, never in `production-release`,
and never in a job that can change the GitHub release.

| Channel | Signing | Publishing access |
| --- | --- | --- |
| Direct GitHub | Existing: Developer ID Application (team `PUB9L83UMZ`), App Store Connect API key for notarization, Tauri minisign key (`.github/updater.pub`). New: **Windows Authenticode** (OV or EV certificate, or a cloud signing service). Optional: a GPG key for detached Linux signatures | Existing `GITHUB_TOKEN` in `production-release` |
| Homebrew Cask | None; the cask carries `sha256` and relies on Developer ID plus notarization [S29] | A GitHub token able to open PRs on a fork of `Homebrew/homebrew-cask`, or on an own tap |
| WinGet | None required [S31]; Authenticode strongly advised | A GitHub token able to open PRs on a fork of `microsoft/winget-pkgs` |
| APT | **Repository GPG key** (offline primary key, signing subkey) for `InRelease`/`Release.gpg`; its public key published for `signed-by` [S32] | Deploy credential for the repository host (for example a separate Pages repository) |
| Flathub | None of ours; Flathub builds and signs [S16] | A GitHub account with write access to `flathub/io.github.knittingcode.deckpilot`; app ID verification by the `io.github` owner [S20] |
| Mac App Store | **Apple Distribution** certificate; **Mac Installer Distribution** certificate; **Mac App Store provisioning profiles** for the app and for every helper (agent login item, a possible plugin XPC service), with `app-sandbox`, `application-identifier`, `team-identifier` and `keychain-access-groups` [S33] | An App Store Connect app record; an App Store Connect API key with a role allowed to upload builds |
| Microsoft Store (MSIX) | The Store signs the package; the manifest's `Publisher` must equal the Partner Center publisher identity (reading of [S34]) | **Microsoft Partner Center** developer account, a reserved app name, and Partner Center API credentials (Entra ID app: tenant, client ID, secret) for automated submissions |
| Microsoft Store (MSI listing) | **Authenticode** from a Microsoft Trusted Root Program CA, every PE file [S25] | Partner Center account as above |

## 6. Sources

Retrieved 2026-11-15. Apple documentation pages are the machine-readable
mirrors of the developer documentation.

- [S1] App Review Guidelines (2.4.5, 2.5.2, 4.7): https://developer.apple.com/app-store/review/guidelines/
- [S2] `com.apple.security.device.usb`: https://developer.apple.com/tutorials/data/documentation/bundleresources/entitlements/com.apple.security.device.usb.md
- [S3] `com.apple.security.cs.allow-jit`: https://developer.apple.com/tutorials/data/documentation/bundleresources/entitlements/com.apple.security.cs.allow-jit.md
- [S4] Embedding a helper tool in a sandboxed app: https://developer.apple.com/tutorials/data/documentation/xcode/embedding-a-helper-tool-in-a-sandboxed-app.md
- [S5] `SMAppService`: https://developer.apple.com/tutorials/data/documentation/servicemanagement/smappservice.md
- [S6] `SMAppService.loginItem(identifier:)`: https://developer.apple.com/tutorials/data/documentation/servicemanagement/smappservice/loginitem(identifier:).md
- [S7] Apple Developer Forums, JIT in Mac App Store apps: https://developer.apple.com/forums/thread/764282
- [S8] Sharing access to keychain items among a collection of apps: https://developer.apple.com/tutorials/data/documentation/security/sharing-access-to-keychain-items-among-a-collection-of-apps.md
- [S9] Apple Developer Forums, `IOHIDDeviceOpen` denied in a sandboxed app: https://developer.apple.com/forums/thread/706390
- [S10] Apple Developer Forums, custom sandbox profiles and helpers: https://developer.apple.com/forums/thread/747880, https://developer.apple.com/forums/thread/724608
- [S11] Microsoft, behind the scenes of packaged desktop apps (full trust, medium IL): https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes
- [S12] `StartupTask`: https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.startuptask
- [S13] App capability declarations (`humaninterfacedevice`, `codeGeneration`, `runFullTrust`): https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations
- [S14] MSIX file system and registry virtualization: https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes
- [S15] Nested jobs: https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs
- [S16] Flathub requirements: https://docs.flathub.org/docs/for-app-authors/requirements; Flatpak sandbox permissions: https://docs.flatpak.org/en/latest/sandbox-permissions.html
- [S17] Flathub linter: https://docs.flathub.org/docs/for-app-authors/linter
- [S18] XDG Background portal: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html
- [S19] Flatpak's seccomp filter (`common/flatpak-run.c`): https://github.com/flatpak/flatpak/blob/main/common/flatpak-run.c
- [S20] Flathub app ID verification: https://docs.flathub.org/docs/for-app-authors/verification
- [S21] StreamController on Flathub (`--device=all` for Stream Decks): https://github.com/flathub/com.core447.StreamController
- [S22] Tauri, Flatpak distribution: https://v2.tauri.app/distribute/flatpak/
- [S23] XDG Secret portal: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Secret.html
- [S24] Microsoft Store policies (10.2.2, 10.2.5, 10.2.9): https://learn.microsoft.com/en-us/windows/apps/publish/store-policies
- [S25] Microsoft Store MSI/EXE submission: https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/create-app-submission
- [S26] Distributing a Win32 app through the Microsoft Store (search result text only, not verified against the page): https://learn.microsoft.com/en-us/windows/apps/distribute-through-store/how-to-distribute-your-win32-app-through-microsoft-store
- [S27] Community report on self-updaters and Flathub: https://github.com/sectapunterx/tabless/pull/356
- [S28] Homebrew Cask Cookbook (`auto_updates`, `sha256`): https://docs.brew.sh/Cask-Cookbook
- [S29] Homebrew acceptable casks: https://docs.brew.sh/Acceptable-Casks
- [S30] Homebrew 5.0.0 (Gatekeeper, `--no-quarantine`): https://brew.sh/2025/11/12/homebrew-5.0.0/
- [S31] WinGet repository submission: https://learn.microsoft.com/en-us/windows/package-manager/package/repository
- [S32] Debian, using third-party repositories (`signed-by`): https://wiki.debian.org/DebianRepository/UseThirdParty
- [S33] Tauri, App Store distribution: https://v2.tauri.app/distribute/app-store/
- [S34] Tauri, Microsoft Store distribution: https://v2.tauri.app/distribute/microsoft-store/
