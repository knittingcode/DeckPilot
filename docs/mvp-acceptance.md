# MVP acceptance checklist (macOS)

A repeatable acceptance run for the macOS MVP. Each step is marked:

- **PASS**: direct evidence for this exact step on macOS. So far that is
  always an automated test that ran on macOS (`tools/release-readiness/check.sh`
  on a Mac and the CI job `Rust (macos-latest)`). The test is named, and
  "automated, simulated device" means it used the simulated MK.2
  (`MockBackend` or `--mock-devices mk2`), not real hardware. An automated
  PASS drives the agent and the app's Tauri commands with the same JSON the
  frontend sends. It does **not** click through the installed app's
  windows.
- **FAIL**: the step was tried and did not work.
- **NOT VERIFIED**: there is no direct evidence for this step on macOS. The
  reason and what the owner has to do are given.

Nothing in this checklist was checked by hand, on a physical Stream Deck
MK.2, with a signed or notarized 0.6.0 build, across a real reboot or with
a real signed update. Every step that needs one of those is NOT VERIFIED
until the owner does it and records the result here.

## Last run

| | |
| - | - |
| Version | 0.6.0 (unreleased) |
| Commit | `1f1de25` on `main` |
| Date | 2026-11-13 |
| Automated evidence | `tools/release-readiness/check.sh` on macOS (results in [release-readiness.md](release-readiness.md#report-for-060)); CI on `1f1de25` |
| Manual run | none yet |

## Steps

| # | Step | Status | Evidence or reason | Still to do by the owner |
| - | ---- | ------ | ------------------ | ------------------------ |
| 1 | Install DeckPilot | NOT VERIFIED | No signed and notarized 0.6.0 disk image exists: no `v0.6.0-alpha.N` release has run. The 0.5.0 test releases (`v0.5.0-1`, `v0.5.0-2`) were signed and notarized ([release-validation.md](release-validation.md#results-for-050)), but that was a different version | Run a signed test release ([releasing.md](releasing.md#the-first-pre-release-command-by-command)), download the draft's disk image on a Mac, check it with `spctl --assess --type open --context context:primary-signature -v` and `xcrun stapler validate`, and drag the app to Applications. Gatekeeper must not warn |
| 2 | Launch it | NOT VERIFIED | Only the agent executable is started by tests (`apps/agent/tests/executable.rs` `the_executable_starts_and_prints_its_version`; `apps/agent/tests/mock_devices.rs`). The native app bundle has not been started outside of development tools | Start DeckPilot from Applications (not `make up`, no dev server). The window opens, the tray icon appears and the agent runs (`deckpilot-agent` in Activity Monitor) |
| 3 | Unlock or initialize secure storage | PASS (automated, no device) | Through the app's commands against a real agent and a real SQLCipher database: `apps/desktop/src-tauri/src/tests.rs` `a_fresh_install_opens_without_a_password` (initialize with a system key), `the_configuration_is_locked_without_the_password`, `switching_between_password_and_system_key`, `a_locked_system_key_configuration_unlocks_with_the_system_check`; agent side `apps/agent/tests/agent.rs` `starts_locked_and_unlocks_only_with_the_right_key`. These tests use an in-memory key store instead of the Keychain. The real macOS Keychain is exercised only in isolation, under throwaway names (`crates/deckpilot-keystore/tests/system_store.rs`, CI job `Credential store (macos-latest)`), and not with the signed app's code identity | In the installed app: complete first-run setup with the default protection (Keychain), quit and restart, and check that it opens without a prompt. Then lock it (Settings → Security → Lock now) and unlock it |
| 4 | Connect Stream Deck MK.2 | NOT VERIFIED | Discovery, layout and key input are tested with a simulated MK.2 only (`apps/agent/tests/agent.rs` `renders_layout_and_reports_status`, `recovers_from_unplug_and_replug`; `deckpilot-dev hardware-test --mock` in check.sh). No physical MK.2 has been tested by the project. The MK.2 wire protocol in draft #55 is held until it passes on hardware | Quit `make up` and the Elgato app, connect a physical MK.2, run `deckpilot-dev hardware-test` and do the [physical MK.2 checklist](release-validation.md#physical-mk2-checklist). Record the firmware, serial and `report.md` |
| 5 | Create a profile | PASS (automated, no device) | `apps/desktop/src-tauri/src/tests.rs` `configuration_flow_through_ipc` (`create_profile`, then it becomes the active profile), `duplicated_profiles_are_isolated` | Create a profile in the installed app's window |
| 6 | Create three projects | PASS (automated, no device) | `apps/desktop/src-tauri/src/tests.rs` `configuration_flow_through_ipc` creates SHOP, CMR and CMS with `create_project` and refuses a duplicate key; `projects_stay_in_their_profile` | Create three projects in the installed app |
| 7 | Create one shared variable | PASS (automated, no device) | `apps/desktop/src-tauri/src/tests.rs` `shared_variables_are_managed_without_revealing_secrets` (`save_shared_variable`, `list_shared_variables`); resolution in runs: `apps/agent/tests/variables.rs` `actions_resolve_references_when_they_run`, `a_changed_value_is_used_by_the_next_run_and_announced`; storage upgrade keeps them: `crates/deckpilot-storage/tests/upgrade_from_0_6_0.rs` `shared_variables_work_on_the_upgraded_database_and_survive_a_reopen` | Create a shared variable in the installed app and use it in an action |
| 8 | Create a parameterized action | PASS (automated, simulated device) | `apps/desktop/src-tauri/src/tests.rs` `runtime_state_and_actions_go_through_the_agent`: `create_action` with the parameter `on {project.key}`, run by the agent, result `on CMR`; `actions_can_be_edited_duplicated_and_are_protected_while_used` (invalid parameters refused). Per-binding parameter overrides (`BindingTarget::Invoke`) are deferred by the MVP freeze ([gestures-and-work-session.md](gestures-and-work-session.md) §3.2) and are not part of this step | Create an action with a parameter that uses a variable and run it from the app |
| 9 | Assign the action to a key | PASS (automated, simulated device) | Assignment: `apps/desktop/src-tauri/src/tests.rs` `layouts_are_designed_offline_and_validated` (binds an action to key 0 of an MK.2 layout with `save_layout`, previews the key, refuses key 15). Key press runs the bound target: `apps/agent/tests/mock_devices.rs` `the_mock_agent_navigates_runs_workflows_and_switches_profiles` (agent executable, simulated MK.2) | Assign the action to a key in the layout editor and press the physical key |
| 10 | Cycle projects using one key | PASS (automated, simulated device) | `apps/agent/tests/agent.rs` `button_press_cycles_project_persists_and_notifies` (one key cycles SHOP → CMR → CMS → SHOP, persisted, only that key redrawn); `apps/agent/tests/projects.rs` `concurrent_presses_advance_exactly_one_project_each`, `keys_show_the_project_after_a_cycle_a_reconnect_and_a_profile_change` | Press a key bound to `projects.cycle` on the physical MK.2 (manual step 5 of [release-validation.md](release-validation.md#3-stream-deck-mk2-physical-device)) |
| 11 | Create and run a workflow | PASS (automated, simulated device) | Create: `apps/desktop/src-tauri/src/tests.rs` `workflows_are_validated_and_keep_their_step_order` (`save_workflow`), `restarting_the_agent_never_ends_running_workflows_silently` (`save_workflow` and `start_workflow` against a running agent). Run: `apps/agent/tests/agent.rs` `runs_workflows_over_ipc`; `apps/agent/tests/workflow_e2e.rs` (sub-workflow, variables, condition, parallel branches, retries, cancellation, restart during a run); key-started run in `apps/agent/tests/mock_devices.rs` | Build a workflow in the installed app's workflow builder and run it from the app and from a key |
| 12 | Close the UI while the agent continues | NOT VERIFIED | Partial evidence only: the agent runs and serves keys with no app at all (`apps/agent/tests/mock_devices.rs`), and a new app process reuses the running, unlocked agent (`apps/desktop/src-tauri/src/tests.rs` `a_restarted_app_reuses_the_unlocked_agent_without_asking`). Closing the real app window on macOS has not been tried | In the installed app, close the window (and quit the app from the Dock). Keys keep working; `deckpilot-agent` keeps running (manual step 9 of release-validation.md §3) |
| 13 | Reopen the UI from the tray | NOT VERIFIED | No automated test covers the tray menu. Only the IPC part is tested: the agent asks the app for windows instead of drawing them (`apps/agent/tests/agent.rs` `requests_windows_from_the_app_instead_of_drawing_them`) | With the window closed, open DeckPilot from the tray icon several times; the window opens each time |
| 14 | Restart the computer and use the app again | NOT VERIFIED | No real reboot was done. Related automated evidence: configuration and the active project persist across agent restarts (`apps/agent/tests/agent.rs` `button_press_cycles_project_persists_and_notifies`, `apps/agent/tests/lifecycle.rs`, `apps/agent/tests/workflow_e2e.rs` `a_run_interrupted_by_a_restart_is_recorded_and_new_runs_work`). Autostart at login and Keychain unlock after a reboot are not tested | Restart the Mac. The agent starts at login, the configuration opens (from the Keychain without a prompt, or after the password), and the profile, projects, variables, actions, key bindings and workflows are unchanged |
| 15 | Update DeckPilot without losing data | NOT VERIFIED | No signed update to 0.6.0 exists. Related evidence: database upgrades keep data (`crates/deckpilot-storage/tests/upgrade_from_0_5_0.rs`, `upgrade_from_0_6_0.rs`, `upgrade_work_sessions_from_schema_8.rs`, in check.sh); `tools/release-validation/update-test.sh` passed for 0.5.0 → 0.5.1 (release-validation.md, results for 0.5.0); the *Update continuity* workflow last passed on a pull request branch (`fix/updater-robustness`, `d69c967`, https://github.com/knittingcode/DeckPilot/actions/runs/43174801300) and has not run on `main` at `1f1de25` | Install the 0.5.0 build (or the first 0.6.0 pre-release), configure it, then update through the app's updater to a signed newer build. The same profiles, projects, variables, actions, layouts and workflows are there and the Keychain opens without a prompt. Also run *Update continuity* on `main` (`gh workflow run "Update continuity" --ref main`; it uses the Developer ID certificate) |

## Summary

| Status | Steps |
| ------ | ----- |
| PASS (automated only) | 3, 5, 6, 7, 8, 9, 10, 11 |
| FAIL | none |
| NOT VERIFIED | 1, 2, 4, 12, 13, 14, 15 |

The checklist **does not pass**: seven steps need a signed build, a physical
MK.2, the real tray, a reboot or a real update. The eight automated passes
also need to be repeated in the installed app for a manual acceptance run.

## How to repeat

1. On a Mac, from a clean checkout of the commit under test:
   `tools/release-readiness/check.sh` (add `--with-compat` when there is
   time). Every test named above is part of it. Record the result lines.
2. Check that CI is green on that commit (required checks, Packaging,
   Signing check, Compatibility).
3. Install the signed and notarized build from the draft release (step 1)
   and do steps 1 to 15 by hand, in order, on one Mac with a physical MK.2.
   Use a fresh macOS user account, or one without an earlier DeckPilot
   configuration, so that step 3 initializes the storage.
4. For each step record PASS or FAIL, the date, the DeckPilot version, the
   macOS version and the MK.2 firmware and serial. Replace the automated
   status only with what was actually observed. A step that was not done
   stays NOT VERIFIED.
