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
  `text()`, `json::<T>()`.
- Permission: `network` for the exact host and the method
  ([permissions.md](permissions.md)).
- A non-2xx status is a `Response`, not an error; map it yourself.
- Limits: URL ≤ 8 KiB, ≤ 32 headers of ≤ 4 KiB, request body ≤ 32 KiB
  (text), response body ≤ 512 KiB, 15 s for all hops, at most 5
  redirects (each re-authorized; never `https` to `http`), 60 requests per
  minute. Bodies are text (UTF-8).
- Response headers are filtered to `content-type`, `content-length`,
  `cache-control`, `etag`, `last-modified`, `retry-after` and `date`
  (`RESPONSE_HEADERS` in `apps/agent/src/plugins/executor.rs`); anything
  else, `Link` and rate-limit headers included, is dropped. Prefer APIs
  that put what you need in the body (Developer Tools uses GitHub's search
  `total_count` rather than paging through `Link`).
- The agent's HTTP client sends no `User-Agent` of its own for plugins;
  APIs that require one (GitHub does) need `.header("User-Agent", …)`.
- The agent never uses a proxy for plugin requests and connects only to
  public addresses it resolved and checked.

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

## What there is not (ABI 1)

No file system, process, clipboard, keychain or shell access, ever. No
clock and no random source: `std::time::SystemTime::now()` and random
number generators fail or panic in the guest, so there is no "now", no
random UUID (v4) and no ULID (Developer Tools uses the `Date` header of a
response as the time of a check, and name-based UUIDs). No reading of the
active profile or project, of variables, no opening URLs, no device
access, no timers or background work, no streaming HTTP, no WebSockets.
The JavaScript design (`plugins/sdk-js`) declares several of these; ABI 1
does not implement them.
