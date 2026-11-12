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
4. **Signed test release.** An alpha tag (`vX.Y.Z-alpha.N`; the former
   `vX.Y.Z-N` test tags no longer start the release workflow, see
   [releasing.md](releasing.md#release-tags-and-the-tag-gate)) runs the real
   release workflow, signs and notarizes, and ends in a **draft** release; verify
   it with `deckpilot-dev verify-release` (see
   [release-validation.md](release-validation.md)).
5. **Update path.** `update-continuity` workflow (Keychain continuity
   between two Developer ID signed builds) and
   `tools/release-validation/update-test.sh` (tampered update rejected, real
   update installed, configuration kept) on macOS.
6. **Manual platform checks** (below) on real hardware where possible.
7. **Report.** Fill in the table for the version: verified automatically,
   verified manually, not verified, blocked, not applicable. Nothing
   untested is marked as passed.

## Manual platform checks

| Platform | Check |
| -------- | ----- |
| macOS | real app and agent; Keychain (system key) and password protection; native notifications; opening applications; workflows; update; signed and notarized build opens without warnings |
| Windows | installer; agent start, IPC and restart; DPAPI-protected system key; notifications (PowerShell toast); process execution |
| Linux | `.deb`/AppImage install; agent; Secret Service; `notify-send`; process execution |
| Hardware | a physical Stream Deck MK.2 with `deckpilot-dev hardware-test` |

## Report for 0.6.0

State on 2026-11-11, commit `cb222c4` on main (the automated checks on this
computer ran on `f5d943c`, which differs only in `CHANGELOG.md`). 0.6.0 is
**not ready to publish**: the signed build, the update path and the manual
checks below have not been done.

| Area | Status | Evidence |
| ---- | ------ | -------- |
| Versions and release notes | verified automatically | `check.sh`: versions are consistent, release notes exist |
| Rust formatting and lints | verified automatically | `check.sh`; CI on main |
| Rust tests (SQLCipher, migrations, IPC, updater, modules, redaction) | verified automatically | `check.sh` on macOS; CI Rust on macOS, Linux and Windows, Storage with SQLCipher |
| Upgrade from a real 0.5.0-2 database | verified automatically | `check.sh` (`upgrade_from_0_5_0`) |
| IPC peers of earlier protocols | verified automatically | `check.sh` (`peers`) |
| Compatibility with v0.5.0-2 (real binaries) | verified automatically | `check.sh --with-compat` on macOS; Compatibility workflow on `d4d099c` |
| Frontend format, lint, types and tests | verified automatically | `check.sh`; CI Frontend |
| Hardware procedure | verified automatically (simulated MK.2 only) | `check.sh` (`hardware-test --mock`) |
| Linux packaging (`.deb`) | verified automatically | CI on main: Package check (Linux .deb) |
| Windows packaging (installers) | verified automatically | CI on main: Package check (Windows installers) |
| Updater signing keys match | verified automatically | required check on every merged pull request |
| macOS signing and notarization | not verified | needs a test tag through the Release workflow |
| Signed update and Keychain continuity | not verified | `update-continuity` workflow and `tools/release-validation/update-test.sh` not run for 0.6.0 |
| Manual platform checks (macOS, Windows, Linux) | not verified | none done for 0.6.0 |
| Physical Stream Deck MK.2 | not verified | `deckpilot-dev hardware-test` not run on hardware |
| GitHub, GitLab and Jira modules against real servers | not verified | tested against local mock servers only |

Known limitation: agents, tests and the compatibility matrix fail with
"local socket name length exceeds capacity of sun_path" when `TMPDIR` is a
very long path (macOS limits socket paths to 104 bytes). The default
temporary folder is short enough.

Before publishing: a signed test release (`v0.6.0-alpha.1`, draft), the
update path from a 0.5.0 build, and the manual checks above.
