# Plugin API 1.1 and 1.2: time, randomness and key state

Status: **design**, implemented by the pull requests named in each section.
Everything here is part of the experimental plugin runtime: off by default
behind the cargo feature `plugins`, `--enable-plugins` and
`VITE_DECKPILOT_PLUGINS=1`. A default build is unchanged.

Plugin API 1.0 gives a plugin no clock, no random source and no way to
change what a key shows (`sdk/lifecycle.md`, "Not in ABI 1"). This adds
three narrow capabilities, each reviewed on its own:

| API level | Addition | Permission | Pull request |
| --- | --- | --- | --- |
| 1.1 | `time.now` | none (always allowed while the plugin is enabled) | "time and randomness" |
| 1.1 | `random.bytes` | none (always allowed while the plugin is enabled) | "time and randomness" |
| 1.2 | key state: the optional export `dp_state` | `device` with `state: true` (low tier) | "key state": the renderer's marks, the runtime and SDK, the agent |

Not added, on purpose: pixels, fonts, HTML or SVG from a plugin; timers
and background work; animation; any knowledge of devices, keys or their
positions; file system, process, screen or input access. Section 5 lists
what is left as follow-up proposals.

## 1. Compatibility

The guest ABI stays **1** and host protocol stays **1**. Nothing a plugin
built for API 1.0 sends or receives changes, byte for byte.

- **New operations** (`time.now`, `random.bytes`) are new variants of
  `CapabilityRequest`. Only a guest that uses them sends them. A host or
  agent that predates them answers `unsupported-op`, as for every unknown
  operation; the SDK returns that as a `CapabilityError` the plugin can
  handle.
- **New export** (`dp_state`). A host that predates it never calls it,
  and the package check allows unknown exports, so a module that has it
  installs and runs on an older agent; its keys just stay static. A host
  that knows it refuses a `dp_state` of another type at load, like the
  other optional exports.
