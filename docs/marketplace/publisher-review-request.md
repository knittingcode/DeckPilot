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
| Agent (host side) | `apps/agent/src/plugins/official.rs`, its tests, and the calls in `service.rs`, `registry.rs`, `fetch.rs` (checklist below) |
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
4. **Threshold and signature files.** One file per signing key,
   `<package>.<KEYID>.minisig` (`signature_file_name`, `key_id_in_file_name`,
   `is_key_id`; `KEYID` as minisign prints it: little-endian key number,
   16 uppercase hex). In `check_signatures` / `check_file` confirm that a
   file counts only for the one pinned current key whose id is both in its
   name and inside its signature, at most once per key; that duplicates
   under another name, two files of one key, unknown keys, mismatched
   names, wrong comments, malformed and oversized files never count; that
   fewer than `threshold` distinct keys refuses (no partial or
   warning-only outcome); that `validate` refuses a threshold of 0 or
   above the number of current keys and a key id naming two keys; and that
   nothing at run time lowers the threshold. `release sign` uses one key
   per invocation; check that the release workflow signs A and B in
   separate jobs (no job holds both secrets) and that the combining job
   runs `release verify` against the pinned keys.
5. **Revocation.** `revokedKeys` beats any other valid signature;
   revoked releases in the lock are refused; a lock cannot both lock and
   revoke a release. Is build-time revocation (no run-time list) enough
   for artifacts that are re-pinned in the agent anyway?
6. **Rollback.** The floor is the highest version in
   `released-pins.json` for the target platform; equal versions need equal
   files. Check the "other platforms do not count" rule and what happens
   when a platform has no recorded pins.
7. **Authenticity vs. authorization.** Confirm that nothing derives a
   grant, tier or the bundled source from a signature: a verified
   artifact becomes a bundled plugin only through a reviewed
   `artifacts.json` change and the pins compiled into the agent, and the
   registry and sideload paths (X12, `is_bundled_id`) are untouched.
8. **The gate.** `check_gate` refuses every 1.9 privileged kind
   (`IssueCode::FirstPartyOnly`), process execution (`tools`) included.
   Decide whether the gate may open, and under which additional
   conditions (the opening path already needs at least 2 distinct keys
   that actually signed; for example also a hardware-backed key, or
   reproducible-build attestation for Git and Docker).
9. **Key handling.** The two secret keys are created offline by the
   owner, stored only as secrets of the `official-release` environment of
   deckpilot-plugins (`DECKPILOT_PUBLISHER_KEY_A`,
   `DECKPILOT_PUBLISHER_KEY_A_PASSWORD`, `DECKPILOT_PUBLISHER_KEY_B`,
   `DECKPILOT_PUBLISHER_KEY_B_PASSWORD`), each passed to its own
   `release sign --key-env … --password-env …` job (never written to
   disk), and never available to pull-request workflows. Review the environment protection rules
   and that no workflow in `knittingcode/deckpilot` can sign
   (`check-release-separation.py`).
10. **Release pipeline.** `official.yml` runs untrusted pull-request code
   with no secret and throwaway keys only; `official-release.yml` runs
   only on a release tag, in the protected environment, builds from the
   tagged commit with the pinned toolchain and `Cargo.lock`, and refuses
   to sign what the verifier would refuse (`release sign` runs the
   verifier first).
11. **Reproducibility.** A release must be rebuildable bit for bit from
    the tag (two clean builds compared in CI). Decide whether the
    application build should rebuild and compare instead of, or in
    addition to, trusting the signature.

## Host side: the agent's install and update path

