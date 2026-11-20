# Capabilities: the `Context` API

Inside an action the module can reach the outside world only through
`deckpilot_plugin_sdk::Context` (`plugins/runtime/guest/plugin-sdk/src/context.rs`).
Each call is one capability request (`CapabilityRequest`,
`plugins/runtime/crates/abi/src/capability.rs`) through the single import
`deckpilot_v1.host_call`; the host forwards it to the agent, where the
permission broker decides and an executor carries it out. The plugin id is
never part of a request: the agent knows which plugin's host is asking.

At most 256 capability calls per invocation
(`MAX_HOST_CALLS_PER_INVOCATION`; the SDK refuses the 257th locally as
`Throttled` instead of letting the host trap). `Context::calls()` counts
them.

## Errors

Every wrapper returns `Result<_, CapabilityError>`. `kind()` is a
`CapabilityErrorKind`:

| Kind | From the host's `CallErrorCode` | Typical cause | `?` in a handler becomes |
| --- | --- | --- | --- |
| `NotPermitted` | `not-permitted` | Not declared, not approved, revoked; plugin disabled | `not-permitted` |
| `Throttled` | `throttled` | A rate limit; more than 256 calls | `throttled` (retryable) |
| `BadRequest` | `bad-request` | Malformed or too large; a forbidden header; a variable of the wrong type | `request-failed` |
| `Unsupported` | `unsupported-op` | An operation this host does not know | `unsupported` |
| `Failed` | `failed` | Allowed but failed: network error, storage quota (`storage-quota…`) | `request-failed` (retryable) |
| `Unavailable` | `unavailable` | No secret stored for a slot; a call during `init`/`shutdown`; an interrupted call; native code without a host | `request-failed` (retryable) |
| `MalformedReply` | | The reply did not have the expected shape | `request-failed` |

An `http.fetch` that failed on the network, or whose redirect the broker
refused for a reason the built-in modules' transport has a code for, also
has a `network_kind()`, a `NetworkErrorKind`. `?` then reports that code
instead of `request-failed` or `not-permitted`, the same code as the
built-in module the plugin replaces:

| `NetworkErrorKind` | Code | Retryable |
| --- | --- | --- |
| `DnsFailed` | `dns-failed` | yes |
| `ConnectionFailed` | `connection-failed` | yes |
| `TimedOut` | `timed-out` | yes |
| `TlsFailed` | `tls-failed` | no |
| `TooManyRedirects` (a refused redirect) | `too-many-redirects` | no |
| `InsecureRedirect` (a refused redirect to `http:`) | `insecure-redirect` | no |
| `BlockedAddress` (the host resolves to a private address) | `blocked-address` | no |

The kind is only a name: never a host, an address or where a redirect
led. A guest gets it only if it exports `dp_features` with
`NETWORK_ERROR_KIND` (every module built with this SDK does); for other
guests the host leaves it out, so modules built before it read the reply
exactly as before. An older agent sends no kind: `network_kind()` is
`None` and `?` reports `request-failed` as before. `timed-out` is not a
shared code: declare it in `errorCodes` if your plugin can report it.

`op()`, `detail()` (the host's reason), `target()` (the request's origin,
never path or query) and `is_denied()` let a plugin react; `Display`
explains a denial and names the permission to declare. Map errors to your
own codes where it helps the user, as Developer Tools does for GitHub's
statuses.

