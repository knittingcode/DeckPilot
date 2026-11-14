# Release readiness

How to decide whether a version is ready to be published, and what is
known about the current one. Publishing itself is described in
[releasing.md](releasing.md); nothing here publishes anything.

## Current status (`main` at `9f8a84a`, 2026-11-14)

This section replaces the blockers of the
[report of 2026-11-13](#report-for-060) below, which is kept as history.

- **No release has been published yet.**
- **Release authorization: tag authority.** `production-release` has no
  required reviewers, deploys only from tags `v*`, and administrators
  cannot bypass it. The *Release tag protection* ruleset restricts the
  creation, update and deletion of `refs/tags/v*`; its only bypass actor
  is the *Repository admin* role (bypass mode *always*). Creating a
  protected `v*` tag is the authorization, and stage B publishes
  automatically once every check has passed again
  ([releasing.md](releasing.md#release-authorization)). The release gate
  (`tools/release-pipeline/check-environment.sh`) verifies this on every
  run; the *Release authorization* job of the *Signing check* workflow
  passed on `9f8a84a`
  ([run 43609631711](https://github.com/knittingcode/DeckPilot/actions/runs/43609631711)).
- **Signing secrets** are environment secrets of `production-release`
  only; the repository-level copies have been deleted.
- **Signed and notarized 0.6.0 build.** The draft release of
  `v0.6.0-alpha.2` (`13cabe6`) was built in
  [run 43597399166](https://github.com/knittingcode/DeckPilot/actions/runs/43597399166):
  macOS arm64 and x86_64 Developer ID signed (team `PUB9L83UMZ`) with the
  hardened runtime, notarized, stapled and accepted by Gatekeeper; updater
  signatures, `SHA256SUMS`, `release-meta.json` and `prerelease.json`
  verified; 17 assets. It was **not** published: its *Publish and verify*
  deployment was cancelled when the required reviewers were removed.
- **Update continuity** passed on `v0.6.0-alpha.2`
  ([run 43606772522](https://github.com/knittingcode/DeckPilot/actions/runs/43606772522)):
  two Developer ID signed builds with the same code identity, a tampered
  update rejected, the signed update 0.5.0 → 0.5.1 installed, the
  configuration opened from the Keychain without a prompt before and after
  the update, and the data preserved and encrypted.
- **Tags.** The historical tags `v0.5.0-1` and `v0.6.0-alpha.1` and the
  draft releases of 0.5.0-1 and 0.5.0-2 have been deleted. The tag
  `v0.5.0-2` is kept as the compatibility baseline.
- **Security alerts:** no open code scanning, Dependabot or secret
  scanning alerts.

Still open:

| Item | State |
| ---- | ----- |
| First published release | not done; a `v*` tag publishes automatically ([releasing.md](releasing.md#the-first-pre-release-command-by-command)) |
| Install of a signed build by hand (acceptance steps 1, 14, 15) | not recorded ([mvp-acceptance.md](mvp-acceptance.md)) |
| Physical MK.2 retest on the current `main` | not recorded; the physical run of 2026-11-13 was on `5569a14` |
| Windows code signing (Authenticode) | not provisioned; Windows and Linux builds are not code-signed |

## Procedure

1. **Version.** `deckpilot-dev set-version <x.y.z>` writes the version to
   every place that declares it (the workspace `Cargo.toml`, `Cargo.lock`,
   `apps/desktop/package.json`, `apps/desktop/src-tauri/tauri.conf.json`).
   `deckpilot-dev check-versions [--tag vX.Y.Z]` refuses any difference
   and a version without CHANGELOG notes. The workspace version is the
   single source; the agent, the app and the update manifest all take it.
2. **Automated checks on this computer.**
   `tools/release-readiness/check.sh --with-compat` runs versions, release
   notes, formatting, lints, all Rust tests (SQLCipher, migrations, IPC,
   updater signatures, modules, redaction), the upgrade from a real 0.5.0
   database, IPC peers, the frontend checks, the simulated hardware
   procedure and the real-binary compatibility matrix. Every line is
   `verified`, `FAILED` or `skipped` — a skipped check is not verified.
3. **CI on main.** All required checks green on the release commit: Rust on
   macOS, Linux and Windows, Frontend, Storage with SQLCipher, Linux and
   Windows packaging, the signing checks, and the Compatibility workflow.
4. **Unsigned installer checks on the tag.** The release workflow's first
   stage calls the *Packaging* workflow on the tagged commit (`.deb`,
   `.rpm`, NSIS, MSI with the release configuration and a throwaway updater
   key, checked by `tools/ci/check-packages.sh`); nothing is signed or
   drafted unless it passes. A failure here is fixed on `main` and released
   under a new tag.
5. **Signed test release.** An alpha tag (`vX.Y.Z-alpha.N`; the former
   `vX.Y.Z-N` test tags no longer start the release workflow, see
   [releasing.md](releasing.md#release-tags-and-the-tag-gate)) runs the real
   release workflow, signs and notarizes into a **draft** release and, under
   tag authority, **publishes** it as a pre-release once every check passes
   again, without an approval. The workflow verifies the draft before it
   publishes; `deckpilot-dev verify-release` checks the assets again by
   hand (see [release-validation.md](release-validation.md)).
6. **Update path.** `update-continuity` workflow (Keychain continuity
   between two Developer ID signed builds) and
   `tools/release-validation/update-test.sh` (tampered update rejected, real
   update installed, configuration kept) on macOS.
7. **Manual platform checks** (below) on real hardware where possible.
8. **Report.** Fill in the table for the version: verified automatically,
   verified manually, not verified, blocked, not applicable. Nothing
   untested is marked as passed.

## Manual platform checks

| Platform | Check |
| -------- | ----- |
| macOS | real app and agent; Keychain (system key) and password protection; native notifications; opening applications; workflows; update; signed and notarized build opens without warnings |
| Windows | installer; agent start, IPC and restart; DPAPI-protected system key; notifications (PowerShell toast); process execution |
| Linux | `.deb`/AppImage install; agent; Secret Service; `notify-send`; process execution |
| Hardware | a physical Stream Deck MK.2 with `deckpilot-dev hardware-test` |

The macOS MVP acceptance run (15 steps, from installing to updating) is
[mvp-acceptance.md](mvp-acceptance.md).

## Report for 0.6.0

> **Historical, superseded** by the [current status](#current-status-main-at-9f8a84a-2026-11-14)
> above. This report describes `main` on 2026-11-13. Its blockers (no
> `production-release` environment, no signed 0.6.0 build, *Update
> continuity* not run, unrestricted creation of `v*` tags) have since been
> resolved, and its owner actions are marked where they no longer apply.

State on **2026-11-13**. The `check.sh` run and the CI table below are for
commit `1f1de25371785931f89c2eec2895197311716edc` (`1f1de25`); the native,
physical and security evidence is for `5569a14`; `main` is now at
`c6834890612305fbd5a6e52e80d882e71022916d` (`c683489`), which adds #171
(`4cddfa2`), #170 (`592d5f1`) and #172 (`c683489`, the D1 fix). 0.6.0 is
**not ready to publish**: no Developer ID signed and notarized 0.6.0 build
exists, the update path and a real reboot have not been tried, the tray
icon has not been clicked, and #169 (environment gate) and #174
(work-session test races) are still open. "Verified automatically" means a test or CI
job ran and passed; "native" means the ad-hoc signed `make package` release
build was run as a macOS app; "physical" means a real Stream Deck MK.2.

### `check.sh` on macOS

`tools/release-readiness/check.sh` on a Mac (Darwin 27.2.0), on a branch
identical to `1f1de25`, with the default `TMPDIR`
(`/var/folders/…/T/`). It ran once and exited 0, so no second run with
`TMPDIR=/tmp` was needed. `--with-compat` was **not** run (the machine was
busy with other work). The Compatibility workflow on `1f1de25` covers it
in CI (below).

```
verified  versions are consistent
verified  release notes exist
verified  Rust formatting
verified  Rust lints
verified  Rust tests (incl. SQLCipher, migrations, IPC, updater, modules, redaction)
verified  upgrade from a real 0.5.0 database
verified  IPC peers of earlier protocols
verified  frontend format
verified  frontend lint
verified  frontend types
verified  frontend tests
verified  hardware procedure (simulated MK.2)
skipped   compatibility with v0.5.0-2 (real binaries) — pass --with-compat (builds the old tag)
skipped   Linux packaging — CI job 'Package check (Linux .deb)'
skipped   Windows packaging — CI job 'Package check (Windows installers)'
skipped   macOS signing and notarization — Release workflow on a test tag, or Signing check workflow
skipped   signed update and Keychain continuity — update-continuity workflow; tools/release-validation/update-test.sh
skipped   manual platform checks — see docs/release-readiness.md
```

The Rust tests reported 1057 passed, 0 failed and 5 ignored (child-process
helpers, a diagnostic, and a test that queries the real platform). They
include `agent_and_client_meet_with_a_long_tmpdir` (#54). The simulated
hardware procedure skipped "Multiple devices" because only one simulated
device was connected.

### CI on `1f1de25`

| Check | Result | Run |
| ----- | ------ | --- |
| Frontend (required) | success | [CI run 43398938268](https://github.com/knittingcode/DeckPilot/actions/runs/43398938268/job/131796655391) |
| Storage with SQLCipher (required) | success | [CI run 43398938268](https://github.com/knittingcode/DeckPilot/actions/runs/43398938268/job/131796655964) |
| Rust (ubuntu-latest) (required) | success | [CI run 43398938268](https://github.com/knittingcode/DeckPilot/actions/runs/43398938268/job/131796656557) |
| Rust (macos-latest) (required) | success | [CI run 43398938268](https://github.com/knittingcode/DeckPilot/actions/runs/43398938268/job/131796655588) |
| Rust (windows-latest) (required) | success | [CI run 43398938268](https://github.com/knittingcode/DeckPilot/actions/runs/43398938268/job/131796655647) |
| Updater signing keys match (required) | success | [Signing check run 43398938208](https://github.com/knittingcode/DeckPilot/actions/runs/43398938208/job/131796655145) |
| Credential store (macOS, Linux, Windows; real system stores, throwaway names) | success | [CI run 43398938268](https://github.com/knittingcode/DeckPilot/actions/runs/43398938268) |
| Release manifest assembly | success | [CI run 43398938268](https://github.com/knittingcode/DeckPilot/actions/runs/43398938268/job/131796733856) |
| Compatibility: previous release (v0.5.0-2) with this commit | success | [Compatibility run 43398938221](https://github.com/knittingcode/DeckPilot/actions/runs/43398938221) |
| Developer ID and notarization credentials | skipped on pushes to main (by design) | [Signing check run 43398938208](https://github.com/knittingcode/DeckPilot/actions/runs/43398938208). Last success was on the pull request of this commit (#67, head `55e8aaf`): [run 43396899637](https://github.com/knittingcode/DeckPilot/actions/runs/43396899637/job/131789912995) |
| Package check (Linux .deb) | not run on `1f1de25` (no packaging path changed); success on its parent `d9e2905` | [Packaging run 43396604797](https://github.com/knittingcode/DeckPilot/actions/runs/43396604797/job/131789002106) |
| Package check (Windows installers) | not run on `1f1de25`; success on `d9e2905` | [Packaging run 43396604797](https://github.com/knittingcode/DeckPilot/actions/runs/43396604797/job/131789002225) |
| Update continuity | never run on main; last success on a pull request branch (`fix/updater-robustness`, `d69c967`, 2026-11-12) | [run 43174801300](https://github.com/knittingcode/DeckPilot/actions/runs/43174801300) |
| Release (signed, notarized) | not run for 0.6.0; last run `v0.5.0-2` (success, draft) | [run 42875993140](https://github.com/knittingcode/DeckPilot/actions/runs/42875993140) |
| CodeQL | Analyze (actions, javascript-typescript, python) success; Analyze (rust) was still running when this was written | [run 43398937728](https://github.com/knittingcode/DeckPilot/actions/runs/43398937728) |

`1f1de25` differs from `d9e2905` only in `tools/dev/src/hardware.rs` and
`docs/release-validation.md`, and neither is packaged. No workflow that
uses signing secrets was started for this report.

Security alerts (read with `gh api`, open alerts only): code scanning
**0**, Dependabot **0**.

### Areas

| Area | Status | Evidence |
| ---- | ------ | -------- |
| Versions and release notes | verified automatically | `check.sh` |
| Rust formatting and lints | verified automatically | `check.sh`; CI on `1f1de25` |
| Rust tests (SQLCipher, migrations, IPC, updater, modules, redaction) | verified automatically | `check.sh` on macOS; CI Rust on macOS, Linux and Windows, Storage with SQLCipher |
| Upgrade from a real 0.5.0-2 database | verified automatically | `check.sh` (`upgrade_from_0_5_0`) |
| IPC peers of earlier protocols | verified automatically | `check.sh` (`peers`) |
| Compatibility with v0.5.0-2 (real binaries) | verified automatically in CI | Compatibility workflow on `1f1de25`; `check.sh --with-compat` was not run locally this time |
| Frontend format, lint, types and tests | verified automatically | `check.sh`; CI Frontend |
| Hardware procedure | verified automatically (simulated MK.2) and on a physical MK.2 | `check.sh` (`hardware-test --mock`); `deckpilot-dev hardware-test` on MK.2 `A00SA6062K8S7V`: 12 passed, multiple devices skipped ([report](mvp-acceptance.md#physical-mk2-2026-11-13)) |
| Linux packaging (`.deb`) | verified automatically | Packaging on `d9e2905` (no packaged file changed since) |
| Windows packaging (installers) | verified automatically | Packaging on `d9e2905` (no packaged file changed since) |
| Updater signing keys match | verified automatically | required check on `1f1de25` |
| Apple signing credentials | verified automatically | Signing check on #67 (`55e8aaf`) found the certificate and notarization credentials valid. Nothing was signed |
| macOS signing and notarization of 0.6.0 | not verified | needs a test tag through the Release workflow |
| Signed update and Keychain continuity | not verified | *Update continuity* has not run on main; `tools/release-validation/update-test.sh` has not run for 0.6.0 |
| Manual platform checks (macOS, Windows, Linux) | macOS partly verified (native, ad-hoc signed release build); Windows and Linux not verified | macOS: app and agent, system-key storage, tray presence, agent restart, physical MK.2 ([mvp-acceptance.md](mvp-acceptance.md)). Not done: password protection, notifications, opening applications, update, a signed and notarized build |
| Physical Stream Deck MK.2 | verified (physical, one device) | `hardware-test` with the debug agent from `5569a14`, then real use on the release app: [Physical MK.2 (2026-11-13)](mvp-acceptance.md#physical-mk2-2026-11-13). Multiple devices not verified |
| MVP acceptance checklist | not passed | [mvp-acceptance.md](mvp-acceptance.md): steps 2–12 PASS (step 9 with the D1 fix #172 merged as `c683489`), step 13 partial (ShowWindow PASS, tray click NOT VERIFIED), steps 1, 14 and 15 NOT VERIFIED, no open FAIL |
| GitHub, GitLab and Jira modules against real servers | not verified | tested against local mock servers only |

### Definition of Done (macOS MVP)

| Item | Status | Evidence |
| ---- | ------ | -------- |
| No P0 blockers | met, as far as known | Defect D1 (on a fresh install, a layout added to the already-active profile did not reach connected devices) was the only P0 found; it is fixed in #172, merged as `c683489`, and covered by automated tests with a simulated MK.2 (the native and physical runs were not repeated after the fix). QA-1 (24/24), the native journey and the physical run found no other defect. The signed build, a reboot and the update have not been tried, so blockers there would not show yet |
| Native macOS app works without dev tools | met (release build, ad-hoc signed) | `make package` on `5569a14`: the React UI renders without dev tools or Vite, encrypted storage is created with a system key and unlocked, quitting the UI leaves the agent running (re-parented to `launchd`), a second agent refuses to start. Isolated `DECKPILOT_DATA_DIR`, Keychain scope "DeckPilot Development". Not Developer ID signed, not notarized, not installed from a disk image |
| Tray consistently opens the UI | partially verified | Native: the tray icon is present, disappears when the agent stops and returns when it restarts; the `ShowWindow` request (`deckpilot-dev show`) reopens the UI. A click on the tray icon is NOT VERIFIED (the session has no Accessibility permission). Acceptance step 13 |
| MK.2 works in real use | met for a single device (physical) | MK.2 `A00SA6062K8S7V`, macOS aarch64: `hardware-test` 12 passed, 1 skipped (multiple devices); real use on the release app: `projects.cycle` with wraparound, click, double click, long press, the Work Session key and its live timer. One single click in the first round was not logged and could not be reproduced. Multiple devices not verified. #55 (own HID protocol) is still a draft |
| Existing MVP features work end to end | met (simulated and physical), with D1 fixed in #172 (merged, `c683489`) | Native journey on the release build with a simulated MK.2, driven over IPC: profile, 3 projects, shared variables, parameterized actions, gestures, a workflow with a variable reference, Work Session, UI close and reopen, agent restart, persistence, cycle wraparound and execution history PASS; "assign actions to keys" failed (D1). QA-1 (automated, simulated): 24/24 PASS. Physical: keys ran their actions on the release app. The configuration was created over IPC, not by clicks in the window |
| Actions and variables fully functional | met (native and automated, simulated device; physical key presses) | Acceptance steps 7–10. QA-1: parameterized action, select by ID, cycle wraparound, variable resolution, Variable Picker, safe deletion and replacement, profile isolation |
| Workflows reliable | met (native and automated, simulated device; physical, from a key) | Acceptance step 11: `workflow_e2e.rs`, `runs_workflows_over_ipc`, `mock_devices.rs`; QA-1 workflows; Work Session workflows started from the physical key. #171 (merged, `4cddfa2`) made the work-session clock test deterministic; #174 (further work-session test races) is still open |
| Config survives restart and update | partially verified | Agent restart: native, simulated (full persistence) and physical (layout, project and brightness restored). Database upgrades: automated. No real reboot and no signed update for 0.6.0 (acceptance steps 14–15) |
| Signed and notarized macOS build verified | not verified | Verified for `v0.5.0-2` only; there is no 0.6.0 test release. The native build was ad-hoc signed |
| No critical data-loss or security defects | met, as far as known | Security and packaging check (#168): SQLCipher, credential redaction, signing configuration, tag gate, packaging workflows and update channels PASS; 0 open code scanning and 0 open Dependabot alerts. #170 (merged, `592d5f1`) creates the unencrypted copy with mode `0600` and refuses a plaintext database that has credentials. No security review of 0.6.0 beyond these |
| Required CI green | met on `95a3006`; not yet complete on `main` at `c683489` | On `c683489` ([CI run 43422946363](https://github.com/knittingcode/DeckPilot/actions/runs/43422946363)): Frontend, Storage with SQLCipher, Rust (ubuntu-latest) and Rust (macos-latest) success, Rust (windows-latest) still running when this was written; [Signing check run 43422946235](https://github.com/knittingcode/DeckPilot/actions/runs/43422946235) and [Compatibility run 43422946370](https://github.com/knittingcode/DeckPilot/actions/runs/43422946370) success. On `95a3006`: [CI run 43414247478](https://github.com/knittingcode/DeckPilot/actions/runs/43414247478): Frontend, Storage with SQLCipher, Rust (ubuntu-latest, macos-latest, windows-latest) success; [Signing check run 43414247273](https://github.com/knittingcode/DeckPilot/actions/runs/43414247273): Updater signing keys match success. CodeQL (all four analyses success, run still finishing) and the scheduled Packaging run were in progress when this was written |
| Physical results documented | yes | [Physical MK.2 (2026-11-13)](mvp-acceptance.md#physical-mk2-2026-11-13) |
| Release configuration blockers identified | yes | (1) the `production-release` environment does not exist and the repository has no environments; #169 adds a fail-closed environment-gate job before any job that uses a secret (open; its *Release gate* check fails, as intended, while the environment is missing); (2) no Developer ID signed, notarized 0.6.0 build; (3) *Update continuity* has never run on `main`; (4) the tag ruleset does not restrict who creates `v*` tags; (5) Dependabot #159–#165 wait for an owner decision (#163 fails CI on deprecations, #159 changes the release workflows, #161, #162, #164 and #165 are major or 0.x-breaking) |
| Acceptance checklist updated | yes | [mvp-acceptance.md](mvp-acceptance.md), 2026-11-13 |
| Acceptance checklist passes | not met | Steps 1, 14 and 15 NOT VERIFIED, step 13 partial |

### Known open items

- **#55** (draft): the MK.2 HID wire protocol with byte-level tests. It
  needs a physical MK.2 and the
  [physical MK.2 checklist](release-validation.md#physical-mk2-checklist)
  before it can leave draft. `main` drives the device through
  `elgato-streamdeck`.
- **#169** (release environment gate) and **#174** (further work-session
  test races): open on 2026-11-13. Merged on 2026-11-13: **#170**
  (`0600` mode on the unencrypted copy, plaintext credentials refused,
  `592d5f1`), **#171** (deterministic work-session clock test, `4cddfa2`)
  and **#172** (D1: a layout added to the active profile now reaches
  connected devices, `c683489`).
- **#63** (optional read-only smoke tests against real services) and
  **#78** (reorder, move and copy pages in the layout editor): held by the
  MVP freeze.
- **#159–#165**: Dependabot updates waiting for an owner decision (the
  GitHub Actions group, the npm group, jsdom 31,
  `@testing-library/jest-dom` 7, `chacha20poly1305` 0.11, `tray-icon` 0.26
  and `base64` 0.23). Several are major versions, and `chacha20poly1305`
  and `tray-icon` touch encryption and the tray. See
  [Security and packaging check](#security-and-packaging-check-2026-11-13)
  for the classification and CI state of each.

### Owner actions before the first pre-release

As of 2026-11-13; items 1 to 3 and 7 are done or replaced (see the
[current status](#current-status-main-at-9f8a84a-2026-11-14)).

1. *Done.* The `production-release` environment exists, with a tag
   deployment policy `v*` and tag authority instead of a required reviewer
   ([releasing.md](releasing.md#release-authorization)).
2. *Done.* The signing secrets are environment secrets only
   ([releasing.md](releasing.md#moving-the-signing-secrets-into-the-environment)).
3. *Done.* *Release tag protection* on `refs/tags/v*` restricts creation,
   update and deletion; its only bypass actor is the *Repository admin*
   role.
4. **Physical MK.2 checks:** done on 2026-11-13 for one device
   ([report](mvp-acceptance.md#physical-mk2-2026-11-13)). Still to do:
   record the firmware version and test two devices at once.
5. **Tray click:** grant Accessibility to the test session, or click the
   tray icon by hand, to verify that the tray opens the UI (acceptance
   step 13).
6. **Signed build, reboot and update:** install a Developer ID signed,
   notarized build, do a real reboot, and update to a signed newer build
   (acceptance steps 1, 14 and 15).
7. *Replaced.* `v0.6.0-alpha.1` was never released and its tag has been
   deleted; the signed draft of `v0.6.0-alpha.2` exists. Under tag
   authority a new `v*` tag is published, not kept as a draft: follow
   [releasing.md](releasing.md#the-first-pre-release-command-by-command),
   then do the manual acceptance run in
   [mvp-acceptance.md](mvp-acceptance.md) with the signed build.

### Security and packaging check (2026-11-13)

A read-only check on `origin/main` at `5569a14`. No setting, secret,
environment or ruleset was changed and no workflow was started.

| Area | Result | Evidence |
| ---- | ------ | -------- |
| SQLCipher and key handling | PASS | `cargo test --locked -p deckpilot-storage -p deckpilot-keystore`: 148 passed, 0 failed (4 ignored: real-Keychain and long-running tests) |
| Unencrypted copy after the legacy migration | PASS (by design), with residual risk | See below |
| Secrets in variables, exports and logs | PASS | `no_secrets_in_exports::an_export_never_contains_secrets_or_values_of_this_computer`, `shared_variables::sensitive_values_secrets_and_other_profiles_never_leave`, `keystore::tests::secrets_are_redacted`, `sqlite::tests::database_key_is_redacted`, `crypto::tests::keys_round_trip_through_hex_and_are_redacted`, `credentials::tests::ids_are_validated_and_secrets_never_printed`, the `resolution` redaction tests. No log statement formats a secret |
| Signing configuration | PASS (static) | `tauri.release.conf.json` adds the agent binary and updater artifacts; `hardenedRuntime: true`, no entitlements (enforced by `.github/scripts/verify-macos-app.sh`); the agent's `Info.plist` fixes `io.github.knittingcode.deckpilot.agent`; notarization and stapling of app and disk image in `release.yml` (build job). The updater public key is injected from `vars.DECKPILOT_UPDATER_PUBKEY` and checked in the binary. Not run: no signing secret was used |
| Release tag gate | PASS | `tools/release-pipeline/test.sh` 80/80, `test-publication.sh` 65/65, `test-updater-key-pair.sh` 8/8 |
| Packaging workflows | PASS | Packaging on `main` (push, `d9e2905`) and on the PR of `5569a14` green; Signing check green on `main` at `5569a14`. Update continuity is `workflow_dispatch` only and has never run on `main` (last run: a PR on 2026-11-12, green) |
| Update channels and signatures | PASS | `deckpilot-update`: 21 passed, among them `stable_installations_never_accept_pre_releases`, `the_prerelease_channel_offers_the_newest_release_of_either_kind`, `rejects_tampered_artifacts_and_bad_input`, `release_artifacts_must_be_signed_for_their_version` |
| Dependency alerts | PASS | 0 open code scanning alerts, 0 open Dependabot alerts (1 dismissed as tolerable: `glib` 0.18, GHSA-wrw7-89jp-8q8g). `pnpm audit`: no known vulnerabilities. `cargo audit` is not installed and was not run |
| Release environment | FAIL (owner action) | `production-release` does not exist and the repository has no environments; see the comparison below. #169 adds a fail-closed environment-gate job before any job that uses a secret (open) |

**Unencrypted copy.** `Vault::encrypt_plaintext_with`
(`crates/deckpilot-storage/src/vault.rs`) renames the original database to
`deckpilot.unencrypted-backup.db` (an older copy to
`deckpilot.unencrypted-backup-<time>.db`) and never deletes it. The agent
does this on its own at the first start with a system key
(`apps/agent/src/service/security.rs`), and so does
`deckpilot-dev seed --encrypt-legacy`; that is where a copy in a development
data directory comes from. It is documented ([security.md](security.md)),
shown in Settings → Security and in the status list, and deleted there as a
sensitive operation (`remove_unencrypted_copies`). Reproduced in a
temporary directory with test data: the copy holds every profile and,
because the migration accepts any plaintext database, also the secrets of
the `credentials` table if the database had one (file mode `0644`, inside
the data directory). A 0.3 database has no `credentials` table and no
released version writes plaintext databases, so no user's secrets are
exposed this way; the copy does hold the old configuration (paths, URLs,
module settings). Hardening: #170 creates the copy with mode `0600` and
refuses the migration when the plaintext database has credentials. It was
merged on 2026-11-13 as `592d5f1`, after this check, so the `0644`
finding above describes `5569a14` and no longer applies to `main`.

**Dependabot pull requests.**

| PR | Update | Kind | CI | Notes |
| -- | ------ | ---- | -- | ----- |
| #159 | 7 GitHub Actions (`checkout` 4→7, `setup-node` 4→7, `pnpm/action-setup` 4→6, `tauri-action` 0.6→1.0, `upload-artifact` 4→7, `download-artifact` 4→8, `attest-build-provenance` 2→4) | major | green | Changes the release and signing workflows; needs a test release |
| #160 | `@xyflow/react` 12.12.1, `lucide-react` | patch/minor | green | |
| #161 | `jsdom` 27→31 | major (test only) | green | |
| #162 | `@testing-library/jest-dom` 6→7 | major (test only) | green | |
| #163 | `chacha20poly1305` 0.10→0.11 | breaking (0.x) | red on all three Rust jobs | Cryptography (export archives). 0.11: `aead` 0.6, `chacha20` 0.10, `poly1305` 0.9, `cipher` 0.5, edition 2024 and MSRV 1.85, `std`/`stream` features removed; no security fix. CI fails because `XNonce::from_slice` is deprecated (`archive.rs:87`, `:144`) under `-D warnings` |
| #164 | `tray-icon` 0.25→0.26 | breaking (0.x) | green | The agent's tray |
| #165 | `base64` 0.22→0.23 | breaking (0.x) | red on Windows only | Used for keys and archives. 0.23 adds SIMD engines behind a default-on `simd-unsafe` feature and raises MSRV to 1.71. The Windows failure is `work_session.rs:890` (`sleep_and_clock_jumps_are_flagged_and_drop_the_approval`), unrelated to base64 |

**Release environment: expected and actual.**

| Expected by `release.yml` | Actual on 2026-11-13 |
| ------------------------- | -------------------- |
| Environment `production-release` used by `prepare`, the four `build` jobs and `publish`; the *Publication gate* requires at least one required reviewer | No environments (`GET /environments`: `total_count` 0). Pushing a `v*` tag would let GitHub create it **unprotected**: `prepare` and the builds would then read the signing secrets without approval, and only publication would fail closed |
| Deployment policy: selected branches and tags, tag rule `v*` | n/a |
| Signing secrets `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_API_KEY`, as environment secrets (repository secrets work too) | All five exist as repository secrets; no environment secrets |
| Variables `DECKPILOT_UPDATER_PUBKEY`, `APPLE_SIGNING_IDENTITY`, `APPLE_TEAM_ID`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER` (optional `DECKPILOT_ALLOW_UNSIGNED_MACOS`) | All five exist as repository variables; `DECKPILOT_ALLOW_UNSIGNED_MACOS` is not set (unsigned macOS releases are refused) |
| Release tags protected | Ruleset *Release tag protection* (active, `refs/tags/v*`): deletion, update, non-fast-forward; no bypass actors. Creation is not restricted |
| Workflow token read-only by default | `default_workflow_permissions: read`, `can_approve_pull_request_reviews: false`; all actions allowed, SHA pinning not required by the repository (pinning is enforced by `check-workflow-gate.py` instead) |

**Owner actions** (as of 2026-11-13, to be run by the owner only; nothing
here was run). Items 1 to 4 are done, with tag authority instead of
required reviewers (see the
[current status](#current-status-main-at-9f8a84a-2026-11-14)); they are
kept as the record.

1. *Owner action.* Create the environment: Settings → Environments → New
   environment → `production-release` → Deployment branches and tags:
   *Selected branches and tags* → Add rule → type *Tag*, `v*`, and set up
   the release authorization (tag authority, or required reviewers). The
   equivalent `gh api` calls are in
   [releasing.md](releasing.md#publication-gate-and-signing-environment-repository-settings).
2. *Owner action.* Verify it:
   `gh api repos/knittingcode/DeckPilot/environments/production-release --jq '{reviewers: [.protection_rules[] | select(.type == "required_reviewers") | .reviewers[].reviewer.login], branch_policy: .deployment_branch_policy}'`
   and
   `gh api repos/knittingcode/DeckPilot/environments/production-release/deployment-branch-policies --jq '.branch_policies[] | "\(.type) \(.name)"'`
   (at least one reviewer, only `tag v*`).
3. *Owner action.* Optionally move the five signing secrets into the
   environment, run the non-publishing validation, and decide about
   *Signing check* and *Update continuity* before deleting any repository
   secret, exactly as in
   [releasing.md](releasing.md#moving-the-signing-secrets-into-the-environment).
4. *Owner action.* Optionally restrict who may create release tags:
   Settings → Rules → Rulesets → *Release tag protection* → enable
   *Restrict creations* and add the maintainers (or the *Repository admin*
   role) as bypass actors. Until the environment exists, any account with
   write access can start stage A with a `v*` tag.
5. *Owner action.* Decide on #159–#165 (above); #163 needs a code change
   (replacing the deprecated `XNonce::from_slice` in
   `crates/deckpilot-portability/src/archive.rs`) before it can merge.

Known limitation: before #54, agents, tests and the compatibility matrix
failed with "local socket name length exceeds capacity of sun_path" when
`TMPDIR` was a very long path (macOS limits socket paths to 104 bytes).
#54 keeps socket paths within that limit, and its test
(`agent_and_client_meet_with_a_long_tmpdir`) passed in this run.
