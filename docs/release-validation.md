# Release validation

How a release candidate is validated before a release is published. Each
part says what is automated, what needs a person, and what the evidence is.
Nothing here publishes a release or touches the release signing keys.

## 1. Signed test release (CI)

A test tag runs the real release workflow: the same build, signing and
notarization as a final release, ending in a **draft** release.

1. Set the version to the test version, e.g. `0.5.0-1` (Windows installers
   accept only numeric pre-release identifiers), and push the commit. RPM
   does not accept `-` in versions, so test releases build `.deb` and the
   AppImage on Linux; final versions build all packages.
2. Tag and push: `git tag v0.5.0-1 && git push origin v0.5.0-1`.
3. The workflow refuses to continue if versions, release notes (a test
   version uses the notes of `0.5.0`), updater or Apple signing are not in
   order. A tag with a pre-release suffix becomes a pre-release, so even if
   it were published it would never become the release the app updates to
   (the app only reads `releases/latest`).

Evidence to collect from the run and the draft:

- macOS jobs: the bundler's notarization log (`Finished with status
  Accepted`), the disk image's notarization (`notarize.sh`), and
  `verify-macos-app.sh` for the app, the updater archive and the disk image
  (strict signature, team, hardened runtime, no entitlements, agent
  identifier `io.github.knittingcode.deckpilot.agent`, `spctl`, `stapler`).
- `Verify artifacts and publish checksums`: every artifact verified against
  `DECKPILOT_UPDATER_PUBKEY`, including the signed version.
- Download the disk image from the draft on a Mac and check it like a user
  would receive it: `spctl --assess --type open --context
  context:primary-signature -v <dmg>`, `xcrun stapler validate <dmg>`, then
  open it and start the app (Gatekeeper must not warn).

Delete the draft release and the tag afterwards, or keep them as a record;
a draft is never visible to users.

## 2. Update between two versions (local, reproducible)

`tools/release-validation/update-test.sh <work-dir>` (macOS) builds two
versions from the current commit (default `0.5.0` → `0.5.1`), signs their
updater artifacts with a throwaway key and compiles them with a loopback
update endpoint (`DECKPILOT_UPDATE_ENDPOINT`, build time only, `http://127.0.0.1`
only). It then:

1. serves a **tampered** update: it must be rejected; the installed version
   and its agent keep running;
2. serves the real update: the installed app finds it, downloads it, verifies
   its signature (including the signed version), stops the agent, replaces
   itself, restarts, starts the new agent, and opens the same encrypted
   configuration (profiles, active profile and project are compared).

The configuration is password protected (the development fixture password)
and isolated in `<work-dir>`; the script unlocks the agent as a user would by
typing the password (`deckpilot-dev validation unlock`). The two builds take
a while (release profile). Windows of the test app appear on screen.

What this test cannot show: **Keychain access after an update**. Local
builds are not Developer ID signed, so with a system-managed key macOS would
ask for the login keychain password after the update (expected; see
[security.md](security.md)). Showing that a Developer ID signed update keeps
the Keychain access needs two consecutive Developer ID signed builds where
the older one can reach the newer one's manifest. The app only reads
`releases/latest`, so this happens at the latest with the first update
between two published releases: install release N, let it create its
configuration with the system key, publish N+1, and confirm that DeckPilot
opens after the update without any Keychain prompt. Until then it is
**unverified**; the stable agent identity it depends on is checked in CI and
in every release build.

## 3. Stream Deck MK.2 (physical device)

Needs a person and a device. With DeckPilot installed (or `make up` with
`--devices none`), in order:

| # | Step | Expected |
| - | ---- | -------- |
| 1 | Connect the MK.2 by USB (Linux: udev rule installed) | Connected Devices lists it with its serial; the tray shows it |
| 2 | Activate a profile with an MK.2 layout | Key images appear within a second; the active page is shown |
| 3 | Press and release a key bound to an action | The action runs once; a workflow summary appears for failures |
| 4 | Switch profiles (app and tray) | Layout and images change; no restart |
| 5 | Switch projects (key bound to `projects.next`) | The active project changes and persists after a restart |
| 6 | Change brightness in the app | The device follows immediately and keeps it after reconnecting |
| 7 | Unplug, wait, plug in again | "Disconnected", then the same layout and page again within ~2 s |
| 8 | Restart the agent (tray → Quit, start DeckPilot) | Images return; profile, project and brightness are restored |
| 9 | Close the configuration app | Keys keep working; the agent keeps running |
| 10 | Lock DeckPilot (Settings → Security → Lock now) | Keys show "Locked" and run nothing; unlocking restores them |
| 11 | Sleep the computer with auto-lock "when the computer sleeps" | After waking, the device shows "Locked" |

Record the DeckPilot version, operating system, MK.2 firmware/serial and the
result of each step. Hardware has **not** been tested by the project yet.

## 4. Windows and Linux

Automated today: CI builds and tests on Windows and Linux, and builds the
Linux `.deb` with the release configuration (bundled agent, udev rule,
desktop entry). Manual checks on real machines:

| Platform | Check |
| -------- | ----- |
| Windows | NSIS and MSI install DeckPilot and `deckpilot-agent.exe` side by side; SmartScreen warns (not Authenticode signed) |
| Windows | First start creates the configuration with the key in Credential Manager (`DeckPilot` entries, local persistence); restart and sign-in open it without a prompt |
| Windows | The agent starts at sign-in (autostart registry entry) and the tray icon appears |
| Windows | An update stops the agent before the installer replaces the executables, and DeckPilot starts again afterwards |
| Linux | `.deb`/`.rpm` install the app, the agent and `/usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules`; the device works without root after replugging |
| Linux | With GNOME Keyring or KWallet: the key is stored in the login collection; without a Secret Service, setup explains it and offers a password |
| Linux | The agent starts at login (XDG autostart); the tray icon needs AppIndicator support |
| Linux | AppImage updates in place; `.deb`/`.rpm` updates ask for administrator authorization |
