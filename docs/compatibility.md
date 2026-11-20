# Compatibility between app and agent versions

The configuration app and the background agent ship together and normally
run the same version. They can still differ for a while: right after an
update the previous agent may still run, an older app may be started
against a newer agent, and an agent may restart under an open app. This
page says what happens then and how it is tested.

## Versions involved

| Build | App/agent version | IPC protocol | Database schema |
| ----- | ----------------- | ------------ | --------------- |
| Tag `v0.5.0-2` (signed draft test build, never published) | 0.5.0-2 | 4 | 5 |
| Releases 0.6.0, 0.7.0, 0.8.0 and 0.8.1 | 0.6.0, 0.7.0, 0.8.0, 0.8.1 | 10 | 9 |
| Release 0.9.0 | 0.9.0 | 11 | 10 |
| Current main | see `Cargo.toml` | 16 | 13 |

Current main speaks protocol 16: the Device Editor's live preview
([editor-navigation-and-simulation.md](editor-navigation-and-simulation.md#3-live-preview-and-navigation-sync)).
Protocol 16 adds the `livePreview` capability: `beginLivePreview`,
`applyLivePreview`, `navigateLivePreview` and `endLivePreview`, with which
the app's editor has one device show and run its unsaved layout (kept in
the agent's memory only, never saved) and the two follow each other's
navigation; `livePreview` messages go only to the connection that owns the
session. `inspectMockDevice` reports a simulated device's frames and
navigation and, like `simulateInput`, is refused without mock devices.
`DeviceInfo` carries the optional `pageId`, which older apps ignore. The
client never sends the new requests to an agent below protocol 16 (refused
locally with `Unsupported`), and the agent refuses them from a connection of
an older protocol; the app then says that changes reach the device when
saved, as before. Nothing an older app or agent sends or receives changes,
and no schema changes.

Before it, protocol 15: restoring DeckPilot's own plugins.
Protocol 15 adds the `restoreBundled` capability (and the plugin runtime
feature `restoreBundled`): the `restoreBundledPlugin` request installs a
bundled first-party plugin the user uninstalled again from the
application bundle, verified against the pins compiled into the agent (the
user's tombstone is cleared), or reinstalls one whose installed copy is
broken, and answers the plugin's details; `listKnownPlugins` marks the
removed plugins the agent can restore with the optional `restorable`,
which older apps ignore. The client never sends the request to an agent
below protocol 15 (a raw request is refused locally with `Unsupported`,
which the app reports as `agentTooOld`), and the agent refuses it from a
connection of an older protocol. Nothing an older app or agent sends or
receives changes, and no schema changes: the restore uses the store's
existing install, uninstall and tombstone records.

Before it, protocol 14 and plugin API 1.7: the Device
Editor's interactive preview
([editor-navigation-and-simulation.md](editor-navigation-and-simulation.md)).
Protocol 14 adds the `simulateInput` capability: the `simulatePress`
request, with which the app presses a control of a connected device through
the agent's own gesture engine and dispatch (answered `simulatedPress`).
The client never sends it to an agent below protocol 14 (a raw request is
refused locally with `Unsupported`, which the app reports as the refusal
`unsupported`), and the agent refuses it from a connection of an older
protocol; nothing an older app or agent sends or receives changes. The
older, mock-only `simulateInput` request is unchanged. Workflows a
simulated press starts are recorded with the execution trigger `simulated`
and the workflow cause `simulated`, which older readers see as `other`
(both enums have a catch-all). Action types may carry the optional
`editorNavigation` and preview faces the optional `navigation`, fields older
apps ignore; neither needs the new protocol. Plugin API 1.7 adds the
manifest's `contributes.actions[].editorNavigation` (no ABI or SDK
change): a manifest with it needs `^1.7` (the validator refuses it below),
so an agent of plugin API 1.6 reports such a version `incompatible`;
manifests without it are unchanged.

Key title appearance (unreleased): a key's appearance may carry
`fontFamily`, `bold`, `italic` and `underline`, optional fields that older
agents and apps ignore (an older app drops them when it saves the layout);
no document version, migration or IPC protocol change. Action types may
carry the optional `title` (older apps ignore it). Plugin API 1.8 adds the
manifest's `contributes.actions[].title` and `contributes.visualAssets`
(no ABI or SDK change): a manifest with either needs `^1.8`, so an agent of
plugin API 1.7 reports such a version `incompatible`; manifests without
them are unchanged ([key-text.md](key-text.md#plugin-titles),
[marketplace/sdk/manifest.md](marketplace/sdk/manifest.md#contributesvisualassets)).

Shared visual assets (unreleased,
[key-text.md](key-text.md#icons-and-backgrounds)): a key's `icon.id` may be
a library asset id (`deckpilot.<name>`, `<plugin>.<id>`) besides a profile
image's UUID, and its `background` may be `asset:<ref>` besides `#rrggbb`
and `transparent`. Stored values of before read unchanged; no document
version and no migration. An older app or agent refuses a layout that uses
one of the new values (as for any unknown value), so such layouts need this
version on both sides. No IPC protocol change (it stays 16): plugin
summaries carry the optional `visualAssets` (older apps ignore it), and
`getPluginIcon` the optional `asset`, which the app sends only to an agent
whose plugin runtime lists the feature `visualAssets` (an older agent would
answer with the plugin's own icon).

Plugin API 1.9 (unreleased,
[marketplace/module-migration.md §7](marketplace/module-migration.md#plugin-api-19-security-reviews-2026-11-20)):
additive. The manifest kinds `tools`, `resources`, `projectContext` and
`hostViews`, `openUrl.fromParams`, `resource` parameters, the view kinds
`workflows` and `projects`, and `contributes.statusItems` and
`contributes.dashboardCards` validate only with `^1.9` and only for a
first-party plugin shipped inside the app; every other manifest is refused
them, and a `^1.8` manifest without them validates exactly as before. The
capability operations `resource.resolve`, `resource.open`, `tool.run`,
`url.openParam`, `project.current`, `project.list`, `project.select`,
`project.cycle` and `profile.read` are compatible additions within ABI 1
(a guest that does not use them never sends them; an older agent refuses
them as unknown). This agent reports plugin API 1.9, carries out `resource`
parameters and `resource.resolve`, and answers the other operations as not
implemented until their host side lands; an agent of 1.8
reports a `^1.9` version `incompatible`. The SDK's `LATEST_API_LEVEL` is
1.9. An older app ignores a navigation entry of an unknown view kind. No
IPC protocol or document change.

Before it, protocol 13 and plugin API 1.6: localization
([i18n.md](i18n.md#plugin-localization),
[marketplace/sdk/localization.md](marketplace/sdk/localization.md)).
Protocol 13 adds the `locale` capability: the `setLocale` request, with
which the app tells the agent its effective language (a tag such as `pl`),
and the optional `locale` field of the status. The client never sends
`setLocale` to an agent below protocol 13 (`AgentClient::set_locale`
returns without sending, a raw request is refused locally with
`Unsupported`), and the agent refuses it from a connection of an older
protocol; such an agent, and every agent before an app said anything,
draws its texts in English and shows plugin texts in the plugin's
fallback chain from English. After a change the agent announces the
plugins whose manifests name translations with the existing `changed`
plugin events (protocol 10), which older apps already reload on. The
plugin runtime lists the feature `locale` in `getPluginRuntime`. Plugin API
1.6 adds the manifest's `locales`, `defaultLocale` and `t:` texts and the
capability operations `locale.get` and `locale.translate` (compatible
additions to ABI 1): a manifest with `locales` or `defaultLocale` needs
`^1.6` (the validator refuses it below), so an agent of plugin API 1.5
reports such a version `incompatible`; below 1.6 a `t:` text is plain text,
so older manifests are unchanged. An agent before 1.6 answers the new
operations `Unsupported`.

Before it, protocol 12 and plugin API 1.5: plugin
dialogs (`dialog.open`, the `dialogs` permission, [dialogs.md](dialogs.md)).
Protocol 12 adds the `pluginDialogs` capability: `dialogOpened` and
`dialogClosed` plugin events, sent only to a connection of protocol 12 that
announced the feature (`subscribePluginEvents { features: ["pluginDialogs"] }`),
and the `answerPluginDialog` request. A protocol 11 app never announces it,
so it receives none of them, and an agent of protocol 11 ignores the
announcement; nothing an 0.9.0 app or agent receives changes. A plugin that
asks for a dialog while no announcing app is connected gets `unavailable`;
a manifest with the `dialogs` permission needs `^1.5`, so a 0.9.0 agent
(1.4) reports it `incompatible`.

Besides these, 0.9.0 provides plugin API 1.4 (0.8.0 and 0.8.1 provide
1.3): a view's `context` and a manifest's `dependencies` and
`requiresDependencies` need it, and older agents refuse a manifest that
uses them; manifests without them install as before. The plugin store
(`plugins.db`, separate from the configuration database) goes from
schema 4 to 5, an additive table for dependencies, after an encrypted
backup in `plugins/.backups/`; a 0.8.1 agent refuses the upgraded file
without writing to it.

Schema 10 moves layouts, action instances and icons into shared tables
and rewrites the profile documents (document version 6); see
[Shared layouts](data-model.md#shared-layouts). Shared layouts need no
protocol of their own (protocol 11 is the live key states of the device
editor): the agent reports the layout each device shows in the existing
`DeviceInfo.layout` and the optional `DeviceInfo.previousLayout`
(older clients ignore it), and *Switch layout* and *Back to previous layout* are
action types of the `deck` module, which need no new request. An app or
agent of 0.6.0–0.8.1 refuses a schema 10 database before writing anything
(“schema version 10 is newer than supported version 9”), as earlier
releases refused newer schemas; going back means restoring the
`deckpilot-schema9-…` backup. A 0.8.1 agent that still had the database
open when a newer app upgraded it would read profile documents of version
6, which it refuses, instead of seeing profiles without layouts. Exports
without layouts of no profile stay payload version 2, which 0.6.0–0.8.1
import; an export of everything with such layouts is version 3, which
they refuse.

Schema 11 gives folder pages their protected Back key and Auto Exit
([The protected Back key](data-model.md#the-protected-back-key)): layout
documents gain the optional page fields `back` and `autoExit` (item
document version 1 is unchanged; the fields are additive) and migration 11
rewrites stored layouts after the usual encrypted backup
(`deckpilot-schema10-…`). A 0.9.0 app or agent refuses a schema 11 database
before writing anything (“schema version 11 is newer than supported
version 10”); going back means restoring that backup. No IPC change: the
reserved Back and Auto Exit are the agent's own, the move is reported in
the existing device status, and preview faces mark the key with an optional
`navigation.system` field, which older callers ignore. Exports carry the
fields; a 0.9.0 import ignores them (its folders get the automatic Back on
the first free key, as before), and importing an older export here applies
the migration rule. Plugin API stays 1.7.

Schema 12 makes folder keys core navigation controls
([Folder controls](data-model.md#folder-controls)): layout documents gain
the optional `recovered` list (item document version 1 is unchanged; the
field is additive) and migration 12 rewrites stored layouts after the usual
encrypted backup (`deckpilot-schema11-…`). An app or agent of schema 11
refuses a schema 12 database before writing anything (“schema version 12
is newer than supported version 11”); going back means restoring that
backup. No IPC change (protocol and plugin API stay as they are): folder
keys are decided from the bindings the agent already reads, and
`save_layout` refusals use new `invalid.folderControl.*` codes that older
apps show by their English message. Exports carry `recovered`; an older
import ignores it (the recovered bindings are lost there, the folder keys
work). Importing an older export here normalizes its folder keys with a
warning.

Schema 13 makes a folder's name the one title of its key
([key-text.md](key-text.md#folder-names)): migration 13 (after the usual
encrypted backup, `deckpilot-schema12-…`) drops a folder key's own title
equal to the folder's name and moves one that differs to the layout's
`recovered` list with the reason `folderLabel` and a migration note; shared
layouts are rewritten once, layouts without such titles stay byte for byte
the same. An app or agent of schema 12 refuses a schema 13 database before
writing anything; going back means restoring that backup. Imports and
saving apply the same rule (an import says so in a warning). No IPC change.
An older import of a new export reads `folderLabel` as an unknown reason
and refuses the layout, as with any newer value.

Text appearances of key titles (wrap, single line, auto fit, marquee;
[key-text.md](key-text.md)) are an optional `text` field of a binding's
appearance: no schema, document, payload or protocol change. A 0.9.0 agent
ignores the field and draws the title wrapped; a 0.9.0 app drops it when it
saves that layout. The app's layout preview reports how titles come out in
an optional `text` field of each control's face, which older callers
ignore.

DeckPilot 0.5.0 was never published. Its two draft prereleases (0.5.0-1
and 0.5.0-2) have since been deleted; only the tag `v0.5.0-2` remains, as
the compatibility baseline that `tools/compat/matrix.sh` builds. "0.5.0" in
this page means that build.

## Protocol negotiation

- The app connects with the newest protocol it speaks. An agent before
  protocol 6 refuses a newer `hello` (`unsupportedProtocol`), so the client
  steps down one version at a time until one is accepted, down to protocol
  4 (`MIN_COMPATIBLE_AGENT_PROTOCOL`). The connection then uses the lower of
  the two protocols.
- What a connection can do is a list of **capabilities** derived from that
  protocol (`Capability::of_protocol`): `workflows` and `prepareForUpdate`
  (2), `moduleNamespaces` (3), `encryptedDatabase` (4),
  `backgroundExecutions` (5), `notifications` (6), `variables` (7),
  `workSession` (8), `variableRegistry` (9), `plugins` (10), `keyStates`
  (11), `pluginDialogs` (12), `locale` (13), `simulateInput` (14),
  `restoreBundled` (15), `livePreview` (16). Agents
  from protocol 6
  also list them in `welcome`; the client never credits an agent with more
  than its protocol has.
- **No unsupported requests.** Every request has the protocol that
  introduced it (`Request::since_protocol`). The client refuses a newer
  request before sending it (`ClientError::Unsupported`, "the running agent
  (0.5.0-2) is too old for `listExecutions` … restart the agent"), and the
  agent refuses one sent anyway (`unsupported`). Events newer than a
  client's protocol are not sent to it.
- Agents before protocol 4 (DeckPilot 0.3 and earlier) cannot be used at
  all: the app reports **update required** and can only stop them (with
  protocol 1) and start its own agent.

## Compatibility states in the app

| State | When | What the app does |
| ----- | ---- | ----------------- |
| compatible | same version | everything available |
| agent outdated | agent older (e.g. after an update) | works with the negotiated protocol; names the features that are unavailable until the agent is restarted; offers the restart |
| agent newer | agent newer than the app | same, from the app's side; the app's own (older) agent must not be started against the newer database (it refuses it, see below) |
| update required | agent below protocol 4 | nothing configurable; offers to replace the agent |
| unknown | a version cannot be parsed | offers the restart |
| other distribution | the agent was delivered by another distribution channel than the app, for example an APT agent next to a GitHub app (rule R6 of [distribution/README.md](distribution/README.md#5-cross-channel-rejection-rules)) | says which installation each belongs to; offers the restart, which replaces it with the agent of the app's own bundle. Checked before the version |
| stopped | no agent | offers to start it; reconnects automatically when it runs again |

The channel comes from the agent's distribution record, an optional
`distribution` field in its status (`getStatus`, protocol 10 unchanged):
the channel, update provider, package format where the build knows it,
release channel, version, platform and architecture, all compiled into
the agent. Older apps ignore the field; agents of 0.6.0-alpha.3 and
earlier do not send it and were all GitHub builds, so the app treats them
as such and checks only their version, as before. A channel name the app
does not know counts as another channel.

The app keeps a subscription to the agent and reconnects every 2 s after
the agent stops or restarts; each reconnection negotiates again, so an
agent that was replaced by another version while the app stayed open is
recognised with its new version, protocol and capabilities.

## Matrix

"Real" means the actual binaries of both versions; "simulated" means a test
peer that behaves like the other version, using messages recorded from the
real binary (`crates/deckpilot-ipc/tests/fixtures/`). Results marked
*measured before 0.6.0-alpha.2* were recorded when main spoke protocol 6
with schema 7 and have not been recorded again for protocol 9 and schema 9.

| Case | How it is tested | Result |
| ---- | ---------------- | ------ |
| 0.5.0 app/client with 0.5.0 agent | real (`tools/compat/matrix.sh`) | works (protocol 4) |
| 0.6.0 client with 0.6.0 agent | real (matrix), agent and desktop tests | works (protocol 6; measured before 0.6.0-alpha.2) |
| 0.6.0 client with 0.5.0 agent | real (matrix: `real_agent` test against the running 0.5.0-2 agent); simulated (`peers` tests) | negotiates protocol 4 after refused `hello` 6 and 5; status read; `listExecutions` refused locally and never sent (measured before 0.6.0-alpha.2) |
| 0.5.0 client with 0.6.0 agent | real (matrix: the 0.5.0-2 CLI client); agent test with recorded protocol 4 messages | works with protocol 4; `welcome` lists protocol 4's capabilities; protocol 5 requests refused with `unsupported` |
| 0.8.1 agent given a schema 10 database | real (`tools/compat/matrix.sh v0.8.1`) | refused before anything is written; the database and key file stay byte-identical; this build still opens it |
| this build given a 0.8.1 database | real (matrix: database created by the 0.8.1 tools); storage tests on the `v0.8.1` fixture | upgraded to schema 10; profiles, layouts in order, bindings, active layouts, actions, icons, active profile and project kept |
| 0.5.0 agent given a 0.6.0 database | real (matrix) | refused ("schema version 7 is newer than supported version 5"); database and key file byte-identical afterwards; the 0.6.0 agent still opens it (measured before 0.6.0-alpha.2) |
| 0.6.0 agent given a 0.5.0 database | real (matrix: database created by the 0.5.0-2 tools) | upgraded to schema 7; profiles, active profile and project kept (measured before 0.6.0-alpha.2) |
| agent before protocol 4 | simulated (`peers`: a protocol 3 peer) | update required; stopping it with protocol 1 still works |
| protocol 5 agent | simulated (`peers`) | background executions without notifications |
| protocol 9 agent (before plugins) | simulated (`peers`) | every plugin request refused locally (`Unsupported`, needs protocol 10) and never sent |
| protocol 10 agent without plugins (any release build) | agent tests (`plugins_e2e`, `with_plugins_disabled_nothing_is_available`) | `getPluginRuntime` says why; every other plugin request fails with `unsupported` and reason `pluginsUnavailable`; plugin events are a separate message sent only after `subscribePluginEvents`, so older clients never see them |
| protocol 11 app or agent with a protocol 12 peer (plugin dialogs) | agent test `plugin_dialogs_reach_only_connections_that_announced_them`; IPC test `plugin_dialogs_are_protocol_12_and_announced` | the client leaves the announcement out below protocol 12 and the agent ignores it there; `answerPluginDialog` refused (`unsupported`) locally and by the agent; no dialog event reaches a protocol 11 connection or one that did not announce; with no announcing app a plugin's dialog is `unavailable` |
| protocol 12 app or agent with a protocol 13 peer (the app's language) | IPC test `a_protocol_12_agent_is_never_told_the_locale`; agent test `a_protocol_12_app_never_sets_the_language` (`tests/locale.rs`); IPC test `the_locale_is_protocol_13_and_only_a_tag` | the client does not send `setLocale` below protocol 13 and refuses it locally; the agent refuses it from an older connection; the status of an older agent has no `locale`; keys and plugin texts stay English; everything else works as before |
| protocol 13 app or agent with a protocol 14 peer (simulated presses) | IPC test `a_protocol_13_agent_is_never_asked_to_simulate_a_press`; agent test `a_protocol_13_app_cannot_simulate` (`tests/simulate_press.rs`); IPC test `simulated_presses_are_protocol_14`; desktop test `simulated_presses_answer_ran_confirmation_or_refused` | the client does not send `simulatePress` below protocol 14 and refuses it locally (the app answers `refused` with code `unsupported`); the agent refuses it from an older connection; nothing is pressed; everything else works as before |
| protocol 15 app or agent with a protocol 16 peer (live preview) | IPC test `a_protocol_15_agent_is_never_asked_for_a_live_preview`; agent test `a_protocol_15_connection_cannot_preview` (`tests/live_preview.rs`); IPC test `live_preview_is_protocol_16`; frontend `use-live-preview.test.tsx` | the client does not send the live preview requests below protocol 16 and refuses them locally; the agent refuses them from an older connection; the editor says changes reach the device when saved; everything else works as before |
| agent temporarily unavailable | desktop tests (`stopped` state), connect timeouts (`deckpilot-ipc`) | stopped state; no request hangs |
| agent restarted during a running workflow | agent test `runs_of_a_stopped_agent_are_recorded_as_interrupted` | run recorded as interrupted; history kept |
| agent version changed while the app stays open | bridge reconnect (negotiates again); simulated peers per version | covered by the same negotiation; not tested end to end with a running app window |

Run the real matrix locally with `tools/compat/matrix.sh [<old tag>] [<work dir>]`;
CI runs it on Linux for changes to the IPC, storage or agent code
(`.github/workflows/compatibility.yml`). It uses throwaway data protected by
the development password, never a credential store.

## Lifecycle

| Situation | Behaviour | Evidence |
| --------- | --------- | -------- |
| Fresh installation | database created by the agent with a system key; empty Action Library | agent/desktop tests (`a_fresh_install_…`, `fresh_installation_has_an_empty_action_library`) |
| Agent already running | a second agent exits (instance lock) | `single_instance_shutdown_and_client_reconnect` |
| App closed while the agent runs | the agent and its runs continue | agent runs are independent of clients (`device_bound_workflows_run_in_the_background`) |
| App reopened | reuses the unlocked agent | `a_restarted_app_reuses_the_unlocked_agent_without_asking` |
| Agent crash or stop during a run | runs marked interrupted on stop and on the next start | `runs_of_a_stopped_agent_are_recorded_as_interrupted` |
| Agent restart from the app | refused while workflows run; with confirmation the runs are interrupted and recorded | `restarting_the_agent_never_ends_running_workflows_silently` |
| App/agent update | installation waits while workflows run (automatic installation retries later; manual installation says why); the intent is recorded, the old agent's endpoint and instance lock are both gone before anything is installed, and the next start completes or reports the update ([updates.md](updates.md#recovery-after-an-interrupted-update)) | `InstallContext::agent_busy` from `runningExecutions`; policy tests; `updates_tests.rs` (`install`, `recovery`) |
| Agent of another installation | refused (other distribution); the restart starts this app's agent only once the other one released the instance lock | `deckpilot-update` compatibility tests; `agent.rs` tests |
| IPC endpoint conflicts | endpoint per data directory; bounded connect | `deckpilot-ipc` endpoint tests |
| Incompatible pair | old agent refuses the newer database without changing it | matrix |
| System restart, logout/login | agent started by autostart; database opens with the system key | manual (release validation); not automated |
| Multiple app windows | one app process; module windows by identifier | `native_windows_are_opened_by_identifier_and_reused` |

An update never ends a running workflow silently: it waits, or the user
cancels the runs, which the history records as cancelled or interrupted.
