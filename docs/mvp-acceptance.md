# MVP acceptance checklist (macOS)

A repeatable acceptance run for the macOS MVP. Each step is marked:

- **PASS**: direct evidence for this exact step on macOS. The kind of
  evidence is always named:
  - **automated**: a test that ran on macOS (`tools/release-readiness/check.sh`
    on a Mac and the CI job `Rust (macos-latest)`). The test is named, and
    "simulated device" means it used the simulated MK.2 (`MockBackend` or
    `--mock-devices mk2`), not real hardware. An automated PASS drives the
    agent and the app's Tauri commands with the same JSON the frontend
    sends. It does **not** click through the app's windows.
  - **native**: the release build (`make package`) was started as a macOS
    app, with no dev tools and no Vite, and the result was observed in its
    window. A native step with a **simulated** MK.2 was driven over IPC
    (`deckpilot-dev`), not by clicks in the window; the UI was watched
    live.
  - **physical**: a real Stream Deck MK.2 was connected and the owner
    pressed its keys or answered the visual questions.
- **FAIL**: the step was tried and did not work.
- **NOT VERIFIED**: there is no direct evidence for this step on macOS. The
  reason and what the owner has to do are given.

No step in this checklist was done by hand with a Developer ID signed or
notarized 0.6.0 build, installed from a disk image, across a real reboot or
with a real signed update. A signed and notarized 0.6.0 build exists (the
`v0.6.0-alpha.2` draft, checked automatically; see step 1), but it has not
been installed and used by hand. The native evidence comes from an
**ad-hoc signed** release build. Every step that needs a signed build, a
reboot or an update is NOT VERIFIED until the owner does it and records the
result here.

## Last run

