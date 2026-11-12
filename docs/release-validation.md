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

`KEY_MODE=system` tests the default protection: the old version's agent
creates the configuration with a random key in the Keychain, a restart of app
and agent must open it from the Keychain, and after the update the new agent
must open it from the Keychain without any prompt. Keychain access is bound
to the code identity, so this mode needs both builds signed with the same
Developer ID (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`); the builds are never notarized. The *Update
continuity* workflow (`.github/workflows/update-continuity.yml`) runs it on a
macOS runner with a temporary keychain: no Apple submission, no release,
nothing public.

## 3. Stream Deck MK.2 (physical device)

`deckpilot-dev hardware-test` is the reproducible procedure: it starts its own
agent on an isolated data directory (`.dev/hardware-test`) with test profiles
whose keys are labelled `A1`…`A15` and `B1`…`B15` and log every press and
release, then checks discovery, the model, every key, images, brightness,
profile and project switching, unplugging, an agent restart and multiple
devices. Keys are pressed in the order of their labels, so a key whose image
and input index disagree (rotation or mirroring) fails the test; it also asks
where A1, A5 and A15 are and whether the keys are black once the agent
stopped, asking where only a person can judge. It refuses to start while
other programs may hold the device (quit `make up` and the Elgato app first)
and writes `report.md`. `--mock` runs it against a simulated MK.2 to check
the procedure itself (CI does this on Linux); that is not a hardware test.

The same steps by hand, with DeckPilot installed (or `make up` with
`--devices none`):

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

Automated: CI builds and tests on Windows and Linux; it builds the Linux
`.deb` and `.rpm` and the Windows NSIS and MSI installers with the release
configuration and checks their contents (bundled agent, udev rule, desktop
entry); test releases build every package and verify every updater
signature. Manual checks on real machines:

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

## Results for 0.5.0

| Area | Result | How |
| ---- | ------ | --- |
| macOS signing | Verified | Test releases `v0.5.0-1` and `v0.5.0-2` (CI): app and agent Developer ID signed, hardened runtime, no entitlements, agent identifier `io.github.knittingcode.deckpilot.agent`, strict verification — for the app on disk, in the updater archive and in the disk image, on Apple silicon and Intel |
| Notarization | Verified | Apple accepted the app and the disk image for both architectures (both test releases); tickets stapled; Gatekeeper accepts the disk image downloaded from the draft with a quarantine attribute (`source=Notarized Developer ID`) |
| Release pipeline | Verified | `v0.5.0-2`: all four platform builds, every manifest entry verified against the updater key including the signed version, `SHA256SUMS` checked, a **draft pre-release**, nothing published. `v0.5.0-1` found two blockers (Windows OpenSSL/Perl, RPM and pre-release versions), both fixed |
| Update between versions | Verified locally | `update-test.sh` 0.5.0 → 0.5.1: tampered update rejected, the installed version and agent kept running; signed update installed, app and agent restarted at 0.5.1, same encrypted configuration (3 profiles, active profile and project). Found and fixed: the app did not follow an agent unlocked elsewhere |
| Keychain after an update | Unverified | Needs two consecutive Developer ID signed releases (section 2) |
| Stream Deck MK.2 | Not tested | No device was connected; checklist in section 3 |
| Windows, Linux | Installers built and verified in CI | Manual checks in section 4 not performed |

## 5. Release security audit

| Topic | Status | Evidence |
| ----- | ------ | -------- |
| Updater signatures | Verified | Every artifact is minisign-signed; the app verifies with the compiled-in key before installing; `verify-release` checks every manifest entry in CI (test release `v0.5.0-2`) |
| Signed version (downgrade by replay) | Verified | `requireSignedVersion`: the signature's signed comment must name the announced version; checked in `verify-release`; the Tauri CLI writes it (checked on real builds) |
| Version comparison, downgrades | By design | The updater installs only newer versions (`allowDowngrades` is off); the stable channel offers no pre-releases (`releases/latest`) |
| Manifest validation | Verified | Download URLs must be DeckPilot release downloads; manifest version must equal the sources' |
| Download integrity | Verified | Signature over the downloaded artifact (a tampered artifact is rejected: update test); HTTPS from GitHub; `SHA256SUMS` and build provenance attestations on releases |
| App/agent compatibility | By design, tested | Same version required: the app detects a mismatching agent and offers to restart it with the bundled one; the update stops the agent before replacing files and starts the new one (update test) |
| SQLCipher migrations | Tested | A newer schema is refused; upgrades run after a verified encrypted backup, in one transaction (storage tests) |
| Failed updates | Tested | A rejected or failed update leaves the installed version and its agent running (update test); the downloaded update stays ready to retry |
| Release artifact integrity | Verified | Draft-only releases; checksums verified in the release workflow; attestations (`gh attestation verify`) |
| Certificate expiry | Checked | The signing check fails on an expired Developer ID certificate and warns 60 days ahead. Shipped apps keep working after expiry: signatures carry a secure timestamp and the notarization ticket is stapled |
| Updater key | Not rotated | The minisign key has no expiry; losing it means installations cannot verify updates (keep the offline backup, see [releasing.md](releasing.md)) |

