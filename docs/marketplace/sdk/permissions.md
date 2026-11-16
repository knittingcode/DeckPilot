# Permissions

Deny by default: `manifest.json` `permissions` is the complete list of
what the plugin may ever ask for, and the user approves each entry
(consent) when installing or enabling. Every capability call is checked
by the permission broker (`plugins/runtime/crates/broker`) against the
**current** grants, so a revocation applies to the next call, even in
the middle of a running action.

## Declaring a permission

```json
"permissions": [
  {
    "kind": "network",
    "hosts": ["api.github.com"],
    "methods": ["GET"],
    "justification": "Reads the open pull requests and the latest Actions run of the repository you name from the GitHub API."
  },
  {
    "kind": "credentials",
    "refs": ["githubToken"],
    "justification": "Sends your optional GitHub token to api.github.com for private repositories and higher rate limits."
  },
  {
    "kind": "notifications",
    "justification": "Tells you when the build status of a repository changes, only if you turn it on in the settings."
  }
]
```

Every permission needs a **`justification`** (10-200 characters): the
consent dialog and the plugin's details show it next to the permission.
Say what the plugin does with it, for the user, in one sentence. At most
16 permissions, and at most one of each kind (list several hosts in the one `network` permission).

## Kinds

Permission keys (`PluginPermission.key` in IPC, `GrantKey`) are what the
user grants and revokes.