| | |
| - | - |
| Version | 0.6.0 (unreleased) |
| Commit | `5569a14` on `main` for the native and physical runs; `1f1de25` for the earlier `check.sh` run. `main` is now at `c683489`, which adds #171, #170 and #172 (the D1 fix) after `5569a14`; the native and physical runs were not repeated on it |
| Date | 2026-11-13 |
| Automated evidence | `tools/release-readiness/check.sh` on macOS (results in [release-readiness.md](release-readiness.md#report-for-060)); CI on `1f1de25` and `95a3006`; automated regression QA-1 (simulated MK.2): 24 of 24 items PASS, no defects; suites passed: desktop 74, core 291, modules 33, portability 29, storage 139, agent 110 (timing files run three times each), frontend 440 |
| Native run | `make package` release build on `5569a14`: `DeckPilot.app` 0.6.0, `io.github.knittingcode.deckpilot`, agent bundled, **ad-hoc signed only** (not Developer ID signed, not notarized), isolated `DECKPILOT_DATA_DIR`, Keychain scope "DeckPilot Development". Journey with a simulated MK.2 driven over IPC, UI observed live |
| Physical run | Stream Deck MK.2, serial `A00SA6062K8S7V`, macOS aarch64: guided `deckpilot-dev hardware-test` with the debug agent built from `5569a14`, then real use on the release app (see [Physical MK.2 (2026-11-13)](#physical-mk2-2026-11-13)) |
| Signed build | `v0.6.0-alpha.2` draft (`13cabe6`, [release run 43597399166](https://github.com/knittingcode/DeckPilot/actions/runs/43597399166)): Developer ID signed, notarized and stapled for macOS arm64 and x86_64, Gatekeeper accepted, checked automatically; not published |
| Manual run with a signed build | none yet |

## Steps

| # | Step | Status | Evidence or reason | Still to do by the owner |
| - | ---- | ------ | ------------------ | ------------------------ |
| 1 | Install DeckPilot | NOT VERIFIED | A signed and notarized 0.6.0 disk image exists in the unpublished `v0.6.0-alpha.2` draft ([release run 43597399166](https://github.com/knittingcode/DeckPilot/actions/runs/43597399166): Developer ID signed with the hardened runtime, notarized, stapled and accepted by Gatekeeper for macOS arm64 and x86_64, checked by the workflow), but no install from it, or from a published signed release, has been recorded. The native run used an ad-hoc signed `make package` bundle, which is not an install from a disk image and says nothing about Gatekeeper. The 0.5.0 test releases (`v0.5.0-1`, `v0.5.0-2`) were signed and notarized ([release-validation.md](release-validation.md#results-for-050)), but that was a different version | Download the disk image of the first published signed release (or of the `v0.6.0-alpha.2` draft) on a Mac, check it with `spctl --assess --type open --context context:primary-signature -v` and `xcrun stapler validate`, and drag the app to Applications. Gatekeeper must not warn |
| 2 | Launch it | PASS (native, release build, ad-hoc signed) | The `make package` build of `5569a14` started as a macOS app: the React UI rendered without dev tools or Vite, the bundled agent ran, and the tray icon appeared in the menu bar. A second agent refused to start ("already running"), so only one instance runs. Automated: `apps/agent/tests/executable.rs` `the_executable_starts_and_prints_its_version`. Not shown: a launch of a Developer ID signed, notarized build from Applications | Repeat with the signed and notarized build from Applications (step 1) |
| 3 | Unlock or initialize secure storage | PASS (native, release build; automated, no device) | Native: on an empty, isolated data directory the release build created the encrypted storage with a system key and unlocked it (Keychain scope "DeckPilot Development", not the signed app's code identity). Automated, through the app's commands against a real agent and a real SQLCipher database: `apps/desktop/src-tauri/src/tests.rs` `a_fresh_install_opens_without_a_password`, `the_configuration_is_locked_without_the_password`, `switching_between_password_and_system_key`, `a_locked_system_key_configuration_unlocks_with_the_system_check`; agent side `apps/agent/tests/agent.rs` `starts_locked_and_unlocks_only_with_the_right_key`. The real macOS Keychain under the production identity is exercised only by the CI job `Credential store (macos-latest)`, under throwaway names | In the signed, installed app: complete first-run setup with the default protection (Keychain), quit and restart, and check that it opens without a prompt. Then lock it (Settings → Security → Lock now) and unlock it |
| 4 | Connect Stream Deck MK.2 | PASS (physical) | MK.2 `A00SA6062K8S7V` on macOS aarch64: `deckpilot-dev hardware-test` passed device discovery, model detection, layout, key images, all 15 keys, brightness, unplug and replug ([report below](#physical-mk2-2026-11-13)). On the release app the Dashboard showed "Stream Deck MK.2 · Connected · USB". Automated (simulated): `apps/agent/tests/agent.rs` `renders_layout_and_reports_status`, `recovers_from_unplug_and_replug` | Record the MK.2 firmware version (not captured). Repeat with the signed build |
| 5 | Create a profile | PASS (native, simulated device; automated, no device) | Native: profile created on the release build over IPC, shown live in the UI, persisted across UI close, reopen and an agent restart. Automated: `apps/desktop/src-tauri/src/tests.rs` `configuration_flow_through_ipc` (`create_profile`, then it becomes the active profile), `duplicated_profiles_are_isolated` | Create a profile by clicking in the signed app's window |
| 6 | Create three projects | PASS (native, simulated device; automated, no device) | Native: three projects created on the release build over IPC, persisted. Automated: `apps/desktop/src-tauri/src/tests.rs` `configuration_flow_through_ipc` creates SHOP, CMR and CMS with `create_project` and refuses a duplicate key; `projects_stay_in_their_profile` | Create three projects by clicking in the signed app |
| 7 | Create one shared variable | PASS (native, simulated device; automated, no device) | Native: shared variables created on the release build and resolved in a workflow reference. Automated: `apps/desktop/src-tauri/src/tests.rs` `shared_variables_are_managed_without_revealing_secrets`; `apps/agent/tests/variables.rs` `actions_resolve_references_when_they_run`, `a_changed_value_is_used_by_the_next_run_and_announced`; `crates/deckpilot-storage/tests/upgrade_from_0_6_0.rs` `shared_variables_work_on_the_upgraded_database_and_survive_a_reopen`; QA-1 variable resolution and Variable Picker | Create a shared variable by clicking in the signed app and use it in an action |
| 8 | Create a parameterized action | PASS (native, simulated device; automated, simulated device) | Native: parameterized actions on the release build (`projects.select` by ID, `projects.cycle`). Automated: `apps/desktop/src-tauri/src/tests.rs` `runtime_state_and_actions_go_through_the_agent` (`create_action` with `on {project.key}`, result `on CMR`); `actions_can_be_edited_duplicated_and_are_protected_while_used`; QA-1 parameterized action and select by ID. Per-binding parameter overrides (`BindingTarget::Invoke`) are deferred by the MVP freeze ([gestures-and-work-session.md](gestures-and-work-session.md) §3.2) | Create an action with a parameter by clicking in the signed app and run it from the app |
| 9 | Assign the action to a key | PASS (physical, release app); defect D1 fixed in #172 (merged, `c683489`) | Physical: on the release app with the MK.2, the owner pressed the keys and the agent log confirms that each bound action ran (`projects.cycle`, click, double click, long press and the Work Session key; see [below](#physical-mk2-2026-11-13)). Native, simulated: this step **failed** on a fresh install (**D1**): a layout added to the profile that is already active did not reach connected devices until the profile was activated again or the agent restarted. Root cause: an empty page stack passes `all()` in `reload_profile` (`crates/deckpilot-core/src/device_runtime.rs`). Fix: #172 (`reload_sends_devices_without_pages_to_a_new_layout_home`, `a_layout_added_to_the_active_profile_reaches_connected_devices`), merged on 2026-11-13 as `c683489`. The fresh-install path with the fix is covered by those automated tests (simulated MK.2); the native and physical runs used `5569a14`, before the fix, and were not repeated. Automated: `layouts_are_designed_offline_and_validated`, `apps/agent/tests/mock_devices.rs` `the_mock_agent_navigates_runs_workflows_and_switches_profiles`; QA-1 per-key bindings | On a fresh install of the signed build, add a layout to the active profile, assign an action to a key and press the physical key without re-activating the profile or restarting the agent |
| 10 | Cycle projects using one key | PASS (physical; native, simulated device; automated, simulated device) | Physical: 4 presses of the `projects.cycle` key gave 4 successful cycles across 3 projects, with wraparound (agent log). `hardware-test`: project switching SHOP → CMR. Native, simulated: cycle wraparound. Automated: `apps/agent/tests/agent.rs` `button_press_cycles_project_persists_and_notifies`; `apps/agent/tests/projects.rs` `concurrent_presses_advance_exactly_one_project_each`, `keys_show_the_project_after_a_cycle_a_reconnect_and_a_profile_change`; QA-1 cycle wraparound | Repeat with the signed build |
| 11 | Create and run a workflow | PASS (native, simulated device; physical, from a key; automated, simulated device) | Native, simulated: a workflow with a variable reference, Work Session (configure, start, verification, `readyToFinish`, finish) and execution history on the release build. Physical: the Work Session key ran the start workflow (state `working`), then the verification workflow (`readyToFinish`), then finished the session. Automated: `workflows_are_validated_and_keep_their_step_order`, `restarting_the_agent_never_ends_running_workflows_silently`, `apps/agent/tests/agent.rs` `runs_workflows_over_ipc`, `apps/agent/tests/workflow_e2e.rs`, `apps/agent/tests/mock_devices.rs`; QA-1 workflows (create, configure, references, execution, cancellation, history, error handling, redaction). The workflow was created over IPC, not by clicks in the workflow builder | Build a workflow by clicking in the signed app's workflow builder |
| 12 | Close the UI while the agent continues | PASS (native, release build) | Quitting the release UI left the agent running (re-parented to `launchd`); the simulated journey closed and reopened the UI with nothing lost. The whole physical `hardware-test` ran with the agent alone, with no app process. Automated: `apps/agent/tests/mock_devices.rs`; `a_restarted_app_reuses_the_unlocked_agent_without_asking` | Repeat with the signed build |
| 13 | Reopen the UI from the tray | NOT VERIFIED (tray click); ShowWindow path PASS (native, release build) | Native: the tray icon is in the menu bar, disappears when the agent stops and returns when it restarts. The `ShowWindow` request, sent over IPC with `deckpilot-dev show`, reopened the UI. A click on the tray icon itself was **not** done, because the session lacks the Accessibility permission. Automated: `apps/agent/tests/agent.rs` `requests_windows_from_the_app_instead_of_drawing_them` | Grant Accessibility to the test session, or click the tray icon by hand: with the window closed, open DeckPilot from the tray several times; the window opens each time |
| 14 | Restart the computer and use the app again | NOT VERIFIED | No real reboot was done. Related evidence: native, simulated, the configuration survived an agent restart (full persistence); physical, `hardware-test` restored layout, project CMR and brightness 60 % after an agent restart; with the agent stopped the UI showed "Stopped" with "Start agent", and after the agent was started directly (not by that button) the UI reconnected to Running by itself. Automated: `button_press_cycles_project_persists_and_notifies`, `apps/agent/tests/lifecycle.rs`, `workflow_e2e.rs` `a_run_interrupted_by_a_restart_is_recorded_and_new_runs_work`. Autostart at login and Keychain unlock after a reboot are not tested | Install the signed, notarized build and restart the Mac. The agent starts at login, the configuration opens (from the Keychain without a prompt, or after the password), and the profile, projects, variables, actions, key bindings and workflows are unchanged |
| 15 | Update DeckPilot without losing data | NOT VERIFIED | No update between two published releases has been installed by hand (no release is published yet). Related evidence: *Update continuity* passed on `v0.6.0-alpha.2` ([run 43606772522](https://github.com/knittingcode/DeckPilot/actions/runs/43606772522)): two Developer ID signed builds with the same code identity, a tampered update rejected, the signed update 0.5.0 → 0.5.1 installed, the configuration opened from the Keychain without a prompt before and after, and the data preserved and encrypted; database upgrades keep data (`crates/deckpilot-storage/tests/upgrade_from_0_5_0.rs`, `upgrade_from_0_6_0.rs`, `upgrade_work_sessions_from_schema_8.rs`, in check.sh); `tools/release-validation/update-test.sh` passed for 0.5.0 → 0.5.1 (release-validation.md, results for 0.5.0) | Install the 0.5.0 build (or the first 0.6.0 pre-release), configure it, then update through the app's updater to a signed newer build. The same profiles, projects, variables, actions, layouts and workflows are there and the Keychain opens without a prompt |

## Summary

| Status | Steps |
| ------ | ----- |
| PASS (physical) | 4, 9, 10, 11 (from a key) |
| PASS (native, release build, ad-hoc signed) | 2, 3, 12 |
| PASS (native, simulated device, driven over IPC) | 5, 6, 7, 8, 10, 11 |
| PASS (automated) | 3, 5, 6, 7, 8, 9, 10, 11 |
| Partial | 13: ShowWindow path PASS (native); tray click NOT VERIFIED |
| FAIL | none open; D1 failed step 9 on a fresh install and is fixed by #172 (merged, `c683489`) |
| NOT VERIFIED | 1, 13 (tray click), 14, 15 |

The checklist **does not pass yet**: steps 1, 14 and 15 need an install of
the signed and notarized 0.6.0 build, a real reboot and a signed update
installed by hand, and step 13 needs
a real click on the tray icon. Steps 5 to 8
and 11 were created over IPC, not by clicks in the window.

## Physical MK.2 (2026-11-13)

Stream Deck MK.2, serial `A00SA6062K8S7V`, on macOS aarch64. Firmware
version not recorded.

**Guided hardware test.** `deckpilot-dev hardware-test` with the debug agent
built from `5569a14`; the owner answered the visual questions. Report:

| Check | Result | Notes |
| ----- | ------ | ----- |
| Device discovery | passed | Stream Deck MK.2 A00SA6062K8S7V via elgato-hid |
| Model detection | passed | Stream Deck MK.2 |
| Layout applied | passed | profile A layout is active |
| Key images and text | passed | confirmed |
| Key press and release | passed | 15 keys, press and release each; the key showing each label reports that key |
| Brightness | passed | 10 %, 100 %, back to 60 % |
| Profile switching | passed | A → B → A |
| Project switching | passed | SHOP → CMR |
| Unplug and replug | passed | disconnected, reconnected with the same layout |
| Keys cleared on stop | passed | confirmed |
| Agent restart | passed | layout, project CMR and brightness 60 % restored |
| Agent without the configuration app | passed | the whole test ran with the agent alone (no app process) |
| Multiple devices | skipped | one device connected |

"Multiple devices" is skipped, so it is NOT VERIFIED.

**Real use.** On the release app (ad-hoc signed `make package` build of
`5569a14`) with the physical MK.2 and no simulated devices. The owner
pressed the keys; the agent log confirms each press:

- `projects.cycle`: 4 presses gave 4 successful cycles across 3 projects,
  with wraparound.
- Double click and long press each ran their own binding.
- Single click ran its binding twice in later attempts. In the first round
  one single click was **not** logged; it could not be reproduced on
  retry. Recorded as an open observation, not as a defect.
- Work Session key: the first click started the session (start workflow
  ran, state `working`), the next click ran the verification workflow
  (`readyToFinish`), and the next click finished the session.
- The owner saw the live timer on the key face.

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
