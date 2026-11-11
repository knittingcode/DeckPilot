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
4. **Signed test release.** A test tag (`vX.Y.Z-N`) runs the real release
   workflow, signs and notarizes, and ends in a **draft** release; verify
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

See the section at the end of this file.
