# Application updates

DeckPilot updates itself from **GitHub Releases** of
`knittingcode/DeckPilot`. Releases contain application binaries and update
metadata only. No profile, database, token or other user data is ever
uploaded.

## Channel and discovery

The app reads
`https://github.com/knittingcode/DeckPilot/releases/latest/download/latest.json`.
GitHub's `latest` alias never points at drafts or pre-releases, so this is the
stable channel. The manifest is produced by the Tauri bundler and lists one
artifact per platform target (`darwin-aarch64`, `darwin-x86_64`,
`linux-x86_64`, `windows-x86_64`) and, on Linux, per package type: the
updater installs the same kind of package that is installed (AppImage,
`.deb` or `.rpm`; the latter two through the system package tools, which
ask for administrator authorization).

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
2. send `prepareForUpdate` to the agent. The agent finishes the request it is
   processing (actions and workflows are never interrupted), blanks and
   releases the devices and exits. An agent from an older release that does
   not know this request is stopped with `shutdown` over protocol 1;
3. install. If installing fails, the previous agent is started again and the
   error is shown;
4. restart the application, which starts the new agent.

A running executable is never overwritten. After any restart the app compares
its version with the agent's (`AgentCompatibility`) and offers to restart a
mismatching agent, so mismatched versions are not left running side by side.

Only one check, download or installation runs at a time; a second attempt
is refused while one is in progress. A failed installation is reported as a
failure, the downloaded update stays ready to retry, and the agent is
started again (it opens a system-managed configuration by itself, or is
unlocked by the app). The agent processes requests one
at a time, so a running workflow finishes before the agent stops for an
update.

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

- Update discovery, manifest validation, signature verification, policy
  decisions and the agent stop/restart sequence are covered by automated
  tests.
- **An actual signed update has not yet been installed end to end.** This
  requires a provisioned signing key and two published releases; see
  [releasing.md](releasing.md). Until it has been verified, the default mode
  stays "Notify me".