| Kind | Fields | Grant key | What it allows in ABI 1 |
| --- | --- | --- | --- |
| `network` | `hosts` (1-16 exact DNS names, optional `:port`; may be empty with `userOrigins`), `methods` (`GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE`; at least one), `privateNetwork`, `userOrigins` ([below](#user-entered-origins)) | `network:<host>` per host; `network:user-origin:<setting>` per origin setting | `http.fetch` to `https://<host>` with a listed method. Exact origins only: **wildcard hosts (`*.example.com`) and `privateNetwork` validate but are never granted** by this version (they are not passed to the broker), so a request to them is denied. No IP literals, no `localhost`; every resolved address must be public. A server the user enters in an origin setting is reached only once the user consented to exactly it ([below](#user-entered-origins)) |
| `credentials` | `refs` (1-8 slot keys of `contributes.credentials`) | `credentials:<key>` per slot | `http.fetch` with `auth` naming the slot, only to the slot's `hosts` (each also a granted `network` host), or to the server of its `userOrigin` setting once the user allowed it, and in its `schemes`; `credentials.has` for the slot |
| `notifications` | | `notifications` | `notify` |
| `storage` | `quotaKb` (257-10,240) | `storage` | Raises the storage quota from 256 KiB to `quotaKb` KiB. Storage itself needs no permission |
| `profile` | `read` (`profile`, `project`) | `profile` | Nothing yet: no ABI 1 capability reads the profile or project |
| `variables` | `read` (variable ids of other owners, a final `.*` allowed) | `variables:<entry>` | Nothing yet: ABI 1 has no capability to read variables. Own variables need no permission |
| `openUrl` | `hosts` (1-16 exact DNS names, optional `:port`) | `openUrl:<host>` per host | `url.open` ([`Context::open_url`](capabilities.md#open-url)) for `https://<host>` URLs: no other scheme, no userinfo, at most 2,048 bytes, 1 per 2 s and 20 per 10 minutes, 3 per invocation, and only in a run the user started (a key, a gesture, the app, or a workflow the user started; never a schedule, a lifecycle event, the agent's start or a work session). Exact hosts only: **wildcard hosts validate but are never granted**. It gives no `network` access to the host, and `network` gives no `openUrl` |
| `background` | `minIntervalSeconds` (15-86,400) | `background` | Nothing yet: no background work or timers in ABI 1 |
| `device` | `state`, `icons` (booleans) | `device:state`, `device:icons` | Nothing yet |

### Notification routing

The `notifications` permission may name the plugin's own settings that say
where its notifications go (G9). The agent reads them, as the user saved
them, every time the plugin shows one; the plugin never does:

```json
{
  "kind": "notifications",
  "routing": { "desktop": "desktop", "statusBar": "statusBar", "minimumSeverity": "minimumSeverity" },
  "justification": "Shows the notifications your workflows ask for, where you choose in the settings."
}
```

- `desktop`: a boolean setting; on, notifications are also shown by the
  operating system.
- `statusBar`: a boolean setting; on, they are listed in the status bar of
  an open DeckPilot window.
- `minimumSeverity`: a choice setting of `info`, `success`, `warning` and
  `error`; less severe notifications are not shown.

At least one member; each must name a setting of `contributes.settings` of
that kind (`invalid-value` or `unknown-reference` otherwise). A member left
out keeps what a plugin without routing gets: no desktop notifications,
the status bar, every severity. The grant, its low tier, the prefix and
the rate are the same with or without it; the consent dialog says that
the plugin's settings decide where they appear. An agent older than
routing refuses the manifest (unknown field).

Kinds that are **never grantable** to plugins have no representation at
all: file system, processes, shell, clipboard, USB/HID, firmware, workflows, UI (the manifest
crate refuses them with a clear message, `FORBIDDEN_PERMISSION_KINDS`).
Declaring a kind that does nothing yet is allowed, but it still costs the
user a consent decision: declare only what you call.

## Always allowed (no permission)

For an **enabled** plugin whose grants the broker accepted:

| Capability | Bound |
| --- | --- |
| `log` | 50 lines per second |
| `storage.*` (own storage) | 256 KiB and 1,000 keys by default; 100 calls per second |
| `settings.get` (own settings) | |
| `run.context` (what started the invocation's own run) | |
| `variables.publish`, `variables.clear` (own declared variables) | 10 per second, 16 KiB per value |

Every request counts towards 200 per second overall, `http.fetch` towards
60 per minute and `notify` towards 5 per minute (`RateLimits::default`).
A disabled or uninstalled plugin has no grants: everything, including
these, is denied.

## Consent, enabling and revocation

- **Install** grants nothing; the plugin is installed disabled.
- **Enable** (`enablePlugin` with `consent: [keys]`) grants the listed keys
  first; **every declared permission must be granted** to enable. Hello
  World declares none, so it is enabled with an empty consent.
- While a declared permission is not granted (revoked after enabling), the
  plugin's actions are listed as unavailable with the reason; an
  invocation through `invokePluginAction` still runs and is denied by the
  broker at the call.
- **Revoke** (`revokePluginPermission`) applies to the next capability
  call. **Grant** (`grantPluginPermissions`) likewise.
- **Credential slots depend on their hosts' network grants.** A secret
  only ever travels to a host with a current network grant. When the user
  revokes such a host, the broker withholds it from every credential slot
  that names it (`Grants::suspended_credentials`; "credential slot
  withheld from hosts without a network grant" in the agent log); a slot
  left without hosts is not granted at all, so `credentials.has` and every
  request using it fail with `not-permitted`. Nothing else changes: the
  settings, the storage, the other hosts and the notifications keep
  working, and the slot is back with the network grant (tested in
  `plugins/runtime/crates/broker/tests/broker.rs` and, for Developer
  Tools, in `apps/agent/tests/plugins_examples.rs`).
- **Updates** that add or broaden a permission (`PermissionChanges`:
  `added`, `broadened`) are staged and need consent for the escalated keys
  (`approvePluginUpdate`); declining keeps the old version.

## User-entered origins

A GitHub Enterprise server, a self-managed GitLab or a Jira site is a host
only the user knows (G2, [module-migration.md §7](../module-migration.md#g2-user-entered-origins-github-ghe-gitlab-self-managed-jira-http)).
The `network` permission declares where the user enters it:

```json
{ "kind": "network", "hosts": ["api.github.com"], "methods": ["GET", "POST"],
  "userOrigins": [{ "setting": "apiUrl", "pathPrefix": true, "privateNetwork": true }],
  "justification": "Reads pull requests from GitHub or your GitHub Enterprise server." }
```

| Slot field | Rule |
| --- | --- |
| `setting` | A one-line `text` setting of `contributes.settings` (`maxLength` at least 12 if set). A default must be an exact host of `hosts` (`https://api.github.com`), so it needs no consent of its own |
| `methods` | Optional subset of the permission's `methods` |
| `pathPrefix` | The user may enter a path (`https://ghe.example.com/api/v3`); requests are then limited to that path and below (`/api/v3`, `/api/v3/…`, never `/api/v30` or `/admin`) |
| `privateNetwork` | The user may enter a server on a private network (a LAN address, a local name such as `ghe.internal`, or a name that resolves to a private address). Each such server needs its own explicit consent. `localhost`, loopback and link-local addresses (cloud metadata) are never reachable |

At most 4 slots. A credential slot is bound to one with `"userOrigin":
"apiUrl"` (its `hosts` may then be empty): its secret goes to the server
the user allowed, within its path prefix, and nowhere else.

How it is granted:

1. **Install** shows the slot as its own permission,
   `network:user-origin:apiUrl` (medium tier; high with `privateNetwork`):
   the plugin may ask to reach a server the user enters. An update that
   adds a slot, more methods, a path prefix or a private network, or binds
   a credential to it, is an escalation.
2. **Saving the setting** validates and normalizes it: `https` only, no
   user name or password, no query or fragment, no wildcard, no `.`/`..`
   segments or encoded slashes, a private host only if the slot allows it
   (`invalidData` otherwise). The agent stores what the broker will parse
   (`https://ghe.example.com/api/v3`), and that is what the plugin reads.
3. **The user allows exactly that server** (`grantPluginUserOrigin`, the
   Configure dialog), naming the normalized origin the setting holds; a
   private network is a separate choice in the same consent, required for a
   private host and never assumed. The consent is recorded per plugin and
   setting with its origin, path prefix, methods, private network and
   credential slots (plugins.db `plugin_user_origins`).
4. **Changing the setting** ends the consent at once (the agent deletes it
   and tells the broker), and so does `revokePluginUserOrigin`, revoking the
   slot permission, or an uninstall. Changing the setting back asks again.

The broker matches a request against the allowed server exactly (scheme,
host, port, and the path prefix), re-checks every redirect the same way (a
redirect out of the prefix or off the origin is refused; a credential never
follows one off the origin), and the executor connects only to addresses it
checked: public ones, plus the private ranges (RFC 1918, carrier NAT,
unique local IPv6) for a server the user allowed on a private network.
Every address a name resolves to must pass, and the connection uses
exactly those, so DNS rebinding cannot swap one in. A server the plugin
declares in `hosts` (the default) needs no consent of its own; there is no
wildcard or unconsented user origin.

## What the broker checks for `http.fetch`

`Session::authorize` (`crates/broker/src/lib.rs`) for every request:

- `https` only; the origin (scheme, host, port) must be a granted
  `network` origin, or a server the user entered and allowed whose path
  prefix contains the URL ([user-entered origins](#user-entered-origins)),
  and the method one of its methods;
- headers are tokens without CR, LF or NUL, and never `Host`,
  `Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie`,
  `Connection`, `Transfer-Encoding`, `Content-Length`, `Origin`, `Forwarded`,
  `X-Forwarded-*`, method overrides, or anything starting `proxy-` or
  `sec-` (`FORBIDDEN_HEADERS`): credentials go through slots only;
- sizes within ABI 1 limits;
- every redirect is re-authorized as a new request (`authorize_redirect`,
  at most 5), and a credential never follows a redirect to another origin;
- the agent's executor connects only to public addresses it checked
  (`check_connect_addr`), or also to private ranges for a server the user
  allowed on a private network (`check_private_network_addr`).

A denial reaches the plugin as `CapabilityErrorKind::NotPermitted`
(`?` turns it into the shared code `not-permitted`) with a message that
names the operation, the origin and the permission to declare.
