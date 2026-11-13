# Release readiness

How to decide whether a version is ready to be published, and what is
known about the current one. Publishing itself is described in
[releasing.md](releasing.md); nothing here publishes anything.

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
   release workflow, signs and notarizes, and ends in a **draft** release; verify
   it with `deckpilot-dev verify-release` (see
   [release-validation.md](release-validation.md)).
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

State on **2026-11-13**, commit
`1f1de25371785931f89c2eec2895197311716edc` (`1f1de25`) on main. 0.6.0 is
**not ready to publish**: no signed and notarized 0.6.0 build exists, the
update path and the manual checks have not been done, and no physical
MK.2 has been tested. Nothing below was checked by hand; "verified
automatically" means a test or CI job ran and passed.

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
| Hardware procedure | verified automatically (simulated MK.2 only) | `check.sh` (`hardware-test --mock`) |
| Linux packaging (`.deb`) | verified automatically | Packaging on `d9e2905` (no packaged file changed since) |
| Windows packaging (installers) | verified automatically | Packaging on `d9e2905` (no packaged file changed since) |
| Updater signing keys match | verified automatically | required check on `1f1de25` |
| Apple signing credentials | verified automatically | Signing check on #67 (`55e8aaf`) found the certificate and notarization credentials valid. Nothing was signed |
| macOS signing and notarization of 0.6.0 | not verified | needs a test tag through the Release workflow |
| Signed update and Keychain continuity | not verified | *Update continuity* has not run on main; `tools/release-validation/update-test.sh` has not run for 0.6.0 |
| Manual platform checks (macOS, Windows, Linux) | not verified | none done for 0.6.0 |
| Physical Stream Deck MK.2 | not verified | `deckpilot-dev hardware-test` has not run on hardware |
| MVP acceptance checklist | not passed | [mvp-acceptance.md](mvp-acceptance.md): 8 steps PASS (automated only), 7 NOT VERIFIED, 0 FAIL |
| GitHub, GitLab and Jira modules against real servers | not verified | tested against local mock servers only |

### Definition of Done (macOS MVP)

| Item | Status | Evidence |
| ---- | ------ | -------- |
| No P0 blockers | met, as far as recorded | No open issues, no failing check on `1f1de25`, and `check.sh` found no defect. The open items below are known gaps, not recorded defects. Hardware, the signed build and the update have not been tried, so blockers there would not show yet |
| Native macOS app works without dev tools | not verified | No 0.6.0 app bundle has been installed and started outside `make up` (acceptance steps 1–2) |
| Tray consistently opens the UI | not verified | No test covers the tray menu. Only the agent's window requests over IPC are tested (acceptance step 13) |
| MK.2 works in real use | not verified | Simulated MK.2 only (`hardware-test --mock`, agent tests). No physical device has been tested, and #55 waits for hardware |
| Actions and variables fully functional | met (automated, simulated device) | Acceptance steps 7–10: the app commands and agent tests named in mvp-acceptance.md. Not yet done in the installed app |
| Workflows reliable | met (automated, simulated device) | Acceptance step 11: `workflow_e2e.rs` (combined graph, cancellation, triggers and the concurrency limit, a restart during a run), `runs_workflows_over_ipc`, `mock_devices.rs`. Not yet done in the installed app |
| Config survives restart and update | not verified | Agent restarts and database upgrades are tested (`lifecycle.rs`, `upgrade_from_0_5_0`, `upgrade_from_0_6_0`, `upgrade_work_sessions_from_schema_8`). There has been no real reboot and no signed update for 0.6.0 (acceptance steps 14–15) |
| Signed and notarized macOS build verified | not verified | Verified for `v0.5.0-2` only; there is no 0.6.0 test release |
| No critical data-loss or security defects | met, as far as known | 0 open code scanning alerts and 0 open Dependabot alerts; the storage upgrade, backup and secret-redaction tests pass. There has been no security review of 0.6.0 beyond these |
| Acceptance checklist passes | not met | 7 of 15 steps are NOT VERIFIED ([mvp-acceptance.md](mvp-acceptance.md)) |

### Known open items

- **#55** (draft): the MK.2 HID wire protocol with byte-level tests. It
  needs a physical MK.2 and the
  [physical MK.2 checklist](release-validation.md#physical-mk2-checklist)
  before it can leave draft. `main` drives the device through
  `elgato-streamdeck`.
- **#63** (optional read-only smoke tests against real services) and
  **#78** (reorder, move and copy pages in the layout editor): held by the
  MVP freeze.
- **#159–#165**: Dependabot updates waiting for an owner decision (the
  GitHub Actions group, the npm group, jsdom 31,
  `@testing-library/jest-dom` 7, `chacha20poly1305` 0.11, `tray-icon` 0.26
  and `base64` 0.23). Several are major versions, and `chacha20poly1305`
  and `tray-icon` touch encryption and the tray.

### Owner actions before the first pre-release

1. **Create the `production-release` environment** with a required
   reviewer and a tag deployment policy
   ([releasing.md](releasing.md#publication-gate-and-signing-environment-repository-settings)).
   On 2026-11-13 the repository has no environments.
2. **Move the signing secrets into it** and run the non-publishing
   validation
   ([releasing.md](releasing.md#moving-the-signing-secrets-into-the-environment)).
3. **Tag ruleset.** A *Release tag protection* ruleset is active on
   `refs/tags/v*`. It blocks deletion, update and non-fast-forward and has
   no bypass actors. Confirm that this is the intended rule set for
   release tags, or extend it (for example, restrict who may create `v*`
   tags).
4. **Physical MK.2 checks:** run `deckpilot-dev hardware-test` on a real
   MK.2 and the
   [physical MK.2 checklist](release-validation.md#physical-mk2-checklist),
   and record the results in release-validation.md.
5. Then make the signed test release `v0.6.0-alpha.1` (draft) with the
   exact commands in
   [releasing.md](releasing.md#the-first-pre-release-command-by-command).
   They were not run for this report. After that, do the manual
   acceptance run in [mvp-acceptance.md](mvp-acceptance.md) with the build
   from the draft.

Known limitation: before #54, agents, tests and the compatibility matrix
failed with "local socket name length exceeds capacity of sun_path" when
`TMPDIR` was a very long path (macOS limits socket paths to 104 bytes).
#54 keeps socket paths within that limit, and its test
(`agent_and_client_meet_with_a_long_tmpdir`) passed in this run.
