# Application updates

DeckPilot updates itself from **GitHub Releases** of
`knittingcode/DeckPilot`. Releases contain application binaries and update
metadata only. No profile, database, token or other user data is ever
uploaded.

## Channels and discovery

Settings → Updates → *Update channel* chooses between two channels; the
default is **Stable**. The choice is stored on this computer only (it is
not part of exports).

| | Stable (default) | Pre-releases |
| --- | --- | --- |
| Offers | stable releases (`vX.Y.Z`) | the newest of pre-releases (`-alpha.N`, `-beta.N`, `-rc.N`) **and** stable releases |
| Where the app looks | `https://github.com/knittingcode/DeckPilot/releases/latest/download/latest.json` | GitHub's release list (`api.github.com/repos/knittingcode/DeckPilot/releases`), then the manifest of the chosen release |
| Manifest | `latest.json` of the latest release | `prerelease.json` of a pre-release, `latest.json` of a stable release |

Each release carries exactly one manifest, named for its channel:
`latest.json` in a stable release, `prerelease.json` in a pre-release
(`deckpilot-dev verify-release` refuses a release with the other one, and
the release workflow attaches only the right one). The channels are
isolated three times over:

1. GitHub's `latest` alias never points at drafts or pre-releases;
2. a pre-release has no `latest.json`, so even a pre-release marked
   "latest" by mistake offers nothing to stable installations (the download
   fails as "no update description");
3. the app refuses any pre-release version on the stable channel, whatever
   manifest announced it (the updater's version comparison,
   `Channel::accepts`).

On the pre-release channel, the app reads the release list (30 newest
releases), ignores drafts, tags that are not release tags and releases
whose GitHub pre-release flag disagrees with their tag, and takes the
highest version newer than the running one. It then reads that release's
manifest; the URL is built from the validated tag, never taken from the
list. Signatures, the signed version and the trusted download prefix are
checked exactly as on the stable channel. The list request is
unauthenticated (GitHub allows 60 per hour per address; the app checks at
most hourly).

**Moving between channels.** A pre-release user is offered the stable
release as soon as it is newer than the pre-release they run
(`0.7.0` > `0.7.0-rc.2`). Switching back to Stable never installs an older
version: the installation stays on its pre-release until a stable release
is newer. Changing the channel forgets an update found on the other
channel; it cannot change while an update is being checked, downloaded or
installed.

Each manifest lists one artifact per platform target (`darwin-aarch64`,
`darwin-x86_64`, `linux-x86_64`, `windows-x86_64`) and, on Linux, per
package type: the updater installs the same kind of package that is
installed (AppImage, `.deb` or `.rpm`; the latter two through the system
package tools, which ask for administrator authorization). Pre-releases
have no `.rpm` (RPM refuses `-` in versions); `.rpm` installations update
from stable releases only.

## Authenticity

- Every updater artifact is signed with a **minisign** key during the release
  build. The app downloads an artifact and verifies its signature against the
  public key compiled into the app (`DECKPILOT_UPDATER_PUBKEY` at build time)
  before installing. Checksums alone are not trusted.
- The signature's signed comment names the version the artifact was built
  for, and the app requires it to match the announced version
  (`requireSignedVersion`). The manifest itself is not signed; this binding
  prevents serving an older, validly signed release as a newer one. The
  release workflow checks it before a draft can be published.
- `requireSignedVersion` is enabled: the version is part of the signed data,
  so an old, genuinely signed artifact cannot be offered as a newer version
  (downgrade protection).
- Builds without a public key (development and self-built versions) report
  updates as "not configured" and never download anything.
- Before a release is published, CI verifies the generated `latest.json` and
  every artifact's signature with `deckpilot-dev verify-release`, and attaches
  `SHA256SUMS`.

The independent manifest and signature checks live in `crates/deckpilot-update`
and are covered by tests with a test-only key pair.

## Modes

| Mode                 | Behaviour                                                   |
| -------------------- | ----------------------------------------------------------- |
| Notify me (default)  | Shows that an update is available                           |
| Download, then ask   | Downloads in the background; installs after confirmation    |
| Install when idle    | Downloads and installs after 10 minutes without device input |

Automatic checks run while the configuration app is open, at a configurable
interval (6 hours to 7 days, default daily). Preferences are stored on this
machine only.

## Installing: app and agent together

The agent ships inside the application bundle, so an update replaces both
executables. The configuration app coordinates the installation:

1. download and verify the update;
2. wait while workflows run in the agent (`runningExecutions` in its
   status): automatic installation tries again later, and installing by hand
   says why it waits, so an update never ends a workflow silently. Agents
   before protocol 6 cannot report running workflows;
3. send `prepareForUpdate` to the agent. The agent finishes the request it is
   processing, blanks and
   releases the devices and exits. An agent from an older release that does
   not know this request is stopped with `shutdown` over protocol 1;
4. install. If installing fails, the previous version stays installed, the
   previous agent is started again and the error is shown;
5. restart the application, which starts the new agent.

