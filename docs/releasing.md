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

### macOS: Developer ID signing and notarization (not yet provisioned)

Required for a macOS release. The workflow **refuses to build macOS
releases** without it, unless the repository variable
`DECKPILOT_ALLOW_UNSIGNED_MACOS` is set to `true` as a deliberate decision.

Why it matters beyond Gatekeeper: the background agent keeps the database
key in the Keychain, and the Keychain grants access to the agent's code
identity. An unsigned (or ad-hoc signed) agent gets a new identity with
every update, so macOS asks for the login keychain password after each
update (verified; see [security.md](security.md#system-key)). A Developer ID
signature keeps the identity (identifier and team) stable across releases.

Needed, as repository secrets (from an Apple Developer Program membership):

| Secret | Content |
| ------ | ------- |
| `APPLE_CERTIFICATE` | the *Developer ID Application* certificate with its private key, exported as `.p12` and base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | the Apple ID used for notarization |
| `APPLE_PASSWORD` | an app-specific password of that Apple ID |
| `APPLE_TEAM_ID` | the team identifier |

With them, the Tauri bundler signs the application and the bundled agent
with the hardened runtime (`bundle.macOS.hardenedRuntime`), notarizes and
staples the app. The workflow then verifies the signature
(`codesign --verify --deep --strict`), that the agent is signed by the team
with the hardened runtime, Gatekeeper's assessment (`spctl`) and the stapled
ticket (`stapler validate`). None of this has run yet: no Apple credentials
are configured, so notarization is **not verified**.

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
   - signs updater artifacts and uploads everything plus `latest.json` to
     the draft release;
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

The updater signing key pair is provisioned in the repository settings, and
the *Signing check* workflow verifies that the secrets and the public key
belong together. A local release build signed with a throwaway key was
checked with the same verification the workflow uses. Apple and Windows code
signing are not provisioned. The release workflow has not run yet: no tag
has been pushed from this repository. The first release should be treated as a test of the
pipeline, and the first update between two published releases as the test of
installation (see [updates.md](updates.md)).
