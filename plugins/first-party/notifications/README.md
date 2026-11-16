# deckpilot.notifications

The first-party Notifications plugin: the replacement of the built-in
`notifications` module (`modules/notifications`) and the first plugin of
the **default set** (`docs/marketplace/module-migration.md` §4.1, PR P10).
Bundled with class `default` (`../bundled.json`): seeded, enabled and cut
over at the first start of a build with plugins, offline, on a fresh
install and on an upgrade alike, unless the user removed or disabled it.

It shows the notifications workflows ask for, and a summary of how a
workflow run went. One permission, `notifications` (low tier), with a
routing: the agent, not the plugin, decides where a notification appears,
from the plugin's settings.

## The contract

Both actions of the built-in, under the same name, so the alias
`notifications.<action>` → `deckpilot.notifications.<action>` (shipped in
`ACTION_ALIASES`) is valid. Two tests keep it that way:

- `crates/deckpilot-modules/tests/contracts.rs` (`first_party`): the
  manifest, converted as the agent registers plugin actions, against the
  built-in's definitions: `alias::incompatibilities` (§5.3) for both
  aliases, and beyond it identical definitions but for the documented
  differences of every first-party plugin (`cancellable`; defaults only on
  optional parameters). Also the settings (the module's three, key for key,
  with their kinds, defaults and portability), the routing by exactly them,
  no variables, and the one low-tier permission.
- `apps/agent/tests/plugins_notifications.rs`: 14 scenarios through the
  real agent and sandboxed host against the built-in in the same
  conditions (the app open or closed, desktop notifications working or
  refused, the settings), with native notifications recorded to a file:
  the same status, message, outputs, error code and native notification.
  The same file covers the first launch, the user's disable and uninstall
  across restarts and an app update, and an alpha.3 upgrade with an older
  build on the same data.

| Built-in | Plugin | Definition | Results |
| --- | --- | --- | --- |
| `notifications.show` | `deckpilot.notifications.show` | compatible: `cancellable`; `severity` and `critical` optional with the same defaults | identical, but for the differences below |
| `notifications.workflow-summary` | `deckpilot.notifications.workflow-summary` | compatible: as `show`, for `onlyOnFailure` and `critical` | identical; the run summary comes from `run.context` (G8), the workflow's name with it |

## Settings

The module's, key for key, all portable: **Desktop notifications**
(`desktop`, on), **Status bar** (`statusBar`, on), **Show from**
(`minimumSeverity`, info). The manifest's `notifications` permission names
them as its `routing` (G9), so the **agent** reads them at every
notification; the plugin only learns where a notification was shown
(`Context::notify_delivered`). An upgrade copies the module's saved values
once (the module's row stays as it was, for an older app).

## Differences from the built-in

All on the presentation, limits or failure paths:

| What | Built-in | Plugin | Why |
| --- | --- | --- | --- |
| Title as shown | the title | "Notifications: " and the title | every plugin notification is prefixed with the plugin's name (the sandbox's rule, kept) |
| Length shown | title 100, text 500 characters | title 64, text 300 characters (cut with "…"); the node's message and outputs still name the 100-character title | what `notify` takes (ABI 1 limits, unchanged) |
| Rate | none | 5 per minute; the 6th is dropped: the node succeeds with `shown: false` and the reason "more than 5 notifications a minute", or fails when `critical` | the plugin rate limit; a higher, manifest-declared rate is medium tier, which a default plugin must not need |
| Reason when below **Show from** | "less important than the module's setting" | "less important than the plugin's setting" | the setting is the plugin's |
| Reason when both destinations are off | "notifications are turned off in Settings › Notifications" | "notifications are turned off in Settings › Plugins › Notifications" | the same |
| Workflow name in a summary | the profile's workflow, full length | from `run.context`, cut to 128 characters (then to the title's 100) | the run context's bound |
| Stopping | not interruptible | cancellable, like every plugin action | the agent can always stop a plugin |

A user who disables or uninstalls the plugin gets the `notifications.*`
actions unavailable (no fallback to the built-in, §5.2); the choice sticks
across restarts and app updates (a tombstone) until the user enables or
installs it again. A user who had turned the built-in module off gets the
plugin turned off too, once, at the upgrade.