A running executable is never overwritten. After any restart the app compares
its version with the agent's (`AgentCompatibility`) and offers to restart a
mismatching agent, so mismatched versions are not left running side by side.

Only one check, download or installation runs at a time; a second attempt
is refused while one is in progress. A failed installation is reported as a
failure, the downloaded update stays ready to retry, and the agent is
started again (it opens a system-managed configuration by itself, or is
unlocked by the app). Restarting the agent from the app is refused while
workflows run unless the user confirms interrupting them; interrupted runs
are recorded as such in the history.

## When an update fails

Every failure says what failed, what state the installation is in and what
to do, followed by the updater's own detail, for example:

- *signature verification*: the package was discarded and nothing was
  installed. It is not retried from the cache; the next attempt downloads
  it again. Never install DeckPilot from another source because of it;
- *connection stopped responding* or *GitHub Releases could not be reached*:
  nothing was installed; a connection that sends no data for 60 seconds
  (30 seconds to connect) ends the check or download instead of keeping
  the updater busy;
- *installation failed*: the previous version is kept and the downloaded
  update stays ready to retry; an agent that was running is started again,
  and if that is not possible the message says to restart DeckPilot. If the
  updater could not put the previous version back (macOS), the message names
  the path of the kept copy;
- *administrator authorization was cancelled* (Linux packages, or an app in a
  folder that needs it): nothing changed.
- on Windows the system installer takes over and the app exits before it
  runs; if the installer is cancelled or fails, the installed version stays
  (Windows Installer rolls a failed installation back; not covered by the
  tests) and DeckPilot starts as before when opened again.

The phase always returns to idle or ready after a failure, so a new attempt
is possible right away. Every check result, download, installation and
failure is also appended to `updates.log` in the log directory (next to the
agent's logs; the logs window shows the directory), with the time and the
running version, so the cause of a failed update survives the restart. The
file holds versions, URLs, paths and error texts only, never keys or
configuration; it is kept below about 256 KiB (`updates.log.1` holds the
previous part).

## First launch after an update

Every start of the configuration app is recorded (`app_launches`). With the
default system-managed key an update needs no prompt; with a password, the
password is needed after the restart anyway. Users can additionally require
an authentication once after each update in Settings → Security (see
[security.md](security.md#locking-and-authentication)). The database is never re-encrypted
by an update. Schema upgrades are preceded by a verified encrypted backup.

A newer database is never opened by an older DeckPilot (see
[data-model.md](data-model.md#compatibility-between-versions)); the updater
never installs an older version than the running one.

## Verification status

| What | How | Status |
| ---- | --- | ------ |
| Manifest parsing, channel (no pre-releases, no downgrades), trusted URLs | `deckpilot-update` unit tests | automated |
| Channels: tag classification, stable never accepts a pre-release, the prerelease channel picks the newest published release of either kind and moves on to stable, drafts and mismatched flags ignored | `deckpilot-update` channel tests; `updates_tests.rs` against a loopback release list | automated |
| Each release carries only its channel's manifest | `deckpilot-dev verify-release` tests; release workflow | automated |
| Non-manifest responses (offline captive portal, empty, truncated JSON) | unit tests | automated |
| Signature verification: valid accepted; tampered, truncated, extended, zeroed, empty package rejected; signature by another key rejected; signed version bound to the announced one | unit tests with throwaway keys | automated |
| Install decisions: modes, idle device, running workflows (`agent_busy`) | policy unit tests | automated |
| Tampered update served to an installed build is rejected; the real update installs, replaces app and agent, restarts, and opens the same encrypted configuration | `tools/release-validation/update-test.sh` (local builds, throwaway updater key, loopback endpoint) | manual run on macOS |
| Keychain access kept across an update between two Developer ID signed builds | `update-continuity` workflow (CI signing identity, no notarization, throwaway updater key) | automated on demand |
| Update flow of the app against a loopback server with throwaway keys: a correctly signed update is downloaded; tampered, truncated, empty, foreign-key, malformed-signature and older-release-as-newer packages are rejected and discarded | `updates_tests.rs` in the desktop crate (real Tauri updater) | automated |
| Interrupted download (connection closed early, stalled connection, server error): fails within the read timeout, nothing partial is kept, the update stays offered and the next attempt starts over and succeeds | `updates_tests.rs` | automated |
| Failed check (captive portal, server error, stalled connection) leaves the updater usable | `updates_tests.rs` | automated |
| Installation into a throwaway installed copy (macOS app bundle, Linux AppImage): a running agent is stopped first, app and agent are replaced together; a signed but uninstallable package leaves the previous version untouched, the update ready to retry and says how to recover the agent | `updates_tests.rs` | automated on macOS and Linux; Windows installs through the system installer and is not run in tests |
| Update between two published releases | needs two public releases | not verified |

- Update discovery, manifest validation, signature verification, policy
  decisions and the agent stop/restart sequence are covered by automated
  tests.
- **An actual signed update has not yet been installed end to end.** This
  requires a provisioned signing key and two published releases; see
  [releasing.md](releasing.md). Until it has been verified, the default mode
  stays "Notify me".
