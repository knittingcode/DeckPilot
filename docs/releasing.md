# Releasing

Releases are built by `.github/workflows/release.yml` when a tag `v*` is
pushed (or when the workflow is started manually for an existing tag). The
workflow creates a **draft** release; nothing reaches users until a
maintainer reviews and publishes it.

## One-time provisioning

The repository does not contain any signing key. A maintainer creates the
updater key pair locally and stores it only in GitHub's encrypted settings:

```sh
mkdir -p ~/.config/deckpilot/signing && chmod 700 ~/.config/deckpilot/signing
pnpm --filter @deckpilot/desktop tauri signer generate -w ~/.config/deckpilot/signing/updater.key
```

1. Store the key and its password as repository secrets without echoing
   them (`gh` prompts for the password and stores it without a trailing
   newline):

   ```sh
   gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.config/deckpilot/signing/updater.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
   ```
2. Store the public key as the repository **variable**
   `DECKPILOT_UPDATER_PUBKEY`; it is compiled into release builds:

   ```sh
   gh variable set DECKPILOT_UPDATER_PUBKEY < ~/.config/deckpilot/signing/updater.key.pub
   ```
3. Keep an offline backup of the private key. Losing it means existing
   installations can no longer verify updates; publishing it lets anyone sign
   updates.

### macOS: Developer ID signing and notarization

Required for a macOS release. The workflow **refuses to build macOS
releases** without it, unless the repository variable
`DECKPILOT_ALLOW_UNSIGNED_MACOS` is set to `true` as a deliberate decision.

