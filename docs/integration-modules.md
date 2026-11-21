# Integration modules

Modules that connect DeckPilot to tools on this computer and elsewhere. They
are ordinary feature modules (see [Feature modules](modules.md)): compiled
in, listed in Settings › Modules, each with its own manifest, capabilities,
settings and action types. Their action types appear in the Action Library
and the workflow editor's catalog automatically; nothing is added to the
Action Library until you create an action.

The built-in integrations are System, HTTP and Notifications. Git, Docker,
Workspace, GitHub, GitLab and Jira are no longer built-in modules: they
come from DeckPilot's own plugins ([below](#moved-to-first-party-plugins)).

Every action resolves machine-specific values (applications, folders,
programs, files) through logical resources bound on this computer (see
[Machine resources](data-model.md#machine-resources)), so an exported workflow works on
another machine once its resources are bound there.


Common to all of them:

- **Effects.** Each action type declares whether it only reads (`readOnly`),
  opens something (`opens`) or changes state (`modifies`); the catalog and
  the inspector show it.
- **Error codes.** A failed action reports a stable code with its message
  (`ActionResult.code`, `StepError.actionCode` in workflows). Codes shared
  by every module: `not-permitted` (capability missing), `no-active-project`,
  `resource-role-missing`, `resource-unbound`, `resource-kind-mismatch`,
  `program-not-found`, `spawn-failed`, `cancelled`, `timed-out`.
- **Sensitive outputs.** Outputs such as a program's output or a response
  body are passed to later nodes but never stored in execution history or
  sent in live run events.
- **Programs** are started without a shell, with an argument list, a
  minimal environment (`PATH`, `HOME`, locale and the system variables
  Windows needs; nothing else of the agent's environment), bounded output
  and a time limit, through `ExecutionContext::run_process`. Cancelling the
  run or reaching the time limit stops the program and everything it
  started.

## System (`system`)

Opens applications, web pages, files and folders, runs commands, checks
whether programs are running and shows CPU, memory, network and disk use.

Capabilities: machine resources, open applications/files/pages, run
programs, see running programs.

Setting: **Allow running commands** (`allowCommands`, off by default). It
is machine-specific: never exported, and an import cannot turn it on.

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `system.open-app` | opens | `application` (application resource), `arguments` (one per line) | `application` |
| `system.open-url` | opens | `url` (`http`, `https` or `mailto`) | `url` |
| `system.open-file` | opens | `location` (resource), `path` (relative), `application` (optional application resource) | `path` |
| `system.open-directory` | opens | `location` (folder resource), `path` (optional relative subfolder) | `path` |
| `system.run-command` | modifies | `program` (resource), `arguments` (one per line), `workingDirectory` (optional folder resource; home folder otherwise), `timeoutSeconds` (1–3600, 60), `failOnNonZero` (on) | `exitCode`, `stdout`*, `stderr`*, `outputTruncated`, `durationMs` |
| `system.process-status` | read-only | `name` (program name; case and `.exe` ignored) | `running`, `count`, `pids` |
| `system.cpu` | read-only | `warningPercent` (0–100, 70), `criticalPercent` (0–100, 90) | `usagePercent`, `level` |
| `system.memory` | read-only | `warningPercent` (70), `criticalPercent` (90) | `usedPercent`, `usedBytes`, `totalBytes`, `level` |
| `system.network` | read-only | `interface` (optional name, case ignored; all but loopback, added up, when empty) | `receivedPerSecond`, `transmittedPerSecond` (bytes/s), `interfaces` |
| `system.disk` | read-only | `location` (optional folder resource; the home folder otherwise), `warningPercent` (% free, 20), `criticalPercent` (% free, 10) | `freePercent`, `freeBytes`, `totalBytes`, `mountPoint`, `level` |

\* sensitive: available to later nodes, not kept.

Opening uses the platform's launcher, never a shell: `open` on macOS
(`-a` for an application path or name, `-b` for a bundle identifier),
`xdg-open` on Linux (an application is started directly, a `.desktop` id
with `gtk-launch`), and on Windows `rundll32 url.dll,FileProtocolHandler`
for pages and files, `explorer` for folders and the executable itself for
applications. Other URL schemes (`file:`, `javascript:`, custom handlers)
are refused because they can start programs. Relative paths cannot leave
their resource (`..`, absolute paths and drive letters are refused), and
what they lead to must be inside it too: a link (symlink) inside the
resource that points outside it is refused with `invalid-path`, a link to
nothing is `not-found`; links that stay inside, and a resource folder that
is itself a link, are fine. An
opened application is independent of the workflow: cancelling does not
close it.

`system.run-command` runs only a program bound as a resource — an absolute
path or a name found on `PATH` — so an imported workflow cannot choose what
runs. Arguments are passed as they are, one per line: nothing is split on
spaces, quoted or expanded. Output is kept up to 64 KiB per stream
(`outputTruncated` tells when more was produced). The node is cancellable
and not retry-safe. In a workflow a node runs at most 10 minutes, so a
longer `timeoutSeconds` only applies when the action runs from a key.

**Monitor keys.** `system.cpu`, `system.memory`, `system.network` and
`system.disk` show their value under the key's title as its live state:
`37%` with the use as a bar (also with its recent values as a chart) for
CPU, `9.9/16 GiB` used and installed with the used share as a bar for
memory, `↓ 1.2 MB/s` and `↑ 56 kB/s` received and sent for the network,
and `55% used` with the used share as a bar for a disk. The indicator is
green, amber from the warning threshold and red from the critical one
(`level`: `ok`, `warning`, `critical`); a critical threshold below the
warning one (above it for free space) is refused when saving. Pressing the
key reports the value as the message and outputs, e.g. `memory 62% used
(9.9 of 16.0 GiB)`. One sampler per module (`src/monitor.rs`) reads the
values with `sysinfo` for every key: a value is read at most every 1.5 s
and shared, keys refresh every 2 s (disk: 30 s), and nothing is read while
no such key is shown. CPU use and throughput are differences between two
readings; the first one, and one after a pause of more than 10 s, takes a
second reading about 250 ms later, so a value is never an average over the
pause (that wait is cancellable). The disk is the mounted volume with the
longest mount point containing the folder. These actions need no
capability (a folder resource needs machine resources); they read only
totals, never per-process data.

Error codes: `invalid-url`, `invalid-path`, `invalid-arguments`,
`not-found`, `commands-disabled`, `exit-nonzero`, `unsupported-target`,
`open-failed`, `unavailable` (the computer does not report the value), and
the shared codes.

Opening cannot detect whether the opened program then shows what was
asked (a launcher reports only that it started); a launcher still running
after 10 s counts as success.

## HTTP (`http`)

HTTP requests and health checks.

Capabilities: send network requests, use stored credentials.

Settings: **Allow this computer and private networks** (`allowPrivateNetworks`,
off), **Private hosts allowed** (`privateHosts`, one per line, e.g.
`localhost` or `*.internal`) — both machine-specific: never exported, an
import cannot open them — and **Largest response kept** (`maxResponseKb`,
1–10240 KiB, 1024). Settings › HTTP also manages **credentials**.

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `http.request` | modifies | `method` (GET, POST, PUT, PATCH, DELETE), `url`, `query` (`name=value` per line), `headers` (`Name: value` per line), `body`, `bodyType` (json, text), credential, `timeoutSeconds` (1–300, 30), `followRedirects` (on), `failOnHttpError` (on), `extract` (JSON pointer) | `status`, `ok`, `contentType`, `headers`, `body`*, `value`*, `size`, `truncated`, `url`, `redirects`, `durationMs` |
| `http.health-check` | read-only | `url`, `expectedStatus` (`200`, `2xx`, `200-299`, `200,204`; `2xx`), `contains`, `timeoutSeconds` (1–120, 10), `failWhenUnhealthy` (on), credential | `healthy`, `status`, `reason`, `durationMs` |

The credential parameters are `credential` (a stored credential's name),
`authScheme` (`bearer`, `basic` with `user:password`, `header`, `query`) and
`authName` (header or parameter name; `X-API-Key`, `api_key`).
\* sensitive: available to later nodes, not kept.

A JSON body is checked before sending and sent as `application/json`;
`extract` reads one value from a JSON response with a JSON pointer (RFC
6901, e.g. `/data/0/id`), so later nodes can use it without any expression
language.

**Network boundary (SSRF).** Only `http` and `https`. Every address a host
resolves to is checked: loopback, private (RFC 1918), shared (100.64/10),
link-local (169.254/16, including cloud metadata services), unique local and
link-local IPv6, IPv4-mapped and NAT64 forms of those, multicast, reserved
and documentation addresses are refused unless allowed for all hosts or the
host is listed. A name resolving to both a public and a private address is
refused. The connection then goes to exactly the checked addresses, so DNS
cannot change between the check and the connection. Redirects (at most 5)
are followed one at a time and checked the same way; a redirect from
`https` to `http` is refused. No proxy is used.

**Credentials** are stored per computer (Settings › HTTP › Credentials) in
the encrypted database, listed by name and description only, and never in a
workflow or an export. A request sends one only over `https` (or to this
computer), never along a redirect to another origin. Headers and query
parameters that hold secrets (`Authorization`, `Cookie`, names containing
`token`, `secret`, `api-key`, `password`, `auth`, …) cannot be written into
a workflow, and URLs with a user name or password are refused, so secrets
always go through a stored credential. Secret response headers
(`Set-Cookie`, tokens) are shown as `[redacted]`; messages and the `url`
output leave out the query string. Nothing logs headers or tokens.

**TLS** uses the operating system's certificate store (rustls with the
platform verifier). Certificate verification cannot be turned off; a
refused certificate fails with `tls-failed`.

**Limits, timeouts and cancellation.** Responses are read up to the
module's limit (`truncated` tells when cut). `timeoutSeconds` bounds the
whole request including redirects; cancelling the run stops the request at
once. `http.request` is not retry-safe (it may change data);
`http.health-check` is retry-safe, idempotent and by default fails when
unhealthy (`unhealthy`), or reports `healthy: false` when told not to fail.

Error codes: `invalid-url`, `credentials-in-url`, `invalid-header`,
`invalid-query`, `sensitive-header`, `invalid-body`, `blocked-address`,
`dns-failed`, `connection-failed`, `tls-failed`, `too-many-redirects`,
`insecure-redirect`, `insecure-credentials`, `request-failed`,
`http-status`, `invalid-response`, `unhealthy`, `credential-missing`, and
the shared codes.

## Notifications (`notifications`)

Shows notifications from workflows: as native desktop notifications and in
the configuration app's status bar.

Capabilities: read profile (for the workflow's name), show notifications.

**A plugin by default (builds with plugins only).** In a build with the
experimental plugin marketplace enabled, notifications come from the
bundled first-party plugin `deckpilot.notifications`
([its README](../plugins/first-party/notifications/README.md),
[module-migration.md](marketplace/module-migration.md) §4). It is part of
the default set: installed and enabled at the first start, offline, with
its one permission (show notifications), on a new computer and on an
upgrade alike. Nothing you saved changes: keys, workflow nodes and actions
keep their `notifications.*` types and run in the plugin, with the same
parameters, outputs and messages; your settings below are copied to the
plugin's (Settings › Plugins › Notifications), where you change them from
then on. Its notifications are titled "Notifications: …", at most 64
characters of title and 300 of text reach the desktop, and at most 5 are
shown per minute (more are dropped, and the node goes on unless it is
critical). If you turned this module off, the plugin starts off too. If
you disable or uninstall the plugin, it stays so across restarts and
updates, and the `notifications.*` actions are unavailable rather than
silently run by this module. Releases up to v0.6.0 have no plugins: there, and
in an older app on the same data, this module runs them as before.

Settings (portable): **Desktop notifications** (on), **Status bar** (on),
**Show from** (`minimumSeverity`: info, success, warning, error; info).

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `notifications.show` | opens | `title` (one line, ≤100), `body` (≤500), `severity` (info, success, warning, error), `critical` (off) | `shown`, `desktop`, `statusBar`, `reason` |
| `notifications.workflow-summary` | opens | `title` (default: the workflow's name), `details` (lines added below), `onlyOnFailure` (off), `critical` (off) | same, plus `summary` |

Title and text can contain outputs of earlier nodes (`{{ steps.<node>.output.<key> }}`).
They are plain text: control characters and bidirectional overrides are
removed, lengths are bounded, and nothing is rendered as HTML (markup is
escaped for Linux notification services, React escapes it in the app).
The node's message names the title only, so the text is not kept in
history.

`workflow-summary` reports how the run is going when the node runs: actions
that succeeded, failed (with their node ids), were skipped or cancelled, and
the time so far — never messages or outputs. Its severity follows the
result (error when something failed). With `onlyOnFailure` a clean run
shows nothing.

**Delivery.** The agent shows a notification in the status bar of every
open app window (protocol 6 `notification` event) and natively:
`osascript` on macOS (`display notification`, text passed as arguments),
`notify-send` on Linux, a PowerShell toast on Windows (text passed in
environment variables and inserted as XML text nodes). A notification that
cannot be shown anywhere — the app is closed and desktop notifications are
off, denied or unavailable — does not stop the workflow: the node succeeds
with `shown: false` and the `reason`, unless it is `critical`
(`notification-not-shown`). Notifications less severe than **Show from**
are not shown and do not fail. macOS cannot report that notifications are
denied for the helper; they are then counted as shown.

Error codes: `notification-not-shown`.

## Moved to first-party plugins

The built-in `git`, `docker`, `workspace`, `github`, `gitlab` and `jira`
modules (and `projects`, see [Feature modules](modules.md)) were removed
(WP-C5, WP-C6 of
[marketplace/module-migration.md](marketplace/module-migration.md), §11 D1
and D2). Each is replaced by a first-party plugin bundled with the app
(class `migration` in `plugins/first-party/bundled.json`), which is now
their only implementation:

| Former module | Plugin | Documentation |
| ------------- | ------ | ------------- |
| `git` | `deckpilot.git` | [plugins/first-party/git/README.md](../plugins/first-party/git/README.md) |
| `docker` | `deckpilot.docker` | [plugins/first-party/docker/README.md](../plugins/first-party/docker/README.md) |
| `workspace` | `deckpilot.workspace` | its manifest (`plugins/first-party/workspace/manifest.json`) |
| `github` | `deckpilot.github` | [plugins/first-party/github/README.md](../plugins/first-party/github/README.md) |
| `gitlab` | `deckpilot.gitlab` | [plugins/first-party/gitlab/README.md](../plugins/first-party/gitlab/README.md) |
| `jira` | `deckpilot.jira` | [plugins/first-party/jira/README.md](../plugins/first-party/jira/README.md) |

**Your keys and workflows.** Nothing you saved is rewritten. Keys, workflow
nodes and actions keep their `git.*`, `docker.*`, `workspace.*`,
`github.*`, `gitlab.*` and `jira.*` types, which the core maps to the
plugin's action types (`ACTION_ALIASES`,
`crates/deckpilot-core/src/alias.rs`). An upgrade installs a plugin only
where its module was in use, copies the module's settings and asks for
what needs your consent in Settings › Plugins: the token or credential of
GitHub, GitLab and Jira, the server you had entered, running `git` or
`docker`, starting and stopping Compose services. Until the plugin serves
(not installed, disabled or uninstalled by you, or waiting for your
consent), such an action is unavailable with the reason, e.g. "`git.status`
now comes from Git (`deckpilot.git`): … waits for your consent", never an
unknown type, and nothing runs it in its place: there is no built-in to
fall back to. Once you allow, enable or restore the plugin, the same keys
work again unchanged. Settings, credentials, projects and resource
bindings are kept.

**Servers you enter.** All three network plugins are read-only (`GET`
only) and reach a server only after you allowed exactly that server; one
on a private network needs its own private-network consent, and this
computer is never reachable. Opened pages stay on the same server.

- **GitHub**: `https://api.github.com` by default; for GitHub Enterprise
  Server set the API URL to `https://<host>/api/v3`. A token (a
  fine-grained token with read access is enough) in the plugin's
  credential slot, sent as bearer.
- **GitLab**: GitLab.com (`https://gitlab.com/api/v4`) by default, or a
  self-managed GitLab under its path. A token sent as `PRIVATE-TOKEN`.
- **Jira**: no default; the site you enter, a Jira Cloud site
  (`https://your-team.atlassian.net`, deployment `cloud`: REST API 3, the
  credential `email:api-token` as Basic) or a Data Center under its path
  (deployment `data-center`: REST API 2, a personal access token as
  Bearer).

A plain `http` address, which the built-ins accepted on this computer, is
not reachable for a plugin. What else differs from the former built-ins
(checks on save, failure messages, limits) is listed in each README.

## Testing

The former built-ins' tests (`integration_services.rs`, `live_smoke.rs`)
were removed with them. The plugins are checked by:

- **Contracts.** `crates/deckpilot-modules/tests/contracts.rs` holds each
  plugin's manifest to the frozen 0.9.0 contract of the built-in it
  replaced (`crates/deckpilot-modules/legacy/<id>.json`), so the aliases
  stay valid.
- **Unit tests** of each plugin, e.g.
  `cd plugins/runtime && cargo test -p deckpilot-github`.
- **Through the real agent** (feature `plugins`, `test-hooks`):
  `apps/agent/tests/plugins_{git,docker,workspace,projects,github,gitlab,jira}.rs`
  run every action in the sandboxed host on canned answers or fake
  programs, never a real service; for GitHub, GitLab and Jira they are
  regression tests against the answers the built-ins gave.
  `plugins_cutover*.rs` run the upgrade from a configuration of the
  built-in, and `core_without_plugins.rs` the core with every plugin off
  or uninstalled ([development.md](development.md)).