Scope: `apps/agent/src/plugins/official.rs` and `official/tests.rs`; the
calls in `apps/agent/src/plugins/service.rs` (`pins`, `fetch`, `update`,
`apply_verdicts`, `open`); `Registry::signature` (`registry.rs`) and
`HttpsSource::signature` (`fetch.rs`); the end-to-end test
`first_party_plugins_need_the_root_authorization_and_a_verified_publisher`
(`apps/agent/tests/plugins_registry_https.rs`). Design:
[publisher-signature.md](publisher-signature.md#the-agent-installing-and-updating-official-plugins).
The reviewer must not be the author; check each item against the code,
not this list, and record failures as findings.

A. **Every `deckpilot.*` registry download is publisher-verified.**
   1. `is_first_party` gates `Official::verify` in `PluginService::fetch`,
      and every path that installs or updates from a registry goes through
      `fetch` (`install_steps`, `update`, `dependencies.rs`
      `verify_downloads`). Search for other callers of
      `Registry::package`, `manager.install`, `manager.update` with a
      registry source.
   2. Without provisioned keys, a valid lock and readable floors,
      `pins` refuses before any byte is downloaded (`Official::ready`).
   3. File and dev installs of `deckpilot.*` stay refused
      (`InstallSource::allows`, `install_steps`).
B. **The claim and the trust anchor.** The claim is the registry's
   verified pin (`target.expected`), never the package's own manifest; the
   keys are `PublisherKeys::pinned()` except in `test-hooks` builds
   (`TEST_KEYS_ENV`). Confirm no release build enables `test-hooks`
   (agent `Cargo.toml`, `release.yml`, `deckpilot-dev package`), and that
   nothing in an artifact, URL, registry document or setting selects a key.
C. **Order and bounds.** `PluginService::official_signatures` downloads
   exactly one file per pinned current key id
   (`Official::signature_key_ids`, never a revoked key's), each capped at
   `MAX_SIGNATURE_BYTES`, from the package's base URL; missing files are
   tolerated only while the threshold can still be met, otherwise the
   install is refused before verification and audited; `verify_release` checks size and sha256 before parsing,
   signatures before the archive; `verify_package` then re-checks against
   the registry's manifest hash and permissions; the store receives only
   that `VerifiedPackage`.
D. **Rollback floor.** `effective_floor` takes the highest of the
   recorded floor, the compiled bundled pin and the compiled
   `released-pins.json` floor for this platform; equal versions with other
   files are refused; `Compiled::check_locked` adds the artifact lock's
   minimum; `record_release` persists before the store acts and never
   lowers a floor; a corrupt or unknown-version `floors.json` refuses everything
   and is never rewritten. Decide whether a local file (outside the
   encrypted `plugins.db`) is an acceptable home: deleting it resets only
   the recorded floors, the bundled pins still apply.
E. **Revocation at run time.** `revoked_installed` covers registry copies
   of `deckpilot.*` only, by compiled lock and by revoked signer of the
   recorded release; `apply_verdicts` runs at every start, also without a
   registry (quarantine only), and after every refresh with the registry's
   verdicts underneath (`official::Verdicts`); `check_verdict` (enable,
   every invocation) and `rollback` apply the official revocations too.
   Signers are recorded for every verified version; a registry copy whose
   version has no record, or whose record cannot be read, is not run
   (fail closed), except a retained version whose installed files match the
   compiled bundled pin (`bundled_match`). A `floors.json` with unknown
   fields is read but never rewritten (`read_floors`, `covered`).
   Check that a quarantined plugin cannot be enabled, and
   that an official revocation cannot be lifted by a registry `Run`
   verdict while the agent still revokes it.
F. **Authenticity is not authorization.** Nothing in `official.rs` writes
   a grant, consent, tier or source. The store's install and update get
   the user's consent only; a transition from the bundle derives nothing
   (unlike seeding); escalations stay staged (`AwaitingConsent`). Confirm
   `decided_by` for any grant touched by this path is `user`.
G. **The gate.** `PRIVILEGED_GATE` is `Closed`; it is a `const`, read in
   `Official::with` only; `admit_privileged` returns the original refusal
   unless `Open` and every review condition holds (threshold at least 2 and
   at least the review's, counted over the keys that actually signed;
   plugin listed; every privileged construct of
   `privileged_constructs` listed; the registry's manifest hash and
   permissions), and then uses only the bundled reader. The store still
   refuses every privileged construct from a non-bundled source
   (`check_privileged_source`), and `check_gate` refuses an unparsable
   manifest.
   Confirm no other code constructs `PrivilegedGate::Open` outside tests.
H. **Transition.** `check_transition` runs only for a record whose source
   is `bundled`, before `trial_load`; it refuses dropped action ids,
   settings and credential slots. The bundled copy stays active on every
   refusal and through crash recovery (`an_interrupted_transition_recovers_and_retries`).
   Seeding leaves a registry copy alone; `restoreBundledPlugin` still
   restores the bundle after an uninstall.
I. **Audit log.** Every verification (accepted or refused, including
   unparsable signatures and floor failures), every refusal in `pins`
   before a download and every missing or undownloadable signature writes
   one chained line; five rotated generations are kept; a write failure is
   logged and does not change the outcome. The limits are documented in
   publisher-signature.md ("Limits of the audit log"). Decide
   whether tamper-evidence (hash chain, head hash in the agent log) is
   enough or a signed or remote log is required.
J. **Migration.** `migration.rs` derives grants only onto a bundled copy;
   `PluginManager::derive_grants` refuses any other source.
K. **Known open items** (not blockers for the closed state):
   `release verify --keys` exists in release builds of the CLI (Agent A's
   finding 3; the plugin repository's CI relies on it with throwaway
   keys). (The former item "only one `.minisig` per release is fetched" is
   resolved: one `<package>.<KEYID>.minisig` per pinned key, threshold 2
   reachable at run time.)

Run: `cargo test -p deckpilot-plugin-registry --all-features --test publisher`
(the multi-signature rules),
`cargo test -p deckpilot-agent --features plugins --lib official`
and `cargo test -p deckpilot-agent --features plugins --test
plugins_registry_https`.

## Out of scope

The registry roles (U-10, U-12), the plugin host sandbox, the 1.9
broker and denial matrix (reviewed in
[module-migration.md §7](module-migration.md#plugin-api-19-security-reviews-2026-11-20)).

## Sign-off

| Reviewer | Date | Outcome | Gate |
| --- | --- | --- | --- |
| | | | closed |
