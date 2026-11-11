# Compatibility between app and agent versions

The configuration app and the background agent ship together and normally
run the same version. They can still differ for a while: right after an
update the previous agent may still run, an older app may be started
against a newer agent, and an agent may restart under an open app. This
page says what happens then and how it is tested.

## Versions involved

| Build | App/agent version | IPC protocol | Database schema |
| ----- | ----------------- | ------------ | --------------- |
| Draft prereleases `v0.5.0-1`, `v0.5.0-2` (signed test builds, never published) | 0.5.0-1, 0.5.0-2 | 4 | 5 |
| Current main (becomes 0.6.0) | see `Cargo.toml` | 6 | 7 |

DeckPilot 0.5.0 was never published: it exists only as the two draft
prereleases above. "0.5.0" in this page means those builds.

## Protocol negotiation

- The app connects with the newest protocol it speaks. An agent before
  protocol 6 refuses a newer `hello` (`unsupportedProtocol`), so the client
  steps down one version at a time until one is accepted, down to protocol
  4 (`MIN_COMPATIBLE_AGENT_PROTOCOL`). The connection then uses the lower of
  the two protocols.
- What a connection can do is a list of **capabilities** derived from that
  protocol (`Capability::of_protocol`): `workflows` and `prepareForUpdate`
  (2), `moduleNamespaces` (3), `encryptedDatabase` (4),
  `backgroundExecutions` (5), `notifications` (6). Agents from protocol 6
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
| stopped | no agent | offers to start it; reconnects automatically when it runs again |

The app keeps a subscription to the agent and reconnects every 2 s after
the agent stops or restarts; each reconnection negotiates again, so an
agent that was replaced by another version while the app stayed open is
recognised with its new version, protocol and capabilities.

## Matrix

"Real" means the actual binaries of both versions; "simulated" means a test
peer that behaves like the other version, using messages recorded from the
real binary (`crates/deckpilot-ipc/tests/fixtures/`).

| Case | How it is tested | Result |
| ---- | ---------------- | ------ |
| 0.5.0 app/client with 0.5.0 agent | real (`tools/compat/matrix.sh`) | works (protocol 4) |
| 0.6.0 client with 0.6.0 agent | real (matrix), agent and desktop tests | works (protocol 6) |
| 0.6.0 client with 0.5.0 agent | real (matrix: `real_agent` test against the running 0.5.0-2 agent); simulated (`peers` tests) | negotiates protocol 4 after refused `hello` 6 and 5; status read; `listExecutions` refused locally and never sent |
| 0.5.0 client with 0.6.0 agent | real (matrix: the 0.5.0-2 CLI client); agent test with recorded protocol 4 messages | works with protocol 4; `welcome` lists protocol 4's capabilities; protocol 5 requests refused with `unsupported` |
| 0.5.0 agent given a 0.6.0 database | real (matrix) | refused ("schema version 7 is newer than supported version 5"); database and key file byte-identical afterwards; the 0.6.0 agent still opens it |
| 0.6.0 agent given a 0.5.0 database | real (matrix: database created by the 0.5.0-2 tools) | upgraded to schema 7; profiles, active profile and project kept |
| agent before protocol 4 | simulated (`peers`: a protocol 3 peer) | update required; stopping it with protocol 1 still works |
| protocol 5 agent | simulated (`peers`) | background executions without notifications |
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
| App/agent update | installation waits while workflows run (automatic installation retries later; manual installation says why) | `InstallContext::agent_busy` from `runningExecutions`; policy tests |
| IPC endpoint conflicts | endpoint per data directory; bounded connect | `deckpilot-ipc` endpoint tests |
| Incompatible pair | old agent refuses the newer database without changing it | matrix |
| System restart, logout/login | agent started by autostart; database opens with the system key | manual (release validation); not automated |
| Multiple app windows | one app process; module windows by identifier | `native_windows_are_opened_by_identifier_and_reused` |

An update never ends a running workflow silently: it waits, or the user
cancels the runs, which the history records as cancelled or interrupted.
