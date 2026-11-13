# Key gestures, stateful keys and work sessions

Status: **design and contract**. The types in
`crates/deckpilot-core/src/gesture.rs` and `crates/deckpilot-core/src/work_session.rs`
(plus `WorkSessionId` and `VerificationId` in `id.rs`) and their mirror at the
end of `apps/desktop/src/lib/types.ts` exist and are tested. Nothing uses them
yet, so behaviour is unchanged until the workstreams in section 12 land.

This document builds on `docs/action-architecture.md`, which covers action
types and instances, shared variables, the resolution pipeline and protocol 7.
Where the two touch, that document wins and this one names the change it
needs.

## 1. Audit of the current code (0.6.0 + open PRs)

| Area | Where | What exists |
|---|---|---|
| HID input | `crates/deckpilot-hid/src/lib.rs` (MK.2 through `elgato-streamdeck`), `deckpilot-device/src/backend.rs` `KeyStateTracker` | The MK.2 reports the full state of all keys. `KeyStateTracker::update` turns each report into **separate `ControlInput::Press` and `ControlInput::Release` transitions per key**, and drops repeated reports of an unchanged state. **So press and release can be told apart today**, for every key, as edges. Events carry **no timestamp**. A fresh tracker is used for each connection. The held #55 (own MK.2 wire protocol) uses the same tracker and gives the same semantics, so nothing here depends on it. #55 stays held. |
| Device model | `deckpilot-device/src/{control,capabilities,model}.rs` | `ControlId::{Key, Encoder, TouchZone}`, `ControlInput::{Press, Release, Rotate, Touch}`. `validate_input` accepts press and release for keys and pressable encoders. Only the MK.2 has a driver (15 keys). XL and Plus are descriptors only. `TouchGesture` is the touch-strip vocabulary and is unrelated to key gestures. |
| Reader thread | `apps/agent/src/devices.rs` | One thread per opened device blocks in `read_input` (100 ms timeout) and sends `DeviceSignal::Input { session, device, control, input }` over an unbounded tokio channel. `session` identifies the connection, and signals from an earlier connection are dropped. A disconnect arrives as `DeviceSignal::Closed`. The thread never runs actions. |
| Dispatch | `apps/agent/src/service.rs::handle_signal`, `crates/deckpilot-core/src/device_runtime.rs::handle_event` | Runs on the single agent service task (`&mut self`). It finds the first binding with `trigger.matches(input)` on the current page. **Action** bindings run **synchronously on the service task** (`Runtime::run_action`). **Workflow** bindings return `WorkflowRequested`, which the agent starts in the background (`start_execution`, at most 4 runs). Navigation is per device (`navigation`, `parked`). While locked, a press only prompts for unlock. |
| Bindings | `crates/deckpilot-core/src/layout.rs` | `Trigger::{Press, Release, RotateClockwise, RotateCounterClockwise, Touch}`, `ControlBinding { control, trigger, target, label, appearance }`, `BindingTarget::{Action, Workflow, OpenPage, Back}` (`Copy`). Validation rejects duplicate `(control, trigger)` pairs and triggers a control cannot produce. Every generated and seeded binding uses `Press`, so **existing "single click" = fire on the press edge, without delay**. The automatic Back key reacts to `Press`. Faces prefer the `Press` binding. |
| Dynamic key state (#33) | `crates/deckpilot-core/src/button_state.rs`, `apps/agent/src/service/states.rs` | A per-action-type provider (`ActionHandler::state_policy`/`button_state`) runs off the loop on threads (at most 4), with at least 1 s between calls (`MIN_REFRESH_INTERVAL`), only for keys that are shown. Results are cached per `(action, project)`. The run loop sleeps until the next state is due (`sleep_until(state_due)`), so nothing polls. **There is no agent-internal state source.** Every state belongs to an action instance. |
| Workflows, typed outputs | `crates/deckpilot-core/src/graph/**`, `workflow.rs`, `runtime.rs` | `Workflow.outputs: Vec<WorkflowOutput>` with `ValueKind::{Text, Integer, Boolean, Path, Url, ProjectKey}` and `sensitive`. Values are strings. `runtime.rs::workflow_outputs(graph, report)` collects declared outputs, but **only for nested runs**. A top-level run's `GraphReport` (handled in `service/executions.rs::finish_execution`) has no output map. `ExecutionTrigger` has `#[serde(other)] Other`, so a new trigger kind is safe for older readers. |
| Shared variables | `crates/deckpilot-core/src/variables/**` (#92), A's #94, E's #97, F's #95/#96 (open) | The roots are a closed set: `steps`, `shared`, `system`, `workflow`. `SystemVariable` is a closed enum with fixed paths. **`work.session.id` does not parse**, because `work` is not a root. Resolution goes through an injected `VariableResolver`, and values are resolved at execution time, never cached. |
| Persistence | `crates/deckpilot-storage/src/sqlite.rs` | `main` is at **schema 7**. **E's #97 (schema 8, `DOCUMENT_VERSION` 5) is still open.** Every migration step runs after an encrypted, verified backup, in one transaction, and is recorded in `migration_history`. Older binaries refuse newer schemas. Modules can also own tables through `ModuleMigration`, versioned in `module_schemas`, with no backup and outside the core version. Timestamps are wall-clock Unix seconds or ms. The agent's wall clock can be injected (`AgentOptions::clock`). |
| Sleep, lock | `apps/agent/src/service/security.rs::check_auto_lock` | Sleep is detected by a wall-clock gap between checks (`auto_lock_check + SLEEP_GAP`). **Auto-lock can lock the database on sleep or idle.** While locked, the agent can neither read nor write it, and `start_execution` refuses. |
| Modules, permissions | `crates/deckpilot-core/src/module.rs`, `registry.rs` | `FeatureModule` provides a manifest (pages, config, capabilities, migrations, windows) and handlers. A handler can change agent state only through `ContextEffect`, which the runtime applies after a capability check. |
| IPC | `crates/deckpilot-ipc/src/protocol.rs` | `PROTOCOL_VERSION = 6` on `main`, and A takes **7**. `Capability::since_protocol` and `Request/Event::since_protocol` gate features, and the agent sends no event a client's negotiated protocol does not know. |

Consequences:

1. Gesture timing must be stamped where the input is read. The service task
   can be busy for the length of a synchronous action, so measuring there
   would make a slow action turn a click into a long press.
2. Gestures are a layer on top of the press and release edges, not a driver
   change. Nothing in `deckpilot-hid` changes.
3. The work session needs an agent-internal state source for key faces, and
   the top-level workflow outputs must be read at the end of a run.
4. Schema numbers: E's schema 8 is not merged, so the work-session tables
   are **schema 9**, appended after E's step (section 6.4).

## 2. Gesture engine

### 2.1 Where it runs

```
device thread ──(Press/Release + at: Instant)──▶ agent service task
                                                  │ GestureEngine (pure, core)
                                                  │   on_input / advance(now) / cancel
                                                  ▼
                                         GestureFired ─▶ Runtime dispatch
                                                          ├ action: as today
                                                          └ workflow: background run
```

- **Device thread**: unchanged, except that `DeviceSignal::Input` gains
  `at: std::time::Instant`, taken right after `read_input` returns. It
  still never runs anything.
- **`GestureEngine`** (`crates/deckpilot-core/src/gesture/engine.rs`, new) is
  a pure state machine that is never given a clock. Every call carries the
  time: `on_input(at, device, control, input, &KeyContext)`,
  `advance(now)`, `cancel(device, GestureCancel)`, `next_deadline()`. It
  returns the gestures that fired. Tests drive it with synthetic `Instant`s
  (`base + Duration`), so they are exact and deterministic.
- **Agent** (`apps/agent/src/service/gestures.rs`, new) owns the engine.
  The run loop's `select!` gets one more arm,
  `sleep_until(engine.next_deadline())`, like `state_due`. When it wakes,
  the agent first takes the pending device signals off the channel
  (`try_recv`) and feeds them **in timestamp order**, then calls
  `advance(now)`. A late wake-up therefore never reorders a release and a
  deadline.
- **Bounded**: at most one pending deadline per `(device, connection, key)`.
  An entry is removed when it returns to idle. Nothing spawns per press.

### 2.2 Key modes

Each control on a page is in one of two modes, decided by its bindings:

- **Raw mode**: only `Press`/`Release` (or rotate and touch) triggers. The
  engine passes the edges through unchanged, exactly as today, with no
  delay. These are the *low-level press and release* bindings.
- **Gesture mode**: at least one of the new triggers `Click`, `DoubleClick`
  or `LongPress` (section 3). The engine resolves each sequence to at most
  one gesture. A page that mixes raw and gesture triggers on one control is
  invalid (`LayoutIssue::MixedTriggers`), so "exactly one gesture per input
  sequence" holds without exceptions.

The automatic Back key stays raw (press edge).

### 2.3 Per-key state machine

The key is `(DeviceId, connection session, ControlId)`, so devices are
independent and a reconnect never continues an old sequence. When the
first press of a sequence arrives, the engine captures a `KeyContext`: the
`BoundGestures` (`gesture.rs`), the `GestureTimings`, the page and the
layout generation.

| State | Event | Condition | Fires | Next |
|---|---|---|---|---|
| Idle | press(t) | only click bound (`ClickTiming::OnPress`) | Click | Swallow |
| Idle | press(t) | otherwise | – | Down1 { since t, deadline t+long if long bound } |
| Down1 | deadline (long) | long bound | LongPress | Swallow |
| Down1 | release(t) | double bound | – | Wait { deadline t+double } |
| Down1 | release(t) | double not bound | Click (if bound) | Idle |
| Wait | press(t) | t ≤ deadline | DoubleClick | Swallow |
| Wait | deadline | – | Click (if bound) | Idle |
| Swallow | release | – | – | Idle |
| Swallow | press | (cannot happen: a press needs a release first) | – | Swallow |
| any | cancel(reason) | – | – | Idle, or Swallow while the key is down |

Rules this encodes:

- **Defaults**: double click 300 ms, measured from the *first release* to
  the *second press*. Long press 700 ms of continuous hold. Allowed ranges:
  double 150–1000 ms and long 300–5000 ms (`GestureTimings::validate`).
- **Single click is delayed only as far as needed** (`ClickTiming`). With
  only a click bound, it fires on the press, which matches today's latency.
  With long press bound, it fires on a release before the threshold. Only
  with double click bound does it wait for the window.
- **Long press fires once per hold**, at the threshold while the key is
  still down. The release after it is swallowed and never clicks. Holding
  longer never repeats it.
- **Double click fires on the second press**, so it does not wait for the
  second release. The second release is swallowed. If the second press is
  held past the long threshold, nothing more happens.
- If long press is not bound, a long hold followed by a release is a click.
- **Exactly one gesture per sequence**: each sequence ends in `Idle` after at
  most one `Fires`. This is tested as a property over random sequences.

### 2.4 Invalidation

`cancel(device, reason)` (`GestureCancel`) drops pending gestures. A key that
is still held goes to `Swallow`, so its release cannot click on the new page:

- **Disconnected**: on `DeviceSignal::Closed` or `DeviceEvent::Disconnected`,
  every key of that connection is forgotten. The new connection starts
  clean.
- **PageChanged**: the device navigated (including when a gesture itself
  opened a page).
- **ProfileChanged**: `set_profile`, `reload_profile` (the generation
  changes).
- **Locked**: the database locked.

A gesture that fires checks that the page and generation still match the
captured `KeyContext` before it is dispatched, which catches stale state.

### 2.5 Dispatch

`GestureFired { device, control, gesture, page, generation, pressed_at }`
goes to `Runtime::dispatch_gesture` (C/G2), which finds the binding for
`(control, Trigger::from(gesture))` and runs its target through today's
`dispatch`. Actions still run where they run today. Workflows run in the
background. Neither runs on a device thread (tested by thread name).
Moving synchronous actions off the service task is out of scope. The
`at` stamps keep gesture timing correct while an action blocks the task.

## 3. Gesture bindings

### 3.1 Data model (profile document, layout pages)

```jsonc
// Trigger gains three variants (layout.rs, C's file; workstream G2 with C)
{ "type": "click" } | { "type": "doubleClick" } | { "type": "longPress" }

// LayoutPage gains per-key timings (KeyGestureSettings, contract)
"gestures": [ { "control": {"kind":"key","index":3}, "longPressMs": 1200 } ]
```

- One `ControlBinding` per `(control, trigger)`, as today. Click, double
  click and long press are independent bindings with independent targets,
  labels are unchanged, and the face uses the click binding (or `Press`).
- The default timings are a machine setting (`settings` table,
  `gestures.defaults`), and pages store only overrides.
- Validation: `MixedTriggers` (raw and gesture triggers on one control),
  gesture triggers only on controls that accept press and release (keys,
  pressable encoders), timings in range, and the stateful rules (section 4).

### 3.2 Targets per gesture

Each gesture binding has a normal `BindingTarget`: a saved action instance,
a workflow, `OpenPage`, `Back`, or the new `Stateful` (section 4).
**Parameterized actions**: under the MVP freeze, inline invocations
(`BindingTarget::Invoke`, action-architecture §3.2) were deferred. The
minimal compatible option:

- **P1 (proposed now)**: `BindingTarget::Action { action, overrides }`, where
  `overrides: ActionParameters` is optional and absent by default. These are
  the same shape and rules as a workflow node's `overrides`: literal,
  `reference` or `template` values (A's #94), merged over the instance's
  parameters and resolved by A's pipeline at execution. One saved instance
  can serve several gestures with different parameters, **with no
  duplicated instances**. Old documents decode unchanged.
- **P2 (later, unchanged contract)**: once C ships `Invoke`, the gesture
  editor offers it as a third target kind. P1 bindings keep working.

`BindingTarget` stops being `Copy` either way. C already planned that.

### 3.3 Migration of existing single-click bindings

- **No rewrite.** Every existing `Press` binding stays a raw-mode `Press`
  binding and fires on the press edge exactly as today. The 0.5.0-2 and
  0.6.0 fixtures must behave identically (tested).
- The gesture editor shows a lone `Press` binding as "Single click
  (immediate)". When the user adds a double click or long press to that
  key, the editor turns `Press` into `Click` in the same save, shows the
  change, and the user confirms it. Nothing is converted silently.
- Document version: the new shapes (`click`/`doubleClick`/`longPress`,
  `gestures`, `overrides`, `stateful`) need a document version an older
  importer refuses. E's #97 introduces `DOCUMENT_VERSION = 5` for 0.7, and
  it is not released. **If these shapes land before 0.7.0 ships, they use 5.
  If 0.7.0 ships first, they bump to 6.** No database schema change is needed
  for bindings, because they live in the profile document.

## 4. Stateful key behaviour

A key whose operation depends on a typed state, with transitions only the
state's owner can make:

```jsonc
{ "type": "stateful", "machine": "workSession",
  "rules": [ { "state": "idle", "gesture": "click", "operation": "start" }, ... ] }
```

- **`StateMachine`** (core trait, `crates/deckpilot-core/src/stateful.rs`,
  G4) declares its id, its states (a closed list), its operations and its
  transition table as data, in the style of `work_session::TRANSITIONS`.
  Built-in machines only: no user code, no expressions, no JavaScript.
- **Bindings map `(state, gesture) → operation`.** Validation at save time:
  the machine exists, every `state` and `operation` is one it declares, each
  `(state, gesture)` appears once, and the gesture is bound on that key. The
  machine can also add its own rules. For the work session, `finish` may
  only be bound to `click` (section 5.3).
- **At runtime** the agent reads the machine's state **as of the press**
  (captured in `KeyContext`), picks the rule and asks the machine to apply
  the operation. The machine checks its guards against its *current* state
  and refuses stale ones. The binding never sets state directly.
- If `rules` is empty, the machine's default table applies, so a key works
  as soon as it is assigned. That table is not an action instance and is
  not stored.
- The face of a stateful key comes from the machine (section 8).
- Later, user-defined machines (for example over a typed `choice` shared
  variable with an explicit transition table) fit the same binding shape.
  They are out of scope here.

Work-session default rules:

| State | Click | Long press | Double click |
|---|---|---|---|
| idle | start | – | – |
| starting | – (shows "Starting…") | cancelStart | – |
| working | verify (only with a verification workflow) | verify (only without one: passes at once, D12) | – |
| verifyingEnd | – (shows "Checking…") | cancelVerification | – |
| readyToFinish | finish | continueWorking | – |
| stopping | – | – | – |

Because long press is bound, a click fires on the release. Double click is
not bound by default, so a click never waits 300 ms. Without a verification
workflow, a click in working does nothing except show "Hold to finish", so
no click ever leads towards finishing (D12).

## 5. Work session state machine

States and events are `WorkSessionState` and `WorkSessionEvent`. The
complete list of state changes is `work_session::TRANSITIONS`, and anything
else is refused (`InvalidTransition`).

### 5.1 Transitions, guards and effects

| # | From | Event | Guard | Effect (one transaction unless noted) | To |
|---|---|---|---|---|---|
| T1 | idle | Start | unlocked; no active session; settings valid; session's profile active | insert `work_sessions` row (`starting`, `requested_at`); if a start workflow is set, run it with tag `Start{session}` | starting |
| T2 | starting | StartSucceeded | run tag matches session; row still `starting` | `started_at = now`; open interval 1; mark the row active; event | working |
| T3 | starting | StartFailed | failure, cancel (`cancelStart`) or interruption of the tagged run | row → `aborted`, notice `startFailed`/`startInterrupted`; **no interval was ever opened** | idle |
| T4 | working | Verify | unlocked; session's profile active; verification workflow not already running | new `VerificationId v`; persist `verifying(v)`; run with tag `Verify{session, v}`. None configured (D12): only by long press, and T4 + T5 commit together with `canFinish = true`, details "no verification configured" | verifyingEnd |
| T5 | verifyingEnd | VerificationPassed | tag `(session, v)` = current; outputs valid, `canFinish = true` | store outcome; `approval = v`, `approved_at`, `approval_boot` | readyToFinish |
| T6 | verifyingEnd | VerificationRejected | tag matches; `canFinish = false` | store outcome and details; notice `notReady` | working |
| T7 | verifyingEnd | VerificationFailed | tag matches; run failed or cancelled, no valid `canFinish`, or interrupted | notice `verificationFailed`/`verificationInterrupted` | working |
| T8 | readyToFinish | Verify | as T4 | the old approval is dropped and a new `v` is used | verifyingEnd |
| T9 | readyToFinish | ApprovalInvalidated | TTL passed, sleep detected, agent restarted, settings changed, or `continueWorking` | clear approval; notice `approvalExpired`/`approvalInvalidated` | working |
| T10 | readyToFinish | Finish | **single click on the key only** (D11; the app has no finish); `approval` = current; approval valid; key pressed ≥ arming delay after entering ready | CAS `readyToFinish → stopping` with `stopped_at = now`, close the open interval at `now` | stopping |
| T11 | stopping | StopCompleted | – | row → `stopped`, clear active marker, event | idle |

Invariants (tested in the contract):

- The timer runs in working, verifyingEnd and readyToFinish. **Only T10
  stops it.** A negative or failed verification, an expired approval, a
  disconnect, a sleep or a restart never does.
- A result is applied only if its tag names the **current session and the
  current verification id**. A result from an old session, an older
  verification or a cancelled run is dropped (logged at debug). An old
  session's verification can never reach T5 for a new one, and T10 requires
  `approval` to equal the id that produced T5 for this session.
- The time before T2 (the start workflow) is not counted (D10).

### 5.2 Final stop: atomic and idempotent

```sql
BEGIN IMMEDIATE;
UPDATE work_sessions SET state = 'stopping', stopped_at = :now, updated_at = :now
 WHERE id = :session AND state = 'readyToFinish' AND approval_id = :approval
   AND approved_at >= :now - :ttl_ms AND approval_boot = :boot;
-- 1 row: close the interval, log the transition, COMMIT, then T11.
-- 0 rows: if the row is 'stopping' or 'stopped' with approval_id = :approval,
--         answer "already finished" (idempotent); otherwise refuse as stale.
UPDATE work_session_intervals SET ended_at = :now, end_reason = 'stopped'
 WHERE session_id = :session AND ended_at IS NULL;
INSERT INTO work_session_transitions (...);
COMMIT;
```

Only a key click can finish (D11). Two clicks, or a duplicated signal, with
the same approval cannot both finish: the second gets "already finished". A
click with an old approval is refused. T11 is a second, trivial transaction. If the agent
dies between the two, recovery completes T11 (section 6.5), and the stop
time is already fixed.

### 5.3 Click safety in readyToFinish

- `finish` may only be bound to `click` (validation).
- The operation is chosen from the state **at the press**. A press that
  started in working (verify) cannot become a finish because the
  verification passed while the key was down or within its double-click
  window.
- **Arming delay**: a finish click whose press comes less than 1 s after
  entering readyToFinish is ignored. Without it, two quick clicks (verify,
  then the instant approval of a fast verification) would finish by
  accident. The 1 s is a proposal, see D9.

## 6. Persistence, recovery and time

### 6.1 Time sources

- **Wall clock** (UTC ms, `AgentOptions::clock`, injectable) for everything
  that is stored: `requested_at`, `started_at`, interval ends,
  `verified_at`, `stopped_at`. **Elapsed time is never stored as a counter.**
  It is `elapsed_ms(intervals, now)` (contract), which treats a negative
  interval as zero.
- **Monotonic** (`Instant`) for gestures, timers and sleep detection only.
- **Sleep and clock jumps**: on each auto-lock check (already periodic),
  compare the wall-clock delta with the monotonic delta. A wall delta that
  is bigger by more than `SLEEP_GAP` means sleep. A wall clock that went
  backwards means the clock was changed.

### 6.2 Heartbeat

While a session is active, `last_alive_at` is updated **once per minute**
and at a clean shutdown. That is a liveness mark, not a counter, at a cost
of one small `UPDATE` per minute. After a restart, `now - last_alive_at`
bounds how long nothing was running.

### 6.3 Reviews of uncertain time

A gap longer than the review threshold (default 15 min, D8) records a
`work_session_reviews` row of kind `agentDown`, `sleep`, `clockBack` or
`clockForward` and shows the notice `needsReview`. The gap is **counted by
default**, so the timer is preserved. The user can exclude it later, in the
session history on the Work Session page (D8). *Exclude* splits the interval in one
transaction: close it at `from`, open a new one at `to`. Elapsed time stays
a plain sum.

### 6.4 Schema 9 (owned by W1)

E's #97 is open and adds schema 8. These tables are **one new migration step
appended after E's** (`crate::work_sessions::SCHEMA`, index 8 → schema 9),
under the existing backup, transaction and history machinery. W1 starts
from `main` after #97 has merged (merging `main` into its branch, no
rebase). If #97 were withdrawn, the same step would become schema 8
unchanged.

```sql
CREATE TABLE work_session_settings (
    profile_id             TEXT PRIMARY KEY NOT NULL REFERENCES profiles (id) ON DELETE CASCADE,
    start_workflow         TEXT,                 -- WorkflowId or NULL
    verification_workflow  TEXT,                 -- WorkflowId or NULL
    approval_ttl_seconds   INTEGER NOT NULL DEFAULT 600 CHECK (approval_ttl_seconds BETWEEN 60 AND 86400),
    review_gap_seconds     INTEGER NOT NULL DEFAULT 900 CHECK (review_gap_seconds BETWEEN 60 AND 86400),
    elapsed_format         TEXT NOT NULL DEFAULT 'minutes' CHECK (elapsed_format IN ('minutes', 'seconds')),
    updated_at             INTEGER NOT NULL
);
CREATE TABLE work_sessions (
    id                     TEXT PRIMARY KEY NOT NULL,       -- WorkSessionId
    profile_id             TEXT NOT NULL,                   -- no FK: history outlives the profile
    project_id             TEXT,
    state                  TEXT NOT NULL CHECK (state IN ('starting', 'working', 'verifyingEnd',
                               'readyToFinish', 'stopping', 'stopped', 'aborted')),
    active                 INTEGER UNIQUE CHECK (active IS NULL OR active = 1), -- 1 while not stopped/aborted: one active session
    requested_at           INTEGER NOT NULL,
    started_at             INTEGER,                          -- set by T2
    stopped_at             INTEGER,                          -- set by T10
    start_workflow         TEXT,                             -- as configured at T1
    verification_workflow  TEXT,                             -- as configured at T4
    start_execution        TEXT,                             -- ExecutionId of the start run
    verification_id        TEXT,                             -- current/last VerificationId
    verification_execution TEXT,
    can_finish             INTEGER CHECK (can_finish IN (0, 1)),
    verification_details   TEXT,                             -- non-sensitive only, ≤ 200 chars
    verified_at            INTEGER,
    approval_id            TEXT,                             -- = verification_id while readyToFinish
    approved_at            INTEGER,
    approval_boot          TEXT,                             -- agent instance id at T5
    last_alive_at          INTEGER NOT NULL,
    problem                TEXT, problem_message TEXT, problem_at INTEGER,
    updated_at             INTEGER NOT NULL
);
CREATE INDEX work_sessions_started ON work_sessions (profile_id, requested_at);
CREATE TABLE work_session_intervals (
    session_id  TEXT NOT NULL REFERENCES work_sessions (id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    end_reason  TEXT CHECK (end_reason IN ('stopped', 'gapExcluded')),
    PRIMARY KEY (session_id, seq)
);
CREATE UNIQUE INDEX work_session_open_interval
    ON work_session_intervals (session_id) WHERE ended_at IS NULL;
CREATE TABLE work_session_reviews (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL REFERENCES work_sessions (id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('agentDown', 'sleep', 'clockBack', 'clockForward')),
    from_at     INTEGER NOT NULL,
    to_at       INTEGER NOT NULL,
    resolution  TEXT CHECK (resolution IN ('counted', 'excluded')),   -- NULL: open
    resolved_at INTEGER
);
CREATE TABLE work_session_transitions (            -- audit log, also the history view
    session_id  TEXT NOT NULL REFERENCES work_sessions (id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    at          INTEGER NOT NULL,
    from_state  TEXT NOT NULL,
    event       TEXT NOT NULL,
    to_state    TEXT NOT NULL,
    PRIMARY KEY (session_id, seq)
);
```

- Every transition updates the row and appends to
  `work_session_transitions` in one transaction. In-memory state is changed
  only after the commit.
- Retention: the last 1000 finished or aborted sessions. Deleting a profile
  that has the active session is refused ("finish the work session first").
- Not exported in this release (machine-local history). A downgrade means
  restoring the pre-upgrade backup, as with schema 8.
- Alternative (D6): module-owned tables (`ModuleMigration`, no core bump).
  This was rejected for now because module migrations take no backup, and
  the agent, not a module handler, writes these tables.

### 6.5 Recovery

On start, and on every unlock, the agent loads the row with `active = 1`:

| Persisted state | Recovery | Timer |
|---|---|---|
| none / idle | idle | – |
| starting | T3 with `startInterrupted`. The start workflow may have run partly, so the user presses Start again | was never started |
| working | stays working; review if `now - last_alive_at` > threshold | runs on, uninterrupted |
| verifyingEnd | T7 with `verificationInterrupted`. The user verifies again | runs on |
| readyToFinish | T9 (the boot changed), notice `approvalInvalidated`. The user verifies again | runs on |
| stopping | T11 (complete the stop) | already stopped at `stopped_at` |

- **Ambiguous cases never pick an outcome that loses time.** The timer is
  kept, and the user acts explicitly: verify again, start again, or resolve
  the review.
- **Sleep and wake**: the wall-clock intervals keep running. On wake the
  approval is invalidated (T9), and a long sleep creates a review. If the
  database auto-locked on sleep, this happens at unlock.
- **Locked**: no transition can be written, so operations answer "Unlock
  DeckPilot". The keys show "Locked" as today. A verification interrupted
  by the lock is recovered as T7 at unlock. Elapsed time is unaffected.
- **Disconnect**: the session does not depend on a device. Only pending
  gestures are cancelled. The faces are correct again on reconnect.
- **UI closed**: the agent owns the session, so nothing changes.

## 7. Workflow contracts

Both workflows are ordinary saved workflows of the session's profile,
started through `start_execution_with` with `ExecutionTrigger::WorkSession`
(new variant; older readers see `other`) and an `ExecutionPurpose` tag kept
on the running execution. `finish_execution` hands the outcome and the
**declared outputs** of tagged runs to the work-session service. This needs
`workflow_outputs` from `runtime.rs` to be callable for top-level runs (a
small change, coordinated with A).

- **Start workflow** (optional): succeeds → T2. Fails, is cancelled or is
  refused (already running, 4 runs active, locked) → T3. Its outputs are
  ignored. It must not have required inputs other than those below.
- **End verification workflow**: must declare the output `canFinish` of
  kind `boolean`. It may declare `details` of kind `text`, which is shown
  unless it is marked sensitive. This is checked by
  `work_session::verification_contract` when the workflow is selected and
  again before each run. A successful run is read with
  `VerificationOutcome::from_outputs`: exactly `true` or `false`, anything
  else is an error (T7), never a "no". A failed run is T7, never T6.
- **Inputs**: if a workflow declares inputs named `sessionId`, `projectId`
  or `elapsedSeconds`, the agent fills them. The variables in section 8 are
  the general mechanism once A's resolver lands.
- **Timeouts**: the existing execution limits and cancellation apply.
  `cancelStart` and `cancelVerification` cancel the tagged run.

## 8. Variables and key faces

### 8.1 Read-only variables

`WorkSessionVariable` (contract) proposes these paths under the existing
`system` root, so the closed root set stays as it is (D5):

| Reference | Type | Value |
|---|---|---|
| `system.work.session.id` | string | active session id |
| `system.work.session.state` | string | `WorkSessionState::name` |
| `system.work.session.startedAt` | string | RFC 3339 UTC of `started_at` |
| `system.work.session.elapsedSeconds` | integer | `elapsed_ms(intervals, now) / 1000`, computed at resolution |
| `system.work.session.projectId` | projectId | project at start (another profile resolves to `missing(otherProfile)`) |
| `system.work.session.canFinish` | boolean | last verification outcome |

All are dynamic, never cached and `missing(noValue)` without a session.
W3 adds them as `SystemVariable` variants together with A's agent resolver
arm and the catalog entries (group System). F's picker lists them with no
picker code of its own. Open pickers refresh on `workSessionChanged`
(section 9), with no polling.

### 8.2 Key state provider

- A key bound to `Stateful { machine: "workSession" }` takes its face from
  the agent's in-memory `WorkSessionSnapshot`, synchronously during render.
  There is no provider thread, no polling and no action instance. `states.rs`
  gets a second source kind next to the module providers.
- Faces (label / detail / indicator):

  | State | Label | Detail | Indicator |
  |---|---|---|---|
  | idle | Start Work | (last notice, if any) | – / warning |
  | starting | Starting… | | running |
  | working | `1:05` (elapsed) | Working | ok, warning with an open notice or review |
  | verifyingEnd | Checking… | elapsed | running |
  | readyToFinish | Finish Work | elapsed | ok |
  | stopping | Stopping… | | running |

- **Refresh**: the faces change on transitions, which are pushed. The
  elapsed text is the only thing that ticks. With the default `h:mm` format
  the agent schedules **one redraw per minute boundary of elapsed time**, and
  with `seconds` one per second. It does so **only while at least one work
  session key is shown on a connected device and the timer runs**. This
  uses one deadline merged into the run loop's `sleep_until`. The frame cache
  skips identical faces, so only that key is sent. With no such key shown,
  nothing is scheduled (tested by counting renders over simulated time).
- The app draws elapsed time from `elapsedMs` at `asOfMs` and ticks locally
  while visible, so it needs no requests.

## 9. IPC and Tauri

**Protocol 8** (after A's 7), `Capability::WorkSession` (since 8):

```jsonc
{ "type": "getWorkSession" }                         → { "type": "workSession", "snapshot": WorkSessionSnapshot }
{ "type": "listWorkSessions", "limit": n /* 1–100 */, "before"?: ms } → { "type": "workSessions", "entries": WorkSessionSummary[] }
{ "type": "getWorkSessionSettings", "profile": id } → { "type": "workSessionSettings", "settings": WorkSessionSettings }
{ "type": "saveWorkSessionSettings", "settings": WorkSessionSettings } → workSessionSettings
{ "type": "resolveWorkSessionReview", "session": id, "review": n, "count": bool } → { "type": "workSessionEntry", "entry": WorkSessionSummary }
{ "type": "clearWorkSessionNotices", "session": id | null } → workSession
// event, protocol ≥ 8 only:
{ "type": "workSessionChanged", "snapshot": WorkSessionSnapshot }
```

- There is **no request that starts, verifies or finishes a session**
  (D11): session operations come only from the key. Tests drive them
  through the agent's `test-hooks` entry point until key bindings do.
- The shapes are in `crates/deckpilot-ipc/src/work_session.rs` and mirrored
  in `apps/desktop/src/lib/types.ts`. A `WorkSessionSummary` carries the
  session's intervals, reviews, transitions, last verification and notice,
  and its counted `elapsedMs`. Verification details are sent at most 200
  characters long and notice messages at most 500; sensitive details are
  never stored, so never sent.
- Settings go through the agent, which owns the session: it checks the
  workflows against the profile (the start workflow needs no inputs a
  session cannot fill; the verification workflow fulfils the contract),
  writes `work_session_settings` and drops an approval when they changed
  (D9). `clearWorkSessionNotices` without a session clears the notice an
  idle session shows. `workSessionChanged` is also sent when only the
  counted time changed (a gap excluded or counted again).
- Disabling the `work-session` module is refused while a session is
  active.
- Tauri (W5): `get_work_session`,
  `list_work_sessions`, `resolve_work_session_review`,
  `clear_work_session_notices`, `get_work_session_settings`,
  `save_work_session_settings`, and the event `agent-work-session`. With an
  agent below 8 they answer "not supported" and the page explains that.
- Gestures need no protocol change. `simulateInput` (press/release) is
  enough for tests and the hardware check.

## 10. UI

- **Gesture editor** (`apps/desktop/src/components/gesture-editor/**`, G3),
  shown in the layout designer's key inspector. It has one row each for
  single click, double click and long press, and each row has the existing
  target selector (action, workflow, page, back, stateful) plus parameter
  overrides through the schema-driven `ParameterField` with F's picker. The
  timings show "default" or a value, with the limits enforced. A note says
  whether single click is immediate, on release, or after the
  double-click window. The raw-to-gesture conversion of section 3.3 is
  confirmed in a dialog. A "Changes with state" mode edits stateful rules
  (machine, state × gesture → operation) with validation.
- **Work Session page** (module page `work-session`, W6,
  `apps/desktop/src/modules/work-session/**`):
  - Status card with live state, elapsed time, project, the last
    verification with details, and notices with **Clear**. **State only:
    no Start, Verify or Finish buttons** (D11). The session is operated
    from its key.
  - Settings: start workflow (any workflow of the profile without required
    inputs) and verification workflow (only workflows that pass the
    contract, with the issue shown for the others), approval lifetime,
    review threshold, elapsed format, and gesture timings for the work
    session key.
  - **Assign to key…** opens the layout designer's key chooser and writes a
    `Stateful { machine: "workSession" }` binding. **No action instance is
    created**, before or after.
  - History: sessions with date, project, duration, result and
    transitions. Flagged gaps show here with **Exclude** (D8).
- **Settings › Devices**: the default gesture timings.
- A built-in module `work-session` (`modules/work-session`, a manifest only
  with page, config and no action types) provides the page and the
  enable/disable switch. Disabling it is refused while a session is active.

## 11. Test plan (Phase 9)

| Requirement | Test | Owner |
|---|---|---|
| click / double (300 ms) / long (700 ms) defaults | engine unit tests with synthetic `Instant`s at boundaries (299/300/301, 699/700/701) | G1 |
| configurable with limits | `GestureTimings::validate` (done), editor validation, layout validation | G0, G2, G3 |
| low-level press/release | raw-mode keys pass edges unchanged, same as 0.6 | G1, G2 |
| exactly one gesture per sequence | property test over random press/release/time sequences | G1 |
| single click delayed only when double bound | `ClickTiming` (done); measured fire times per binding set | G0, G1 |
| long press once per hold, release never clicks | hold 5 s → one long press; the release fires nothing | G1 |
| disconnect cancels pending | unplug during hold and within the window (engine + mock MK.2 agent test) | G1 |
| profile/page change invalidates | navigate or reload while pending → nothing fires; held release swallowed | G1 |
| device identity, devices independent | two mock MK.2s with interleaved sequences; reconnect = new connection | G1 |
| bounded non-blocking timers | entry count ≤ keys; a 2 s synchronous action does not turn a click into a long press (`at` stamps) | G1 |
| never on the HID thread | dispatch thread name ≠ `device-*` | G1 |
| controllable clock | engine takes times; agent tests use `tokio::time::pause` and an injected wall clock | G1, W2 |
| independent bindings per gesture | layout model, validation, runtime dispatch per trigger | G2 |
| saved instance / parameterized / workflow | dispatch tests for each target; overrides merged and resolved | G2 (+A) |
| references in parameters | override with `reference`/`template` resolved at press time | G2 + A |
| existing bindings unchanged | fixtures 0.5.0-2 and 0.6.0 decode and fire on press, unchanged | G2 |
| no auto-duplicated actions | saving gesture bindings and assigning the session key leave `actions` untouched | G2, G3, W6 |
| stateful mapping | unknown state or operation refused; `finish` only on click; press-time state used | G4 |
| every work session transition and guard | `TRANSITIONS` invariants (done); service tests per T1–T11 | G0, W2 |
| start failure → idle + error | failing, cancelled and refused start workflow | W2 |
| canFinish typed | `from_outputs` strictness (done); missing or ill-typed output → T7 | G0, W2 |
| verification never stops the timer | elapsed keeps growing through T6/T7/T9 | W2 |
| finish only by click in ready, atomic + idempotent | only a key click finishes (no IPC path), double click, duplicated signals, arming delay | W2 |
| stale approvals | TTL, sleep, restart, re-verify, settings change | W2 |
| old session's verification cannot stop a new one | result tagged with an old session or old `v` is dropped | W2 |
| disconnect never stops | unplug in each state | W2 + T |
| restart in each state | kill and restart with the store in each persisted state (table 6.5) | W1, W2 |
| sleep/wake, clock jumps | injected wall clock vs paused monotonic time; reviews created and resolved | W2 |
| SQLCipher persistence, schema 9 | migration from E's schema 8 fixture, backup, rollback, refusal by older; no counter writes (only the 1/min heartbeat) | W1 |
| variables | resolver values per state, `elapsedSeconds` computed, picker lists them | W3, F |
| key faces | face per state; render count over simulated minutes; none when not shown | W4 |
| settings page | workflow filtering by contract, assign to key, history, clear notices | W6 |
| no default instance | fresh database with the module enabled has no actions or bindings | W6, E |

## 12. Work split

| WS | Scope | Owns (files) | Depends on |
|---|---|---|---|
| **G0** | this contract | `crates/deckpilot-core/src/{gesture.rs, work_session.rs}`, the ids in `id.rs`, the gesture/work-session section at the end of `types.ts`, this document | – |
| **G1** gesture engine | `GestureEngine`, `at` stamps, loop deadline, invalidation hooks | `crates/deckpilot-core/src/gesture/engine.rs` (new; `gesture.rs` becomes `gesture/mod.rs`), `apps/agent/src/devices.rs` (`at`), `apps/agent/src/service/gestures.rs` (new), the arms in `service.rs::run`/`handle_signal`, `apps/agent/tests/gestures.rs` | G0. Feeds raw-mode edges until G2 lands |
| **G2** gesture bindings | `Trigger::{Click, DoubleClick, LongPress}`, `LayoutPage.gestures`, `MixedTriggers`, `Action.overrides` (P1), `Runtime::dispatch_gesture` | `layout.rs` and `device_runtime.rs` (**C's files**: done by C or in a PR C reviews, after C makes `BindingTarget` non-`Copy`) | G0, C-model. References: A |
| **G3** gesture editor | editor UI, conversion dialog, timings | `apps/desktop/src/components/gesture-editor/**` (new); the integration lines in `pages/layout-designer.tsx` (**D's file**, coordinated) | G2 (types), F (`ParameterField` with the picker) |
| **G4** stateful bindings | `StateMachine` trait, `BindingTarget::Stateful`, validation, press-time state | `crates/deckpilot-core/src/stateful.rs` (new), the variant in `layout.rs` (with C) | G2 |
| **W1** storage | schema 9, store API (`WorkSessionStore` methods on `SqliteStore`), settings, retention, fixtures, upgrade tests | `crates/deckpilot-storage/src/work_sessions.rs` (new), the `MIGRATIONS` entry in `sqlite.rs`, `crates/deckpilot-storage/tests/upgrade_*work_session*` | G0, **E's #97 merged** |
| **W2** agent service | state machine application, tagged runs, recovery, heartbeat, sleep/clock detection, arming, idempotent finish | `apps/agent/src/service/work_session.rs` (new); the `ExecutionPurpose` tag in `service/executions.rs`; the hook in `service/security.rs::check_auto_lock`; `ExecutionTrigger::WorkSession` in `history.rs`; making `workflow_outputs` callable (`runtime.rs`, **with A**) | W1, G0. Key operations: G4 |
| **W3** variables | `SystemVariable` variants, resolver arm, catalog | `variables/expression.rs` (contract PR), the arm in A's `apps/agent/src/service/variables.rs` | A's resolver, W2 |
| **W4** key faces | the work-session state source, the elapsed-refresh deadline | `apps/agent/src/service/states.rs` (a second source kind), `apps/agent/src/service/work_session_faces.rs` (new) | W2, G4 |
| **W5** IPC + Tauri + module | protocol 8, `Capability::WorkSession`, requests and event, Tauri commands, `modules/work-session` manifest crate | `crates/deckpilot-ipc/src/protocol.rs` (**after A's protocol 7 merges**), `apps/desktop/src-tauri/src/commands/work_session.rs` (new) + registration, `modules/work-session/**` (new), `apps/desktop/src/lib/api.ts` (the work-session calls only) | A's IPC 7, W2 |
| **W6** Work Session page | the page, settings, history, assign to key | `apps/desktop/src/modules/work-session/**` (new) | W5 (starts on a mocked `api.ts`), G4 for assignment |
| **T** end-to-end | mock-device agent tests for gestures and sessions, hardware checklist entry | `apps/agent/tests/work_session.rs` (new), `docs/release-validation.md` (an entry) | G1, W2, W4 |

`types.ts`: each workstream appends its own section, and changes to this
contract section go through a contract PR. `service.rs` stays shared: every
workstream adds arms only, as A and B do.

**Suggested start order**: G0 → in parallel **G1**, **W1** (once #97 is
merged), **G2 with C**, and **W6 on mocks** → **G4**, **W2** → **W4**, **W3**
(after A's resolver), **W5** (after A's protocol 7) → **G3**, W6 integration →
**T**.

## 13. Decisions

Decided by the user (2026-11-12): D4, D8, D11, D12. The rest are the
proposed defaults, accepted by the coordinator.

- **D1** Single click is delayed only when double click is bound to that
  key. With long press bound it fires on the release. Double click fires on
  the second *press*.
- **D2** Existing `Press` bindings are not rewritten: they stay immediate
  (raw mode). The editor converts them to `Click` only when the user adds
  another gesture, with a confirmation.
- **D3** Parameterized gesture actions: **P1 now**, a saved instance plus
  per-binding `overrides` (references allowed). There is no inline
  invocation now. `Invoke` may come later through C, with no contract
  change.
- **D4** Elapsed text on keys is `h:mm`, redrawn once per minute and only
  while a work session key is shown. `seconds` (1 Hz) is opt-in per profile.
- **D5** Variables are `system.work.session.*` (existing root).
- **D6** Storage is core **schema 9** after E's schema 8.
- **D7** There is one active session per database. It records its profile
  and project. Start and verify need that profile to be active. Finish does
  not.
- **D8** A gap over 15 min (agent down, sleep, clock jump) is counted and
  flagged for review. The user can exclude it later in the session history.
- **D9** The approval lasts 10 min and is invalidated by sleep, restart, a
  new verification and settings changes. A finish click whose press comes
  less than 1 s after entering readyToFinish is ignored.
- **D10** Time counts from entering `working`. The start workflow's
  duration is not counted.
- **D11** Finishing is possible **only** by a single click on the key in
  readyToFinish. The app has no Finish button and no IPC request that
  finishes. The Work Session page shows the state only.
- **D12** With no verification workflow configured, a **long press** in
  working passes at once to readyToFinish (`canFinish = true`, details "no
  verification configured"). In that configuration a click in working does
  nothing.
