# Privileged capabilities for first-party plugins (proposal)

Status: **proposed, for the owner's decision. Nothing here is implemented.**
Every operation, permission kind and limit below is a proposal; none exists
in plugin API 1.2. It covers the four plugins of the catalog that wait for
a privileged host service (`docs/plugin-factory.md` in deckpilot-plugins,
§6.6): **system-monitor**, **power**, **screen-capture** and **clipboard**.

The rule that frames all four: a plugin never gets an unrestricted file
system, process, screen or input capability. Each capability does one
narrow thing, the agent keeps every security-relevant decision, and the
permission kinds that are refused today (`FORBIDDEN_PERMISSION_KINDS` in
`crates/deckpilot-plugin-manifest/src/lib.rs`: `clipboard`,
`machineResources`, `process`, `fileSystem`, …) **stay refused**. Where a
proposal touches one of those areas, it uses a new, narrower kind with a
different name, so that an old manifest asking for the broad kind is still
refused.

## 1. What it builds on

Everything below reuses mechanisms that exist and are tested:

| Mechanism | Where | Used for |
| --- | --- | --- |
| Closed set of capability requests, strict JSON, size checks | `plugins/runtime/crates/abi/src/capability.rs` | one new variant per operation |
| Per-call broker decision, deny by default, grants read per call | `plugins/runtime/crates/broker/src/lib.rs` (`Session::authorize_in`) | every check below |
| Only in runs the user started | `RunContext::is_user_initiated`, `DenyReason::NotUserInitiated` (`url.open`, G8) | power, screen capture, clipboard |
| Read-only state calls | `state::allowed_in_state`, `Session::authorize_state` (#293) | system metrics on keys |
| Token-bucket rate limits per plugin | `broker/src/rate.rs` (`RateLimits`) | every budget below |
| Audit records without values | `broker/src/audit.rs` | every operation, by name only |
| Consent tiers low, medium, high | `crates/deckpilot-plugin-store/src/grants.rs` (`Tier`) | the tier of each kind |
| Executors in the agent, outside the sandbox | `apps/agent/src/plugins/executor.rs` | the platform code, behind traits |
| Test hooks that record instead of acting | e.g. `DECKPILOT_TEST_PLUGIN_OPEN_URL` (feature `test-hooks`) | end-to-end tests that never sleep or shut down a machine |
| Machine metrics without a plugin | `modules/system` uses `sysinfo` (feature `system`) for `system.process-status` | system metrics |

The plugin host stays as it is: one wasm module, one import, no WASI, an
OS sandbox. All new platform code runs in the agent, behind the broker.

## 2. Common rules (proposed)

- **One operation, one permission kind, one tier.** No kind implies
  another (as `grant_kinds.rs` tests for today's kinds).
- **No data leaves the agent that is not in the reply's schema.** Replies
  have fixed, bounded fields. No path, no window title, no process name,
  no host name, no image data is ever in a reply unless the section below
  says so explicitly, and none of the four does for images.
- **User-initiated only** for anything that changes the machine or
  touches the user's data (power, screen capture, clipboard): a key, a
  gesture, the app, or a workflow the user started. Never a schedule, an
  event, the agent's start or a work session. Checked before any rate
  limit, as for `url.open`.
- **Never in a state call** except system metrics (read-only, aggregate).
- **Never in `dp_init` or `dp_shutdown`** (no invocation; already true).
- **Audited by operation name**, plus the action of `power.request`.
- **Revocation applies on the next call** (grants are read per call).
- **Off on platforms where the OS cannot enforce the boundary** described
  for that platform (§3-§6, "OS permissions"): the operation answers
  `unavailable`, never a weaker fallback.
- Plugins do not run on Windows today; the Windows parts apply once they
  do.

## 3. System monitor (read-only metrics)

### Broker API

`system.metrics {}` → `{ cpuPercent, memoryPercent, swapPercent?,
uptimeMinutes, loadAverage1m? }`:

- whole-machine figures only, rounded: percentages as integers 0-100,
  uptime to the minute, load to one decimal (Unix only);
- no totals in bytes, no core count, no model names, no disks, mounts or
  paths, no network interfaces, no per-process figures, no temperatures
  (all of these identify the machine or reveal what runs on it; none is
  needed for a usage bar);
- the agent samples once for all plugins: one shared `sysinfo::System`,
  refreshed at most once per second (CPU usage needs two refreshes apart);
  a call within the second gets the last sample.

Per-process inspection (G11 `process.query` in `module-migration.md`) is
**not** part of this proposal and stays as described there.

### Permission and tier

New kind `systemMetrics` (no fields), grant key `systemMetrics`, **low**
tier. Not `machineResources`, which stays refused (in DeckPilot it means
the user's bound programs and paths).

### Consent flow

Shown at install with the other low-tier grants: "Read the computer's
total CPU and memory usage (no programs, files or names)." Revocable on
the plugin's page; revoked, the call is `not-permitted` and the key's
state call returns its static face.

### OS permissions

None on macOS, Windows or Linux: whole-machine CPU and memory figures need
no privilege (`host_statistics64`/`sysctl`, `GetSystemTimes` and
`GlobalMemoryStatusEx`, `/proc/stat` and `/proc/meminfo`; through
`sysinfo`).

### Never exposed

Processes, their names, owners or command lines; files and disks; network
interfaces and addresses; hardware identifiers; anything that changes the
machine.

### Budgets

20 calls per second per plugin (the `time.now` budget); also allowed in
state calls (§2), where the 8-call and 5 s floor of key state already
apply. Reply under 128 bytes.

### Threat model

| Threat | Mitigation |
| --- | --- |
| Fingerprinting the machine | No totals, model or counts; percentages change all the time |
| Inferring user activity (CPU spikes) | Coarse (integer percent, at most one new sample per second); the same is visible to any web page through timing; low tier and consent |
| Denial of service by sampling | One shared sampler, at most one refresh per second, bucket per plugin |

### Test plan

- ABI: request and reply shapes, unknown fields refused, bounds.
- Broker: denied without the grant; allowed with it; allowed in a state
  call; rate limit; audit without values.
- Executor: a fake sampler (trait) for exact values and rounding; the real
  `sysinfo` sampler on all three CI systems returns values in range; two
  calls within a second refresh once.
- Agent e2e: a test plugin's key shows CPU usage through key state; revoke
  makes it static.

## 4. Power (lock, sleep, restart, shut down)

### Broker API

`power.request { action }` with `action` one of `lock`, `sleep`,
`restart`, `shutdown`; reply `null` once the agent has handed the request
to the operating system, or an error. Nothing else: no timer, no "force",
no log-off of other users, no hibernate setting, no wake timers.

`restart` and `shutdown` are **always confirmed by the agent** at run time
with a DeckPilot dialog naming the plugin and the action (default:
cancel, 30 s timeout). The plugin cannot skip, pre-answer or tell when it
appears; a cancelled or timed-out confirmation is `not-permitted`.

### Permission and tier

New kind `power` with `actions` (a non-empty subset of the four), grant
key per action (`power:lock`, …):

| Action | Tier |
| --- | --- |
| `lock` | low |
| `sleep` | medium |
| `restart`, `shutdown` | high |

### Consent flow

Each action is a separate line in the consent dialog with its tier.
Proposed: a high-tier line needs an explicit tick of its own, apart from
the install's general consent. The plugin's page shows the granted
actions and lets the user revoke each. Run-time confirmation (above) is
in addition to the grant, not instead of it.

### OS permissions (to be verified per OS version before implementation)

| | Lock | Sleep | Restart / shut down |
| --- | --- | --- | --- |
| macOS | no permission known to be needed for locking the session; the supported route is to be confirmed (public APIs are limited) | `pmset sleepnow` or IOKit, no admin | an Apple Event to System Events ("Automation" consent for DeckPilot, asked once by macOS) |
| Windows | `LockWorkStation` | `SetSuspendState` | `ExitWindowsEx`/`InitiateShutdown` with the shut-down privilege a desktop user has; no forced close |
| Linux | logind `LockSession` over D-Bus | logind `Suspend` (polkit decides) | logind `Reboot`/`PowerOff` (polkit may ask for a password; the agent never passes one) |

Where the OS refuses (polkit, Automation consent denied, no session bus),
the call is `failed` with the OS's reason in words; nothing is retried.

### Never exposed

Any process control, logging off other users, firmware or boot settings,
scheduling, forcing applications to close, or reading power state beyond
what the call returns.

### Budgets

`power` bucket: 1 call per 10 s per plugin; `restart`/`shutdown`
additionally 1 per minute, and at most one confirmation dialog open per
plugin.

### Threat model

| Threat | Mitigation |
| --- | --- |
| A plugin shuts the machine down unexpectedly (lost work) | High tier, user-initiated only, run-time confirmation the plugin cannot see or answer, no force |
| Annoyance by repeated locking or sleeping | User-initiated only, rate limits, revocable per action |
| A scheduled workflow powers off the machine | Refused: never in runs the user did not start (an owner decision could allow `lock` from schedules later) |
| Privilege escalation through polkit or Automation | The agent never supplies credentials; the OS asks the user itself or refuses |

### Test plan

- Broker: each action needs its own grant; user-initiated gate checked
  before rate limits; `restart`/`shutdown` budgets; audit names the action.
- Executor: a `PowerBackend` trait; a recording backend in tests; the
  confirmation step with a fake dialog (confirm, cancel, time-out); the
  real backends are built on all three CI systems but **never called** in
  CI.
- Agent e2e: with a test hook (`DECKPILOT_TEST_PLUGIN_POWER` recording
  requests, feature `test-hooks`), a key press locks; a scheduled workflow
  is refused; revoked is refused; a cancelled confirmation is refused.
- Manual: a short checklist per OS, run by a person, on a machine that may
  sleep and restart.

## 5. Screen capture

### Broker API

The plugin is a trigger; **pixels never enter the sandbox**.

`screen.capture { area, destination }`:

- `area`: `display` (the display with the pointer), `allDisplays`, or
  `interactive` (the OS's own selection UI, where one exists);
- `destination`: `folder` (a folder the **user** picked in DeckPilot for
  this plugin, held by the agent as a handle) or `clipboard` (the image is
  placed on the clipboard by the agent);
- reply `{ saved: bool, width, height }` and, for `folder`, the file name
  the agent chose (`Screenshot 2026-11-17 09.41.12.png`), never a path;
  `saved: false` when the user cancelled an interactive selection.

No window list, no window titles, no capture of a named window or
application, no video, no repeated or timed capture, no image data in the
reply, nothing sent anywhere by the agent.

### Permission and tier

New kind `screenCapture` with `destinations` (`folder`, `clipboard`),
**high** tier. The folder is chosen by the user in DeckPilot after the
grant, through a folder dialog of the app; the plugin never sees the path.
Without one, `folder` is `not-permitted`.

### Consent flow

High-tier line with an explicit tick, worded: "Take a screenshot when you
press its key, and save it to a folder you choose or put it on the
clipboard. The plugin never sees the image." Then the OS's own consent
(below) the first time. DeckPilot shows a short notice after each capture
naming the plugin (not suppressible by the plugin).

### OS permissions (to be verified)

| | |
| --- | --- |
| macOS | "Screen Recording" (TCC) for the DeckPilot agent, asked by macOS on first use; granting it lets the agent capture, and only this operation uses it. ScreenCaptureKit; `interactive` through the system's selection UI |
| Windows | Windows.Graphics.Capture (shows the system's capture border) or GDI; `interactive` through the system's snipping UI |
| Linux | Wayland: the xdg-desktop-portal Screenshot interface, which asks the user every time (`interactive` is the portal's own UI). X11: no OS gate, so the agent's own confirmation is mandatory there before every capture |

### Never exposed

Image data to the plugin or the network; any other folder; overwriting a
file; listing windows or reading their titles; continuous capture or
recording; input.

### Budgets

1 capture per 5 s per plugin, 20 per hour; the agent refuses an image
larger than 64 megapixels; files are PNG, never overwritten (a counter is
added).

### Threat model

| Threat | Mitigation |
| --- | --- |
| Exfiltrating what is on screen (passwords, messages) | The plugin never receives pixels; the reply has dimensions only; the agent sends nothing over the network; the folder is the user's |
| Silent capture in the background | User-initiated only; never from schedules or events; a notice after each capture; on Wayland the portal asks every time |
| Filling the disk | Rate limits, size cap, one folder |
| Abusing the agent's macOS Screen Recording grant | Only this operation captures; no other code path in the agent reads the screen; tests assert that |

### Test plan

- ABI and broker: grants per destination, user-initiated gate, budgets,
  the reply has no image data and no path (a test inspects every field).
- Executor: a `ScreenBackend` trait with a fake returning a known image;
  file naming, no overwrite, the size cap, cancelled interactive capture.
- Agent e2e: a test hook writes the would-be capture to the test's
  directory; a scheduled workflow is refused; the notice is recorded.
- Manual per OS: the OS consent appears once (macOS) or every time
  (Wayland portal); denied consent gives `failed`, not a black image.

## 6. Clipboard (write text)

### Broker API

`clipboard.writeText { text }`: puts plain text on the clipboard; reply
`null`. At most 16 KiB of UTF-8, no control characters except tab and
newline.

**Reading the clipboard is not proposed.** It holds passwords and
whatever the user copied last from any application; no narrow form of
"read" exists. If a plugin needs text from the user, it asks through an
action parameter. Images, files and rich formats are not proposed either.

### Permission and tier

New kind `clipboardWrite` (no fields), **medium** tier. The broad kind
`clipboard` stays in `FORBIDDEN_PERMISSION_KINDS`.

### Consent flow

Medium-tier line: "Replace the clipboard's content with text, when you
press its key." DeckPilot shows a status-bar note naming the plugin after
each write.

### OS permissions (to be verified)

| | |
| --- | --- |
| macOS | none for writing (`NSPasteboard`) |
| Windows | none (`OpenClipboard`/`SetClipboardData`) |
| Linux | X11: none. Wayland: writing needs a surface with focus or the data-control protocol; where neither is available the call is `unavailable` |

### Never exposed

Reading the clipboard, clipboard history, other formats, watching for
changes.

### Budgets

1 write per second per plugin, 30 per minute; 16 KiB per write.

### Threat model

| Threat | Mitigation |
| --- | --- |
| Pastejacking: replacing what the user copied with a malicious command | User-initiated only (the user just pressed the plugin's key), never from schedules; a visible note after each write; medium tier |
| Reading secrets | No read operation |
| Flooding | Rate limits and the size cap |

### Test plan

- ABI and broker: the size and character bounds, the grant, the
  user-initiated gate, budgets.
- Executor: a `ClipboardBackend` trait; a recording backend in tests; the
  real one on macOS and Windows CI writes and reads back through the OS
  in a test that restores the previous content.
- Agent e2e: a test hook records writes; a scheduled workflow is refused.

## 7. Summary

| Plugin | Operation | Kind | Tier | User-initiated only | In state calls | OS consent |
| --- | --- | --- | --- | --- | --- | --- |
| system-monitor | `system.metrics` | `systemMetrics` | low | no | yes | none |
| power | `power.request` | `power` (per action) | low / medium / high | yes | no | Automation (macOS restart, shutdown); polkit (Linux) |
| screen-capture | `screen.capture` | `screenCapture` | high | yes | no | Screen Recording (macOS); portal (Wayland) |
| clipboard | `clipboard.writeText` | `clipboardWrite` | medium | yes | no | none (Wayland may refuse) |

## 8. Open questions for the owner

1. **Which, if any, to build.** System metrics is low risk and unlocks the
   system-monitor plugin and keys showing CPU or memory; the other three
   change the machine or touch user data.
2. **Power from schedules.** Proposed never. `lock` at a time of day is a
   plausible wish; allowing it would be a separate decision.
3. **Screen capture's OS grant.** Granting macOS Screen Recording to the
   agent makes the agent able to capture for any purpose; this proposal
   limits that by code and tests, not by the OS. Acceptable?
4. **Clipboard read.** Proposed never. Confirm.
5. **API level.** These would be plugin API 1.3 (new operations, new
   kinds); each plugin would declare `^1.3`.
6. **Where they live.** Each could instead stay a built-in module (as
   `system` does today) and not be offered to plugins at all; the
   catalog's plugins would then not be built.

Before any implementation, each OS row marked "to be verified" needs a
short spike on that OS.