Why it matters beyond Gatekeeper: the background agent keeps the database
key in the Keychain, and the Keychain grants access to the agent's code
identity. An unsigned (or ad-hoc signed) agent gets a new identity with
every update, so macOS asks for the login keychain password after each
update (verified; see [security.md](security.md#system-key)). A Developer ID
signature keeps the identity stable across releases — together with the
agent's fixed signing identifier, `io.github.knittingcode.deckpilot.agent`,
which comes from an `Info.plist` embedded in the agent executable
(`apps/agent/Info.plist`; never change it). Without it, `codesign` would
derive the identifier from the build's UUID and change it in every release.

Credentials (Apple Developer Program; notarization with an App Store Connect
API key):

| Name | Kind | Content |
| ---- | ---- | ------- |
| `APPLE_CERTIFICATE` | secret | the *Developer ID Application* certificate with its private key, exported as `.p12`, base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | secret | the `.p12` password |
| `APPLE_API_KEY` | secret | the App Store Connect API **private key** (`AuthKey_….p8` content, or its base64 encoding) |
| `APPLE_SIGNING_IDENTITY` | variable | `Developer ID Application: Name (TEAMID)` |
| `APPLE_TEAM_ID` | variable | the team identifier |
| `APPLE_API_KEY_ID` | variable | the API key's ID |
| `APPLE_API_ISSUER` | variable | the API key's issuer ID |

(The Tauri bundler's own `APPLE_API_KEY` environment variable means the key
*ID*; the workflow writes the private key to a file and passes it as
`APPLE_API_KEY_PATH`, with `APPLE_API_KEY_ID` under the bundler's name.)

What the release workflow does on macOS:

1. The Tauri bundler imports the certificate into a temporary keychain and
   signs inside out: the bundled agent, the app executable and the app
   bundle, each with the hardened runtime (`bundle.macOS.hardenedRuntime`)
   and **no entitlements** — DeckPilot needs none (USB HID access through
   IOKit, the login Keychain, Touch ID through LocalAuthentication and the
   system WebKit all work under the hardened runtime without them). The app
   uses the system WebKit, so there are no embedded frameworks.
2. The bundler submits the app to Apple's notary service, **waits for the
   result**, fails the build unless it is *Accepted* (printing Apple's log),
   and staples the ticket to the app. The updater archive (`.app.tar.gz`) is
   made from the stapled app and signed with the updater key as before.
3. The workflow notarizes the disk image the same way
   (`.github/scripts/notarize.sh`: `notarytool submit --wait`, anything but
   *Accepted* fails), staples it and replaces the uploaded image in the
   draft release.
4. `.github/scripts/verify-macos-app.sh` checks the app on disk, the app in
   the updater archive and the app in the disk image: a strict signature over
   the bundle, every executable signed by `APPLE_TEAM_ID` with the hardened
   runtime and without entitlements, the agent's fixed identifier,
   Gatekeeper's assessment (`spctl`) and the stapled ticket
   (`stapler validate`). If the bundler skipped notarization (for example
   because of incomplete credentials), this step fails the release.

The *Signing check* workflow checks the credentials without a release: the
certificate provides exactly the configured identity of the configured team,
a throwaway executable signs with it (hardened runtime, secure timestamp),
and Apple's notary service accepts the API key. A manual run with
`notarize: true` (`gh workflow run "Signing check" -f notarize=true`) also
submits that throwaway executable for notarization; this records a
submission in the developer account.

### Windows: Authenticode (not yet provisioned)

Installers are not Authenticode-signed; SmartScreen warns on first start.
Signing needs a code-signing certificate (or a cloud signing service) and
`bundle.windows.signCommand` or `certificateThumbprint` in the Tauri
configuration. The updater signature does not depend on it. Windows has no
per-application access list on the Credential Manager, so unsigned updates
do not affect the system-managed database key.

The updater signature is independent of all platform signatures.

## Checking the signing configuration

Without building or publishing anything:

- **In CI:** the *Signing check* workflow (`.github/workflows/signing-check.yml`)
  runs on pull requests that touch release code and on demand
  (`gh workflow run "Signing check"`). It signs a throwaway file with the
  secrets and verifies the signature against the variable with the same
  code the app uses. A failure such as *Wrong password for that key* means
  the two secrets do not belong together.
- **Locally:** sign a throwaway file with your key (the password is read
  without echo and never appears in the command line) and verify it against
  the public key:

  ```sh
  printf 'test\n' > /tmp/deckpilot-signing-test.bin
  read -rs "TAURI_SIGNING_PRIVATE_KEY_PASSWORD?Password: " && export TAURI_SIGNING_PRIVATE_KEY_PASSWORD; echo
  pnpm --filter @deckpilot/desktop tauri signer sign -f ~/.config/deckpilot/signing/updater.key /tmp/deckpilot-signing-test.bin
  unset TAURI_SIGNING_PRIVATE_KEY_PASSWORD
  DECKPILOT_UPDATER_PUBKEY="$(cat ~/.config/deckpilot/signing/updater.key.pub)" \
    cargo run -p deckpilot-dev -- verify-signature /tmp/deckpilot-signing-test.bin /tmp/deckpilot-signing-test.bin.sig
  rm /tmp/deckpilot-signing-test.bin /tmp/deckpilot-signing-test.bin.sig
  ```

  (`read -rs "VAR?prompt"` is zsh; in bash use `read -rsp "Password: " VAR`.)

## Release steps

1. Update the version in `Cargo.toml` (`[workspace.package]`),
   `apps/desktop/package.json` and `apps/desktop/src-tauri/tauri.conf.json`.
   `cargo run -p deckpilot-dev -- check-versions` confirms they match; CI
   checks it too.
2. Merge to `main`, then tag: `git tag v0.3.0 && git push origin v0.3.0`.
3. The workflow:
   - checks that the tag matches the versions, that updater signing is
     configured, that `CHANGELOG.md` has notes for the version (they become
     the release notes and the notes shown by the in-app updater), and that
     macOS signing is configured (or explicitly waived);
   - builds the agent and the app for Linux (x86_64), macOS (Apple silicon
     and Intel) and Windows (x86_64), bundling the agent as an external
     binary;
   - signs updater artifacts and uploads everything to the draft release;
     each build also hands its `latest.json` entries (target keys, asset
     names, signatures) to a single job that assembles `latest.json` once,
     after all builds, and attaches it: concurrent merges by the builds
     could lose entries. It fails if a platform or package type is missing,
     if two builds claim the same entry or if an asset is not in the draft
     (`tools/ci/assemble-latest-json.sh`, tested on every pull request with
     fake signed packages by `tools/ci/test-latest-json.sh`);
   - checks that the built app embeds the configured updater public key;
   - on macOS, verifies code signature, agent identity and notarization when
     signing is configured;
   - attests build provenance for every artifact
     (`gh attestation verify <file> --repo knittingcode/DeckPilot`);
   - downloads the draft, verifies the manifest and every signature with
     `deckpilot-dev verify-release` — including that each signature names
     the announced version, which installations require
     (`requireSignedVersion`, the protection against downgrade attacks) —
     and attaches `SHA256SUMS`.
4. Review the draft and publish it. Missing signing secrets, a missing
   changelog section or a failed verification stop the workflow before
   anything could be published.

Publishing the draft is the explicit approval step; the workflow never
publishes by itself.

## Status

The updater signing key pair and the Apple Developer ID and notarization
credentials are provisioned in the repository settings; the *Signing check*
workflow verifies them on every change to release code. A local release
build signed with a throwaway updater key was checked with the same
verification the workflow uses. Windows code signing is not provisioned. The
release workflow has not run yet: no tag has been pushed from this
repository, so signing and notarization of the real application bundle and
disk image have **not** been exercised end to end. The first release should be treated as a test of the
pipeline, and the first update between two published releases as the test of
installation (see [updates.md](updates.md)).
