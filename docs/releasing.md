# Releasing

Releases are built by `.github/workflows/release.yml` when a release tag
is pushed (or when the workflow is started manually for an existing tag).
The workflow creates a **draft** release; nothing reaches users until a
maintainer reviews and publishes it.

## Release tags and the tag gate

Only these tags build a release; the trigger's `v*` is just a preliminary
filter:

| Tag | Example | Kind | GitHub release |
| --- | ------- | ---- | -------------- |
| `vX.Y.Z` | `v0.6.0` | stable | release |
| `vX.Y.Z-alpha.N` | `v0.6.0-alpha.1` | pre-release | pre-release |
| `vX.Y.Z-beta.N` | `v0.6.0-beta.2` | pre-release | pre-release |
| `vX.Y.Z-rc.N` | `v0.6.0-rc.1` | pre-release | pre-release |

Exactly the expression
`^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$`:
no leading zeros, `N` from 1, no build metadata, nothing normalized.
`testv0.6.0`, `v0.6.0-bettta.1`, `v0.6`, `v01.0.0`, `v1.0.0-beta.0`,
`v1.0.0-preview.1` and `v1.0.0-extra` are all refused.

The first job of the workflow, **Validate the release tag** (`validate-tag`,
`tools/release-pipeline/validate-tag.sh`), is the authoritative gate. With
read-only permissions (`contents: read`, `checks: read`) it checks, without
changing anything:

1. the tag against the expression above;
2. the commit the tag points to (annotated tags are dereferenced with
   `git rev-parse "$TAG^{commit}"`) is the checked-out, unmodified commit
   and is reachable from the protected `main` branch: a tag on a side
   branch or on an unmerged commit is refused;
3. every version declaration matches the tag:
   `deckpilot-dev check-versions --tag <tag>` (the workspace version in
   `Cargo.toml`, which the agent and every crate inherit, the workspace
   packages in `Cargo.lock`, `apps/desktop/package.json`,
   `apps/desktop/src-tauri/tauri.conf.json` — the version the updater
   artifacts are signed for — and a `CHANGELOG.md` section);
4. the required checks passed on **exactly** that SHA, using the checks API:
   for every required check name, the latest check run of that commit
   (highest ID) must have concluded `success`; a missing, running,
   cancelled, skipped or failed check refuses the tag, and runs of other
   commits never count. The required checks are the pinned list in
   `tools/release-pipeline/required-checks.txt` — `Frontend`, `Storage with
   SQLCipher`, `Rust (ubuntu-latest)`, `Rust (macos-latest)`,
   `Rust (windows-latest)`, `Updater signing keys match`, the required
   status checks of the *Protect main* ruleset — plus whatever the branch
   rules of `main` require when the release runs (read with
   `GET /repos/{owner}/{repo}/rules/branches/main`; if that cannot be read,
   the pinned list alone applies, with a warning). The *Signing check*
   workflow also runs on every push to `main` so that `Updater signing keys
   match` exists for the squash-merge commit that is tagged;
5. classification: `alpha`, `beta` and `rc` are pre-releases, a plain
   version is stable (`deckpilot-dev release-tag <tag>` must agree).

Its outputs `version`, `channel` (`stable` or `prerelease`), `sha`,
`prerelease` (`true`/`false`) and `msi_version` drive the rest of the
workflow. `msi_version` exists because Windows Installer accepts only
numeric versions (`0.6.0-alpha.1` fails the MSI build): alpha.N becomes
`X.Y.Z.N`, beta.N `X.Y.Z.(20000+N)`, rc.N `X.Y.Z.(40000+N)`, a stable
release `X.Y.Z`. Windows Installer compares only the first three fields
and DeckPilot's installer allows same-version upgrades, so the stable
release installs over its pre-releases. A version an MSI cannot carry
(major or minor above 255, patch above 65535, N above 19999) is refused by
the gate. Every later job builds `sha`, not whatever the tag points to by
then. Every other
job depends on `validate-tag` through `needs` (directly or transitively);
`tools/release-pipeline/check-workflow-gate.py` fails the *Release pipeline
tests* workflow if a job is added without it, if the gate gets write
permissions or if the workflow-level permissions grant write access. Only
the jobs that write to the draft release (build, manifest, checksums) have
`contents: write`.

