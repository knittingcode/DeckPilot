# Security review request: first-party publisher signature

Status: **requested, not done.** Until a reviewer independent of the
author signs off below, the privileged-kinds gate stays closed and no
production publisher key is pinned.

## Scope

The verification of signed first-party release artifacts
([publisher-signature.md](publisher-signature.md)) and the release
pipeline of `knittingcode/deckpilot-plugins` that produces them.

| Area | Files |
| --- | --- |
| Verifier | `crates/deckpilot-plugin-registry/src/publisher.rs`, `src/sig.rs` (reused) |
| Signing and CLI | `crates/deckpilot-plugin-registry/src/publish/release.rs`, `src/publish/keys.rs` (`--role publisher`, `from_text`), `src/publish/cli.rs` (`release sign`, `release verify`) |
| Pinned keys and lock | `crates/deckpilot-plugin-registry/publisher/first-party.json`, `plugins/first-party/artifacts.json` |
| Tests | `crates/deckpilot-plugin-registry/tests/publisher.rs`, the unit tests in `publisher.rs` |
| Plugin repository | `official/`, `tools/official.sh`, `tools/check-official.py`, `.github/workflows/official.yml`, `official-release.yml` |

## What the reviewer must check

1. **Trust anchor.** Keys come only from the compiled-in
   `publisher/first-party.json`; nothing in an artifact, manifest, id,
   URL, repository name or environment selects or adds a key. `--keys`
   is a tool option only, and the output says it is not the pinned keys.
   Is it acceptable for that option to exist in a release-built CLI?
2. **Signature binding.** The trusted comment binds id, version, sha256
   and size; minisign's global signature covers it; `sig::trusted_comment`
   refuses anything after the four signature lines. Look for replay of a
   valid signature onto another version, plugin or file, and for
   confusion with registry (`deckpilot-registry v1 role:`) or updater
   signatures, in both directions.
3. **Order of checks.** The claim's size and sha256 are checked before any
   parsing; the archive is parsed only after a valid signature. Confirm
   no untrusted byte reaches the tar or gzip reader first.
4. **Revocation.** `revokedKeys` beats any other valid signature;
   revoked releases in the lock are refused; a lock cannot both lock and
   revoke a release. Is build-time revocation (no run-time list) enough
   for artifacts that are re-pinned in the agent anyway?
5. **Rollback.** The floor is the highest version in
   `released-pins.json` for the target platform; equal versions need equal
   files. Check the "other platforms do not count" rule and what happens
   when a platform has no recorded pins.
6. **Authenticity vs. authorization.** Confirm that nothing derives a
   grant, tier or the bundled source from a signature: a verified
   artifact becomes a bundled plugin only through a reviewed
   `artifacts.json` change and the pins compiled into the agent, and the
   registry and sideload paths (X12, `is_bundled_id`) are untouched.
7. **The gate.** `check_gate` refuses every 1.9 privileged kind
   (`IssueCode::FirstPartyOnly`), process execution (`tools`) included.
   Decide whether the gate may open, and under which additional
   conditions (for example a threshold of 2, a hardware-backed key, or
   reproducible-build attestation for Git and Docker).
8. **Key handling.** The secret key is created offline by the owner,
   stored only as an environment secret in deckpilot-plugins, passed to
   `release sign --key-env` (never written to disk), and never available
   to pull-request workflows. Review the environment protection rules
   and that no workflow in `knittingcode/deckpilot` can sign
   (`check-release-separation.py`).
9. **Release pipeline.** `official.yml` runs untrusted pull-request code
   with no secret and throwaway keys only; `official-release.yml` runs
   only on a release tag, in the protected environment, builds from the
   tagged commit with the pinned toolchain and `Cargo.lock`, and refuses
   to sign what the verifier would refuse (`release sign` runs the
   verifier first).
10. **Reproducibility.** A release must be rebuildable bit for bit from
    the tag (two clean builds compared in CI). Decide whether the
    application build should rebuild and compare instead of, or in
    addition to, trusting the signature.

## Out of scope

The registry roles (U-10, U-12), the plugin host sandbox, the 1.9
broker and denial matrix (reviewed in
[module-migration.md §7](module-migration.md#plugin-api-19-security-reviews-2026-11-20)).

## Sign-off

| Reviewer | Date | Outcome | Gate |
| --- | --- | --- | --- |
| | | | closed |