Off wasm (unit tests, `cargo check`) every call is `Unavailable` unless it
goes through `testing::MockHost` ([building.md](building.md#testing)).

## HTTP

```rust
let response = ctx.fetch(
    Request::get(format!("{GITHUB_API}/repos/{repo}/actions/runs?branch={branch}&per_page=1"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "deckpilot-developer-tools/0.1.0"),
)?;
match response.status {
    200..=299 => {}
    404 => return Err(PluginError::new("repo-not-found", "GitHub does not know this repository")),
    s => return Err(PluginError::new("github-error", format!("GitHub answered HTTP {s}"))),
}
let runs: Runs = response.json().map_err(|_| PluginError::new("invalid-response", "unexpected answer"))?;
```

- `Context::fetch(Request) -> Response`, `Context::http_get(url)`.
  `Request::{new, get, post, put, patch, delete, head}`, `.header(name,
  value)`, `.body(text)`, `.json(&value)` (sets `Content-Type`),
  `.credential(&handle, scheme)`, `.bearer(&handle)`. `Response { status,
  headers, body }` with `is_success()`, `header(name)` (case-insensitive),
  `header_values(name)` (every value, in order), `link(rel)` (the target
  of the RFC 8288 `link` entry with that `rel`: `next`, `last`, ...),
  `text()`, `json::<T>()`.
- Permission: `network` for the exact host and the method
  ([permissions.md](permissions.md)).
- A non-2xx status is a `Response`, not an error; map it yourself.
- Limits: URL ≤ 8 KiB, ≤ 32 headers of ≤ 4 KiB, request body ≤ 32 KiB
  (text), response body ≤ 512 KiB, 15 s for all hops, at most 5
  redirects (each re-authorized; never `https` to `http`), 60 requests per
  minute. Bodies are text (UTF-8).
- Response headers are filtered to an allowlist of exact names
  (`RESPONSE_HEADERS` in `plugins/runtime/crates/abi/src/capability.rs`),
  by lowercase name:
  - content and caching: `content-type`, `content-length`,
    `content-language`, `cache-control`, `etag`, `last-modified`,
    `expires`, `age`, `vary`, `retry-after`, `date`;
  - pagination: `link`, and GitLab's `x-total`, `x-total-pages`, `x-page`,
    `x-per-page`, `x-next-page`, `x-prev-page`;
  - rate limits: GitHub's `x-ratelimit-limit`, `x-ratelimit-remaining`,
    `x-ratelimit-reset`, `x-ratelimit-used`, `x-ratelimit-resource`, and
    GitLab's and the IETF draft's `ratelimit`, `ratelimit-policy`,
    `ratelimit-limit`, `ratelimit-remaining`, `ratelimit-reset`,
    `ratelimit-observed`.

  Everything else is dropped: `set-cookie`, `www-authenticate`,
  `authorization`-style and vendor headers (`x-oauth-scopes`, ...),
  `location` (redirects are followed by the agent). A value longer than
  4 KiB or with control characters is dropped, and at most 32 headers are
  returned. When the request carried a credential, every form of the
  secret is replaced by `[redacted]` in the values first (a `link` that
  echoes `?api_key=…` arrives as `?api_key=[redacted]`).
- The agent's HTTP client sends no `User-Agent` of its own for plugins;
  APIs that require one (GitHub does) need `.header("User-Agent", …)`.
- The agent never uses a proxy for plugin requests and connects only to
  public addresses it resolved and checked.

## Open URL

```rust
let url = format!("https://github.com/{repo}/pull/{number}");
ctx.open_url(url)?;
Ok(Output::message("opened the pull request"))
```

- `Context::open_url(url)` (op `url.open`) asks the agent to open the page
  in the user's browser. The answer is only whether it was opened.
- Permission: `openUrl` for the URL's exact host
  ([permissions.md](permissions.md)): `openUrl:github.com` opens
  `https://github.com/...` on port 443, not `gist.github.com` or
  `github.com:8443`. For a server the user entered in an origin setting
  and allowed: `openUrl.userOrigins`
  ([permissions.md](permissions.md#pages-of-a-server-the-user-entered)).
- Only `https:` URLs without userinfo. Never `http:`, `file:`, `mailto:`,
  `javascript:`, `data:` or a custom scheme (`x-github-client:`,
  `vscode:`), which could start a program; never an IP address or a local
  name (`localhost`, `*.local`, ...), whatever the grant. The agent opens
  exactly the normalized URL through the operating system's handler,
  without a shell.
- Limits: 2,048 bytes; 1 per 2 s and 20 per 10 minutes per plugin
  (`Throttled`); 3 per invocation (`Throttled`).
- The agent's audit and log name the origin only, never the path or
  query, which may carry tokens.
- Only in a run the user started: a key press, a gesture, the app (the
  profile editor, the Action Library or the plugin's own page), or a
  workflow the user started that way. In a run that a schedule, a
  lifecycle event, the agent's start or a work session started, the
  broker denies the call with `NotPermitted` ("pages open only in an
  action the user started …") before it uses any rate limit, and the
  executor checks again before anything is opened. The run context is the
  agent's, never something the plugin sends. Use
  [`Context::run_context`](#run-context) to tell beforehand, for example
  to report a link as an output instead.

## Dialogs

`Context::dialog(Dialog::form(..))` (op `dialog.open`, plugin API 1.5)
shows a standard confirm, alert, form or selection dialog and returns the
user's checked answer. It needs the `dialogs` permission and a run the
user started; see [dialogs.md](dialogs.md).

## Language and translations

`Context::locale()` (op `locale.get`, plugin API 1.6) is the app's language
as a tag (`en`, `pl`, `pl-PL`); `Context::t(key, &[("count", 3)])` and
`Context::text(key)` (op `locale.translate`) resolve one of the plugin's
own translation keys in it, with the fallback chain, `{{name}}`
placeholders and plurals of [localization.md](localization.md). Both need no
permission, are allowed in every run and answer plain text; a plugin can
only reach its own keys.

## Run context

```rust
let run = ctx.run_context()?;
if run.is_user_initiated() {
    ctx.open_url(page)?;
}
if let Some(summary) = &run.run {
    // A node of a workflow graph: how its own run is going so far.
    let failed = summary.has_failed();
    let name = summary.workflow.as_deref().unwrap_or("workflow");
}
```

- `Context::run_context() -> RunContext` (op `run.context`, always
  allowed): what started the invocation's run, and for a workflow node how
  that run is going.
- `trigger` (`RunTrigger`): `key` (a key, dial or touch control), `gesture`
  (a double or long press), `app` (the editor or the Action Library),
  `test` (the plugin's own page in the app), `workflow`, or `other`
  (unknown). For `workflow`, `cause` (`RunCause`) says what started the
  run: `user` (a key, a gesture or the app), `schedule`, `event` (a
  lifecycle event such as a device connecting), `startup` (the agent
  started), `workSession`, or `other`.
- `RunContext::is_user_initiated()`: `key`, `gesture`, `app`, `test`, or
  `workflow` with cause `user`. Exactly the runs `url.open` is allowed in.
- `run` (`RunSummary`, only for a node of a workflow graph): the
  workflow's name (at most 128 characters), `elapsedMs`, how many nodes
  `succeeded`, `failed`, were `skipped` or `cancelled` so far, and the ids
  of the nodes that failed (`failedNodes`, at most 32). `has_failed()`.
  That is the invocation's own run only: no messages, outputs or
  parameters, nothing of another workflow or plugin, never a secret.
- Compatible within ABI 1: a module built before it never sends the op,
  and nothing it receives changes. The reply ignores fields it does not
  know, and a trigger or cause added later reads as `other`. An agent
  older than the op refuses it (`UnsupportedOp`); treat that as unknown.
- `testing::MockHost::allow_all()` answers it as a run from the app.

## Credentials

Secrets are **handles**: the plugin names a credential slot, the agent
injects the secret into one request, and the secret never enters the
module's memory, logs or results.

```rust
let token = SecretHandle::slot("githubToken")?;          // contributes.credentials[].key
let mut request = Request::get(url).header("User-Agent", USER_AGENT);
if ctx.has_credential(&token)? {
    request = request.bearer(&token);                      // injected by the agent
}
let response = ctx.fetch(request)?;
```

- Declare the slot in `contributes.credentials` (hosts, schemes,
  `headerName`/`queryName`, `setupUrl`) and in a `credentials` permission's
  `refs` ([manifest.md](manifest.md#contributescredentials)).
- The user stores the secret with `setPluginCredential` (1 byte to 16 KiB,
  no line break or NUL); `listPluginCredentials` shows whether it is set,
  never the value. It lives in the encrypted plugin database.
- `Request::credential(&handle, AuthScheme::{Bearer, Basic, Header,
  Query})` and `Request::bearer(&handle)`. The broker allows it only for a
  granted slot, to the slot's own hosts (each also a granted `network`
  host), in a declared scheme, and never across a redirect to another
  origin. The executor sends it as `Authorization: Bearer|Basic …`, the
  slot's `headerName`, or the `queryName` query parameter, over https
  only.
- No secret stored: the fetch fails with `Unavailable` ("no secret is
  stored"). `Context::has_credential(&handle) -> bool` (op
  `credentials.has`, needs the slot's grant) tells beforehand, so the
  plugin can go without or fail with `credential-missing`.
- Every form of the secret (as is, form-encoded, base64) is replaced by
  `[redacted]` in response headers, body and error messages before the
  module sees them. Never ask a user to paste a secret into a parameter or
  setting; there is no way to read a secret back.

## Settings

```rust
let settings = ctx.settings()?;
let repo = settings.text("defaultRepo");                   // Option<&str>
let branch = settings.text("defaultBranch").unwrap_or("main");
let notify = settings.boolean("notifyOnChange") == Some(true);
```

- `Context::settings() -> Settings` (op `settings.get`, always allowed).
  `Settings::{get::<T>, text, integer, boolean, contains, as_map,
  to_json}`.
- Declared in `contributes.settings.fields`; the user saves them with
  `setPluginSettings`, validated like action parameters; saved values that
  no longer fit (after an update) fall back to the default; a field with
  neither value nor default is absent, and a required one is reported to
  the app as `missing` (the action still runs).
- Read them in each invocation: ABI 1 has no `settingsChanged`
  notification ([lifecycle.md](lifecycle.md)).
- Stored in the plugin database, not exported with the configuration.

## Variables

```rust
ctx.publish_variable("openPrs", count)?;          // integer
ctx.publish_variable("buildStatus", &status)?;    // string
ctx.clear_variable("buildStatus")?;               // no value
ctx.clear_variables()?;                           // all of them
```

- `Context::publish_variable(id, value)`, `clear_variable(id)`,
  `clear_variables()` (ops `variables.publish`, `variables.clear`; always
  allowed, 10 per second, ≤ 16 KiB per value, id ≤ 100 characters).
- `id` is relative and must be declared in `contributes.variables`
  (otherwise `NotPermitted`); the value must have the declared `valueType`
  (`BadRequest`); only `scope: "machine"` can be published (`Unavailable`
  for `profile` and `project`). `null` clears.
- Workflows and keys read them as `{{ plugins.<publisherCamel>.<nameCamel>.<id> }}`,
  e.g. `plugins.acme.developerTools.buildStatus`; the agent announces each
  change (`sharedVariablesChanged`). No plugin code runs during a
  resolution.
- Values live in the agent's memory only: gone after an agent restart, and
  dropped when the plugin is disabled, updated, rolled back, uninstalled or
  quarantined. Keep what must survive in storage and publish it again.
- A plugin cannot read variables (its own or others'): ABI 1 has no
  `variables.get`.

## Storage

```rust
#[derive(Serialize, Deserialize)]
struct LastBuild { repo: String, branch: String, status: String, checked_at: String }

let previous: Option<LastBuild> = ctx.storage_get_as("last.buildStatus")?;
ctx.storage_set("last.buildStatus", &last)?;
let keys = ctx.storage_keys(Some("last."))?;
let usage = ctx.storage_usage()?;   // used_bytes, keys, quota_bytes, max_keys
```

- `Context::storage_get(key) -> Option<serde_json::Value>`,
  `storage_get_as::<T>(key) -> Option<T>`, `storage_set(key, value)`,
  `storage_delete(key)`, `storage_keys(prefix) -> Vec<String>` (sorted),
  `storage_usage() -> StorageUsage`. Always allowed for an enabled plugin;
  100 calls per second.
- The plugin's own key-value store in the agent's encrypted plugin
  database (`plugin_storage`), per plugin and per computer: no plugin can
  reach another's keys, it is never exported, and it is **deleted on
  uninstall** (never kept). It survives restarts, updates and disabling.
- Keys 1-128 characters without control characters; values JSON, at most
  64 KiB each. Quota: 256 KiB and 1,000 keys (`DEFAULT_STORAGE_QUOTA_BYTES`,
  `MAX_STORAGE_KEYS`), bytes counted as key plus JSON value in UTF-8; a
  `storage` permission raises the bytes to its `quotaKb` (at most 10 MiB).
  A write that would grow past the quota fails with `Failed` and a detail
  starting `storage-quota`; a write that does not grow it is allowed even
  above the quota (after `storage` was revoked).

## Notifications

```rust
if let Err(e) = ctx.notify(format!("Build {status}"), format!("{repo} {branch}: {was} -> {status}")) {
    ctx.warn(format!("notification not shown: {e}"));
}
```

- `Context::notify(title, body)` (op `notify`). Needs the `notifications`
  permission. Title ≤ 64, body ≤ 300 characters; 5 per minute. The agent
  shows it as its own notification, titled with the plugin's name.
- `Context::notify_with(NotifySeverity::{Info, Success, Warning, Error},
  title, body)` sets how it looks. The severity changes nothing else: the
  same permission, prefix and rate (and, without a routing, destination).
  An agent that predates severities refuses it as `BadRequest`; fall back
  to `notify`.
- **Where it goes is the agent's decision.** Without a routing, a
  notification goes to the status bar of an open DeckPilot window. With a
  `routing` in the manifest's `notifications` permission
  ([permissions.md](permissions.md#notification-routing)), the agent reads
  the plugin's settings it names, as the user saved them, each time: on
  the desktop (a notification of the operating system), in the status bar,
  and only from a least severity on. The plugin never reads or decides
  this; a user who turns them off in Settings › Plugins is obeyed.
- `Context::notify_delivered(severity, title, body)` answers where it was
  shown: `Ok(Some(NotifyDelivery { shown, desktop, status_bar, reason }))`.
  Shown nowhere (the app closed and desktop notifications off or refused,
  less severe than the user's setting) is still `Ok`, with `shown: false`
  and the `reason` in words for the user; not granted and the rate limit
  are errors as for `notify`. `Ok(None)` comes from an agent that predates
  deliveries. Guests built with this SDK report the ABI feature
  `NOTIFY_DELIVERY` (`dp_features`); every other guest gets `null`, as
  before.

## Log

```rust
ctx.debug(format!("{repo}: {count} open pull requests"));
ctx.log(LogLevel::Warn, "rate limit almost reached");
```

- `Context::log(level, message)`, `debug`, `info`, `warn`, `error` (op
  `log`, always allowed, 50 lines per second). Lines are cut to 2 KiB
  (`MAX_LOG_LEN`). Best effort: a dropped or throttled line is not an error
  and returns nothing. The agent writes them to its log, attributed to the
  plugin. Never log secrets or personal data.

## Time

```rust
let now = ctx.now()?;
let quiet = (22 * 60..24 * 60).contains(&now.local_minutes_of_day());
ctx.storage_set("snoozedUntil", &(now.unix_ms + 10 * 60_000))?;
```

- `Context::now()` (op `time.now`, plugin API 1.1) returns `Now
  { unix_ms, utc_offset_minutes }`: the agent's wall clock in milliseconds
  since the Unix epoch (UTC) and the local UTC offset at that instant in
  minutes, east positive. Helpers: `unix_seconds()`, `local_seconds()`,
  `local_minutes_of_day()` (0 to 1,439), `local_weekday()` (0 is Monday)
  and `millis_since(earlier)`.
- **No permission**: always allowed while the plugin is enabled. There is
  no time zone name and there are no rules for other instants; the offset
  of another moment (after a change to summer time) is what `now()`
  returns then.
- The clock is the system's: it can jump when the user or the network
  sets it. Compare stored instants with `millis_since`, which answers 0
  for a clock that went back, and never sleep or spin on it: a plugin has
  no timer, and an action that waits is stopped at its time limit.
- 20 calls per second (`throttled` beyond). `std::time::SystemTime::now()`
  still panics in the guest; use `ctx.now()`.
- An agent older than plugin API 1.1 answers `Unsupported`. A plugin that
  cannot work without the time declares `"pluginApi": "^1.1"` and
  `Plugin::requires_api(ApiLevel { major: 1, minor: 1 })`; older agents
  then do not install it.

## Randomness

```rust
let seed = ctx.random_u64()?;              // 8 bytes
let card = ctx.random_below(52)?;          // 0..52, unbiased
let nonce = ctx.random_bytes(16)?;         // 1 to 64 bytes
```

- `Context::random_bytes(n)` (op `random.bytes`, plugin API 1.1) returns
  `n` bytes, 1 to 64 (`MAX_RANDOM_BYTES`), from the operating system's
  cryptographic random source in the agent, fresh for every call.
  `random_u64()` takes 8 of them; `random_below(bound)` gives every value
  in `0..bound` the same chance (one call for 32 bytes).
- **No permission**: always allowed while the plugin is enabled.
- **4,096 bytes per minute** per plugin (a token bucket; `throttled`
  beyond). Take a seed once and expand it in the plugin (a shuffle of 52
  cards needs one `random_u64` and a small generator, not 52 calls). A
  length outside 1 to 64 is refused by the SDK without a host call.
- Random number crates that read the system (`getrandom`, `rand`'s
  default generators) still do not link or panic in the guest; seed a
  deterministic generator from `ctx.random_u64()`.

## Plugin API 1.9: resources, tools and the project context

Plugin API 1.9 adds the operations DeckPilot's own git, docker, workspace
and projects plugins need (`plugins/runtime/crates/abi/src/privileged.rs`;
reviewed in
[module-migration.md §7](../module-migration.md#plugin-api-19-security-reviews-2026-11-20)).
All but `profile.read` need permissions only **first-party plugins shipped
inside the app** can declare ([permissions](permissions.md#plugin-api-19-first-party-bundled-plugins-only));
for every other plugin they are denied. They need
`"engines": { "pluginApi": "^1.9" }` (`PRIVILEGED_API_LEVEL` in the SDK).

**Status: the ABI, the SDK and the broker's decisions are in place; the
agent carries out `resource` parameters, `resource.resolve` and
`tool.run`.** Every
other allowed call is answered `unsupported-op` ("… is not implemented by
this version of DeckPilot", `CapabilityErrorKind::Unsupported`) until its
host side lands (WP-A4 to WP-A6 of the extraction). A refused
`resource.resolve` starts its message with the reason code:
`no-active-project` (`unavailable`; also with the Projects feature off),
`resource-not-set` (`unavailable`), `resource-unbound`,
`resource-role-missing`, `not-found` (`failed`), `resource-kind-mismatch`
and `not-permitted` (`not-permitted`), `throttled`.

The agent also carries out the project operations and `profile.read`. They
read and switch only the active profile's projects, and only while a
project context provider is active (the Projects feature). A refusal
starts its message with the reason code: `projects-off`, `locked`,
`no-active-profile` and `unavailable` (`unavailable`); `not-found`,
`no-projects`, `at-last-project` and `at-first-project` (`failed`); and
`not-permitted` (`not-permitted`) for a switch in a run the user did not
start. `project.select` and `project.cycle` answer the new project. The
app's `projectChanged` event names the plugin. `profile.read` without an
active project has no `project`; that is not an error. In a workflow run a
switch is the run's, as a built-in project action's: only an action whose
node declares `changesContext` may switch, nothing changes until it
succeeds, the nodes after it run in the selected project, and the
workflow's project scope decides whether the active project follows at its
end; `project.current` and `profile.read` answer the run's project there.
`resource.resolve`'s `role` is the role name the user gave the project's
resource (not blank, no control character, at most 1,024 bytes).

The agent also carries out `tool.run`. It checks the array again before
anything runs: the denylists first (every option before `--`, as `--x`,
`--x=v`, `--x v`, `-xv` and each letter of `-xyz`), then the host's table
(`apps/agent/src/plugins/tools/{git,docker}.rs`), then the plugin's own
declaration; every value and positional fills a typed slot (a ref, a
`branch.<ref>.remote|merge` key, a remote name `git remote` lists, a
container, service or project name, `--tail` 1-1000, a `--since`), never
a path, a URL or anything starting with `-`. The working directory is a
handle: a `directory` for git, which must be the repository's own
folder, with its git directory inside it or a linked worktree's (git
never looks above it; a `.git` file or link to another repository, a
submodule and a bare repository are refused: `not-a-repository`,
`tool-outside-binding`); a `directory` or compose `file` for `docker
compose`; for docker's other commands also a `dockerEnvironment`, whose
context is used. For compose the host adds `--project-directory`, `-f`
(the bound file, or the folder's own `compose.yaml`, `compose.yml`,
`docker-compose.yaml` or `docker-compose.yml`) and `--project-name` (the
file's `name:`, else the folder's name); a plugin never passes them. A
`git log` format must start with `format:` or `tformat:`. Limits: 10 s for git's local reads, the user's
network time limit (default 20 s) for `ls-remote`, 30 s for docker's
reads, the user's change time limit (default 300 s) for docker's changes;
`timeoutMs` only shortens them, and the invocation's deadline always
applies. Output is capped at 128 KiB (64 KiB for logs; 16 KiB of standard
error), `truncated` says so, and URL userinfo is removed. A non-zero exit
is a result, not an error. A refusal starts with its reason code:
`tool-option-denied`, `tool-subcommand-denied`, `tool-not-allowed`,
`tool-not-declared`, `tool-invalid-cwd` and `tool-outside-binding`
(`not-permitted`),
`tool-invalid-argument` (`bad-request`), `not-user-initiated`
(`not-permitted`), `tool-unknown-remote`, `tool-executable-invalid`,
`not-a-repository`, `timed-out`, `cancelled` and `revoked` (`failed`),
`tool-not-found`
(`unavailable`).

| Operation | `Context` | Needs | Run origin | Rate |
| --- | --- | --- | --- | --- |
| `resource.resolve {param \| role, projectId?, kinds}` → `{handle, kind, label, displayPath?}` | `resource_param`, `resource_role`, `resolve_resource` | `resources`; `displayPath` only with `resources:paths` | any | 20 per invocation |
| `resource.open {handle, with?, position?, editor?}` | `open_resource` | `resources:open` | user-started | 1 per second, 5 per invocation |
| `tool.run {tool, args, cwd, timeoutMs?}` → `{exit, stdout, stderr, truncated, durationMs}` | `run_tool` | `tools:git` or `tools:docker`; `ls-remote` also `tools:git:network`; docker's `start`, `stop`, `restart`, `compose up`, `compose down`, `compose restart` also `tools:docker:modify` | any; with `modify` user-started or a work session the user started | 20 starts per second; with `modify` 6 per minute |
| `url.openParam {param, index?}` | `open_url_param` | `openUrl:from-params` | user-started | as `url.open`, 10 per invocation |
| `project.current` → project or `null`, `project.list` → `{projects}` | `current_project`, `projects` | `projectContext` | any | |
| `project.select {id}`, `project.cycle {direction, wrap?}` | `select_project`, `cycle_project` | `projectContext:select` | user-started (never a lifecycle event, so no loop on `ProjectChanged`) | 2 per second |
| `profile.read` → `{profile?, project?}` | `profile` | `profile` with the `read` scopes (every publisher) | any | |

- A resource **handle** is 128 random bits (32 lowercase hex characters),
  bound to the plugin and the invocation; the guest never receives a path
  (unless `resources:paths`). A `resource` parameter of an action reaches
  the guest as such a handle (a string parameter value) when it resolved to
  a binding of one of its declared kinds; otherwise it is absent and
  `resource.resolve {param}` says why. The label is the binding's key. A
  binding of kind `directory`, `application` or `dockerEnvironment` is that
  kind; one of kind `other` whose value is an absolute path is a `file`. A
  `directory` or `file` must exist. A role resolves in the run's project,
  or with `projectId` in a project the action's own project parameter
  names.
- `tool.run` takes an **argument array**, never a shell string, starting
  with the subcommand (an array that starts with an option is refused,
  `NoSubcommand`). The working directory is a handle. The executable, the
  environment and the fixed options (`-C <dir> --no-pager …`) are the
  host's; the agent checks every array against the host policy the
  manifest declared a subset of
  ([`tools`](permissions.md#tools)).
- `resource.open` opens the folder or file of a handle with the
  application of another handle of kind `application` (a binding the user
  chose) or with the system's default handler; `editor` picks the host's
  argument plan for `position`. The plugin never names a program, an
  argument or a path. A handle of kind `url` (a project's URL role,
  resolved with `resource.resolve {role, kinds: ["url"]}`) opens in the
  default browser only, without `with`, `position` or `editor`, under the
  URL rules of `url.openParam`; the guest sees its label, never the
  address. Refusals start with `not-user-initiated`, `not-permitted`,
  `resource-kind-mismatch`, `invalid-application`, `not-found`,
  `invalid-position`, `invalid-url`, `bad-request` or `throttled`.
- `url.openParam` opens the literal value the user stored in the
  invocation's own parameter (http or https, no userinfo, at most 2,048
  bytes, no template output): the plugin never passes a URL. The value must
  be printable ASCII (an internationalized host in its `xn--` form), with
  its host written exactly as parsed (no percent-encoded or numeric
  shorthand host, no backslash). Private and local hosts open. Only a
  value the user wrote as it is opens, in any run the user started (a
  workflow's included); a default, a template, a variable or a workflow
  node's mapped value is `url-param-not-literal`. Other refusals: `invalid-url`,
  `url-param-missing`, `url-param-index`, `not-user-initiated`,
  `throttled`.
- No operation is available in a [state call](actions.md#key-state).
- Every call, allowed or denied, is audited by operation, never by value.

## What there is not (ABI 1)

No file system, process, clipboard, keychain or shell access, ever (plugin
API 1.9's `tool.run` is not process access: DeckPilot's own bundled
plugins name `git` or `docker` and an argument array, and the agent's
policy builds and checks the command). No
ambient clock or random source: `std::time::SystemTime::now()` and random
number generators that read the system fail or panic in the guest; the
time and random bytes come from the agent ([Time](#time),
[Randomness](#randomness)). No sleep and no timers. No reading of the
active profile or project before plugin API 1.9's `profile.read` (the run
context names what started the run and how its workflow run is going,
nothing more), no reading of variables, no device
access, no timers or background work, no streaming HTTP, no WebSockets.
The JavaScript design (`plugins/sdk-js`) declares several of these; ABI 1
does not implement them.
