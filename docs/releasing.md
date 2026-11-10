# Releasing

Releases are built by `.github/workflows/release.yml` when a tag `v*` is
pushed (or when the workflow is started manually for an existing tag). The
workflow creates a **draft** release; nothing reaches users until a
maintainer reviews and publishes it.

## One-time provisioning

The repository does not contain any signing key. A maintainer creates the
updater key pair locally and stores it only in GitHub's encrypted settings:

```sh
pnpm --filter @deckpilot/desktop tauri signer generate -w ~/deckpilot-updater.key
```

1. Store the private key file's content as the repository secret
   `TAURI_SIGNING_PRIVATE_KEY` and its password as
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
2. Store the public key (`~/deckpilot-updater.key.pub`) as the repository
   **variable** `DECKPILOT_UPDATER_PUBKEY`. It is compiled into release builds.
3. Keep an offline backup of the private key. Losing it means existing
   installations can no longer verify updates; publishing it lets anyone sign
   updates.

Optional, not yet configured: Apple code signing and notarization
(`APPLE_CERTIFICATE`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`,
`APPLE_TEAM_ID`) and Windows Authenticode signing. Without them, macOS
Gatekeeper and Windows SmartScreen warn on first start. The updater signature
is independent of these platform signatures.

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
   - downloads the draft, verifies the manifest and every signature with
     `deckpilot-dev verify-release`, and attaches `SHA256SUMS`.
4. Review the draft, edit the release notes (shown in the app) and publish.

## Status

The workflow has been written and its configuration checked, but it has not
run yet: no signing key has been provisioned and no tag has been pushed from
this repository. The first release should be treated as a test of the
pipeline, and the first update between two published releases as the test of
installation (see [updates.md](updates.md)).