- **New replies need no feature bit.** `dp_features` exists for additions
  to replies of operations older guests already call (`NOTIFY_DELIVERY`,
  #265). The new operations have new replies, read only by guests that ask.
- **The manifest decides whether an agent runs the plugin.** A plugin
  that cannot work without an addition declares `engines.pluginApi`
  `"^1.1"` or `"^1.2"`; an agent whose `PLUGIN_API` is lower reports the
  version as `incompatible` and does not install it (the existing check).
  A plugin that only benefits from an addition keeps `"^1.0"` and handles
  `unsupported-op`. The agent's `PLUGIN_API` becomes 1.1 and then 1.2.
- **SDK.** `deckpilot_plugin_sdk::API_LEVEL` stays 1.0, the level a
  plugin needs by default, so existing plugins and their tests are
  unchanged; `LATEST_API_LEVEL` is what the SDK implements (1.1, then
  1.2). A plugin's own requirement is `Plugin::api_level()`: 1.0 unless
  it declares more with `Plugin::requires_api`.
  `testing::manifest_mismatches` compares the manifest's
  `engines.pluginApi` with that. A plugin whose keys merely look better
  with state keeps `"^1.0"`: on an older agent they stay static.

## 2. `time.now` (API 1.1)

```json
{ "op": "time.now", "args": {} }
→ { "ok": { "unixMs": 1794838800123, "utcOffsetMinutes": 60 } }
```

- `unixMs`: the agent's wall clock, milliseconds since the Unix epoch
  (UTC). Not monotonic: it follows the system clock.
- `utcOffsetMinutes`: the local UTC offset at that instant, east
  positive, within ±1,440. No time zone name and no rules: a plugin that
  needs the offset of another instant asks again then.
- SDK: `Context::now() -> Result<Timestamp, CapabilityError>` with
  `unix_ms`, `utc_offset_minutes`, `unix_seconds()`, `local_minutes_of_day()`
  and `local_weekday()`.

**Permission: none.** Every HTTP response already carries a `date`
header, so the time is not new to a plugin with a network grant, and the
time alone identifies nobody. The offset is the one new fact: it places
the user in one of about 38 zones. That is far less than the IP address a
server the plugin talks to sees, and a plugin can send it only where the
user granted network access. Quiet hours, "once a day" and local
countdowns need it, so asking consent for it would put a prompt on nearly
every plugin for no protection. It is therefore always allowed while the
plugin has grants (is enabled), like `storage.*` and `settings.get`.

**Limits.** 20 calls per second per plugin (`RateLimits::time`), inside
the 200 per second overall limit and the 256 calls of one invocation;
exceeded: `throttled`. One reply is under 80 bytes. Millisecond
resolution, each call a round trip between two processes: too coarse to
be a timing side channel inside the sandbox.

**Audit.** One record per decision (`time.now`, no target), like every
operation.

## 3. `random.bytes` (API 1.1)

```json
{ "op": "random.bytes", "args": { "length": 16 } }
→ { "ok": { "bytes": "base64, standard alphabet, padded" } }
```

- `length`: 1 to 64 (`MAX_RANDOM_BYTES`); anything else is `bad-request`.
- The bytes come from the operating system's CSPRNG in the agent, fresh
  for every call. The agent keeps no generator state a plugin could learn
  or influence, and no two plugins ever see the same bytes.
- SDK: `Context::random_bytes(n)`, `random_u64()` and
  `random_below(bound)` (unbiased, by rejection).

**Permission: none.** Randomness reveals nothing and reaches nothing.
Always allowed while the plugin is enabled.

**Limits.** 64 bytes per call; 4,096 bytes per plugin per minute as a
token bucket (`RateLimits::random_bytes`, burst 4,096), which bounds one
run as well; exceeded: `throttled`. Shuffling a deck needs 8 bytes for a
seed. Audited as `random.bytes` without the length or the bytes.

## 4. Key state (API 1.2)

**The plugin describes, the host draws.** A plugin returns a small typed
description of a key. The agent's existing renderer, which knows every
device's resolution and image format, draws it. A plugin sends no pixels
and learns nothing about devices, so device independence and the fallback
for devices without key displays cost nothing, and no decoder in the
agent reads untrusted bytes.

### 4.1 Why a pull (`dp_state`) and not state in an action's result

- A key must show its state before anyone presses it: after the agent
  starts, after a page or profile switch.
- One action changes several keys (a flipped card, a paused timer), and
  time changes them without any action (a countdown).
- The agent already pulls state for built-in modules
  (`ActionHandler::state_policy` and `button_state`, scheduled by
  `apps/agent/src/service/states.rs`): only for keys shown on a device,
  off the agent's task, cached, redrawn only on change. A plugin action
  becomes one more provider of that pipeline, so there is no second
  scheduler, cache or drawing path. The state travels in the same
  `KeyFace` and is drawn by the same renderer (`deckpilot-render`) the
  configuration app uses for its previews. Like the state of a built-in
  module, it is shown on devices (physical and mock); the app's layout
  preview draws keys without running anything, so it shows the static
  face, for plugins and built-ins alike.

State attached to results would need a store of last states per key, an
answer for keys never pressed, and still a pull for time.

### 4.2 ABI

Optional export `dp_state: (ptr: i32, len: i32) -> i64`. Input: the
`InvokeInput` JSON of `dp_invoke` (`action`, the key's normalized
`params`). Output: `ptr << 32 | len` of a `StateOutcome` JSON, at most
4 KiB (`MAX_STATE_OUTPUT`):

```json
{ "ok": { "label": "12:40", "detail": "Focus", "indicator": "running",
          "progress": 48, "toggle": true, "accent": "green",
          "glyph": "timer", "badge": "3", "series": [10, 40, 35, 80] } }
{ "error": { "code": "not-configured", "message": "Choose a city" } }
```

Every member of the state is optional; `{}` leaves the key as the user
designed it.

| Member | Bound | Drawn as |
| --- | --- | --- |
| `label`, `detail` | 32 characters each, no control characters | title and second line |
| `indicator` | `ok`, `warning`, `error`, `running`, `unknown` | corner mark (as for built-ins) |
| `progress` | 0 to 100 | bar along the bottom |
| `toggle` | boolean | mark along the top |
| `accent` | `blue`, `green`, `amber`, `red`, `purple`, `teal`, `grey` | the key's background, from the agent's fixed palette (each with a text colour of sufficient contrast). The user's own key colours win |
| `glyph` | one of the host's built-in glyphs (4.5) | a simple shape above the title; a key with an icon the user chose keeps it |
| `badge` | 4 characters (ASCII) | a small label in the top left corner |
| `series` | at most 32 integers 0 to 100 | a sparkline behind the texts |

Unknown members, unknown names and values out of bounds make the whole
state invalid (`invalid-state`); texts are cleaned again by the agent
(`ButtonState::sanitized`). Host protocol: `state` (supervisor to host)
and `stateResult` (host to supervisor) beside `invoke` and `result`; a
module without the export answers `unsupported`.

SDK: `Action::state(fn(&mut Context, &Args) -> Result<KeyState, PluginError>)`
and the `KeyState` builder; the `plugin!` macro exports `dp_state`.

### 4.3 When the agent asks

For an action whose manifest entry has `state` (in the schema since 1.0:
`refresh: "onEvents" | "every"`, `intervalSeconds`), for a key bound to
it that **is shown on a device now**:

- when the key appears, and after something happened in DeckPilot (a key
  press, a finished action or workflow, a project switch, a reload):
  the existing invalidation, so the plugin's own action is followed by a
  fresh state without any push from the plugin;
- with `every`: again each `intervalSeconds`, never more often than the
  **floor of 5 s** for plugins (built-ins keep 1 s) and at most 24 h.

A plugin cannot ask for a redraw and has no timer; both are follow-ups
(section 5).

### 4.4 What a state call may do

Inside `dp_state` the broker allows only `storage.get`, `storage.keys`,
`settings.get`, `time.now` and `log` (`Session::authorize_state`; every
other operation: `not-permitted`, audited). A state call cannot reach the
network, notify, open a page, write storage or publish variables: it is
read-only by construction, which is also why the manifest requires
`effect: readOnly` for an action with `state`. Data from the network is
fetched by an action and read from storage here.

Permission: the existing `device` kind with `state: true`
(`device:state`, low tier; the broker's `keyState`). Not declared, not
granted or revoked: the broker refuses before the host is asked, and the
key stays static.

The manifest's rule that `state` needs an action whose `node.effect` is
`readOnly` (the default) is kept. It dates from a design in which the
action itself was polled; with a state call that only reads it may no
longer be needed, so that an action that `modifies` could show state too.
That is an open question (section 7), not decided here.

### 4.5 Glyphs

A closed set drawn by the agent as vector shapes at the device's
resolution: `check`, `cross`, `play`, `pause`, `stop`, `timer`, `bell`,
`up`, `down`, `dot`, `circle`, `square`, `diamond`, `triangle`, `star`,
`heart`. An unknown name invalidates the state. The plugin's packaged
`assets.keyIcons` (`device` permission, `icons: true`) are a follow-up:
they need a bounded decode of package images in the agent, which deserves
its own review.

### 4.6 Budgets

Per plugin unless stated.

| What | Bound | When exceeded |
| --- | --- | --- |
| Refresh interval | at least 5 s, at most 24 h | clamped |
| State calls at a time | 1 per plugin, in the plugin's one host process and lane, so it also never overlaps an action; they share the agent's 4 provider slots with built-in modules, longest waiting first | the plugin's next key waits its turn, at most 6 s, then keeps what it showed |
| Waiting in the lane | a state call is never queued behind a running or waiting action | the key keeps what it showed; tried again at the next interval |
| One state call | 2 s wall clock, 250 ms CPU between host calls, a tenth of an action's fuel, 8 capability calls, 4 KiB of output | the call fails; past the wall clock the host process is stopped (counts for the crash-loop breaker) |
| Failures in a row (error, timeout, invalid, crash) | 3 for a key: no call for 5 s, doubling up to 5 min | the key shows the static face with the error mark ("unavailable") |
| Crashes | the existing breaker: 5 in 10 min disable the plugin | all its keys are static |
| Redraws | only when the typed state changed | nothing is sent to a device |

Nothing on the agent's task waits for a plugin: the call runs on a
provider thread, which waits for the plugin's lane with the same time
limit. A press of a key is not delayed by more than one state call (2 s
at worst), and a state call never fills the action queue.

### 4.7 Fallbacks

| Situation | Key shows |
| --- | --- |
| Device or control without a display | nothing; no state call is made |
| Plugins off, plugin disabled, quarantined, uninstalled, unsupported platform (Windows: plugins do not run) | the static face |
| `device:state` not granted or revoked | the static face (asked again after 30 s) |
| Module without `dp_state`, an action without a state handler, or an agent older than API 1.2 | the static face |
| Error, timeout, invalid or oversized state | the static face with the error mark; back-off |
| First state pending | the static face with the loading mark |

## 5. Follow-up proposals (not in these pull requests)

- **Timers**: the `background` permission made effective
  (`contributes.background`, a host-invoked action at an interval), so a
  countdown can notify without a key press.
- **`state.invalidate`**: a rate-limited request to pull the plugin's
  shown keys again, for changes that no DeckPilot event announces.
- **Packaged key icons** (`assets.keyIcons`, `device` `icons: true`).
- **Animation**: host-drawn only (a pulsing indicator, a host-side
  countdown between pulls); a plugin never drives frames.
- **Device facts** (`supports.device`, grid size) for games.

- **State for actions that are not `readOnly`** (section 4.4).

## 6. What a malicious plugin can do with all of it

Learn the time and the UTC offset; draw bounded text, one of 16 shapes,
one of 7 colours, a bar and a small chart on keys the user bound to its
own actions; keep one provider thread busy for 2 s at a time until the
breaker disables it. It cannot draw on another key, imitate a system
dialog (keys only, the user's bindings only), spend device bandwidth
(the agent redraws only on change, at most once per 5 s per key), block
the agent's task or other plugins, or reach anything from a state call
that it could not already read.

## 7. Open questions for the owner

1. **`state` on an action that `opens` or `modifies`.** The manifest
   refuses it today ("button state needs a readOnly action"). A state
   call cannot change anything whatever the action does, so the rule
   could go; an agent below API 1.2 would then refuse such a manifest,
   which a plugin would have to declare with `"pluginApi": "^1.2"`.
2. **The floor of 5 s.** Enough for minutes and data from the network; a
   seconds countdown on a key would need a host-drawn countdown between
   pulls (animation, section 5) rather than a lower floor.
3. **The palette and the glyph set.** Seven colours and 16 shapes are a
   first cut; adding a name is a change of the renderer, the ABI and the
   SDK together.
4. **The UTC offset without consent** (section 2): the one new fact about
   the user that every enabled plugin can read.
5. **Live state in the configuration app.** The layout preview draws
   static faces for built-in modules and plugins alike; showing live
   state there needs the agent to send faces to the app.
