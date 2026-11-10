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

Optional, not yet configured: Apple code signing and notarization
(`APPLE_CERTIFICATE`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`,
`APPLE_TEAM_ID`) and Windows Authenticode signing. Without them, macOS
Gatekeeper and Windows SmartScreen warn on first start. The updater signature
is independent of these platform signatures.

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
   - checks that the tag matches the versions and that signing is configured;
   - builds the agent and the app for Linux (x86_64), macOS (Apple silicon
     and Intel) and Windows (x86_64), bundling the agent as an external
     binary;
   - signs updater artifacts and uploads everything plus `latest.json` to
     the draft release;
   - attests build provenance for every artifact
     (`gh attestation verify <file> --repo knittingcode/DeckPilot`);
   - downloads the draft, verifies the manifest and every signature with
     `deckpilot-dev verify-release`, and attaches `SHA256SUMS`.
4. Review the draft, edit the release notes (shown in the app) and publish.

Publishing the draft is the explicit approval step; the workflow never
publishes by itself.

## Status

The signing key pair is provisioned in the repository settings, and the
public key variable is validated. The release workflow has not run yet: no
tag has been pushed from this repository. The first release should be treated as a test of the
pipeline, and the first update between two published releases as the test of
installation (see [updates.md](updates.md)).