The tests (`tools/release-pipeline/test.sh`, run by
`.github/workflows/release-tests.yml` on pull requests that touch the
pipeline; not a required check) use a synthetic repository and mocked API
answers (`tools/release-pipeline/gh-stub.sh`); no real tag is created.

### The former `vX.Y.Z-N` test tags

`v0.5.0-1` and `v0.5.0-2` were signed draft test builds. Tags of that form
**no longer start a release**: the gate refuses them. The two existing
drafts stay as they are (the compatibility workflow still uses the
`v0.5.0-2` binaries). To test the pipeline end to end, push an `alpha` tag
of a commit on `main`: it builds the same signed, notarized draft, which is
never published unless a maintainer publishes it. To build installers
without a release, use the packaging workflow instead.

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
  runs on every pull request, on every push to `main` and on demand
  (`gh workflow run "Signing check"`). *Updater signing keys match*
  decrypts the secret key with its password and checks that its key ID and
  public key equal `DECKPILOT_UPDATER_PUBKEY`
  (`.github/scripts/updater-key-pair.py`, Python standard library only;
  tested with throwaway keys by `tools/release-pipeline/test-updater-key-pair.sh`).
  *wrong password or key* means the two secrets do not belong together.
  While a secret is in the job, nothing else runs: the jobs check out only
  `.github/scripts`, restore no cache and install no dependencies, so no
  package installation or build script can see a secret. Pull requests
  from forks and from Dependabot get no secrets; the jobs are skipped for
  them, and a skipped required check counts as passed.
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
2. Merge to `main`, wait until the required checks have passed on the merge
   commit, then tag that commit:
   `git tag -s v0.6.0-alpha.1 <sha> -m "DeckPilot 0.6.0-alpha.1" && git push origin v0.6.0-alpha.1`.
3. The workflow:
   - validates the tag (see [the tag gate](#release-tags-and-the-tag-gate));
   - checks that updater signing is
     configured, that `CHANGELOG.md` has notes for the version (they become
     the release notes and the notes shown by the in-app updater), and that
     macOS signing is configured (or explicitly waived);
   - builds the agent and the app for Linux (x86_64), macOS (Apple silicon
     and Intel) and Windows (x86_64), bundling the agent as an external
     binary;
   - signs updater artifacts and uploads everything to the draft release;
     each build also hands its update-manifest entries (target keys, asset
     names, signatures) to a single job that assembles the manifest once,
     after all builds, and attaches it — `latest.json` for a stable
     release, `prerelease.json` for a pre-release, never both (the update
     channels, see [updates.md](updates.md#channels-and-discovery)): concurrent merges by the builds
     could lose entries. It fails if a platform or package type is missing,
     if two builds claim the same entry or if an asset is not in the draft
     (`tools/ci/assemble-latest-json.sh`, tested on every pull request with
     fake signed packages by `tools/ci/test-latest-json.sh`);
   - checks that the built app embeds the configured updater public key;
   - on macOS, verifies code signature, agent identity and notarization when
     signing is configured. The Developer ID certificate is put into the
     environment only right before the bundler runs and removed after it,
     so dependency installation scripts and the agent build never see it;
   - attests build provenance for every artifact
     (`gh attestation verify <file> --repo knittingcode/DeckPilot`);
   - downloads the draft, verifies the manifest and every signature with
     `deckpilot-dev verify-release` — including that each signature names
     the announced version, which installations require
     (`requireSignedVersion`, the protection against downgrade attacks) —
     refuses the other channel's manifest, checks that the manifest has an
     entry for every platform and package
     type (macOS app on both architectures, Linux AppImage, `.deb` and, for
     final releases, `.rpm`, Windows MSI and NSIS), and attaches
     `SHA256SUMS`.
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
