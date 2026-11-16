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
| `network` | `hosts` (1-16 exact DNS names, optional `:port`), `methods` (`GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE`; at least one), `privateNetwork` | `network:<host>` per host | `http.fetch` to `https://<host>` with a listed method. Exact origins only: **wildcard hosts (`*.example.com`) and `privateNetwork` validate but are never granted** by this version (they are not passed to the broker), so a request to them is denied. No IP literals, no `localhost`; every resolved address must be public |
| `credentials` | `refs` (1-8 slot keys of `contributes.credentials`) | `credentials:<key>` per slot | `http.fetch` with `auth` naming the slot, only to the slot's `hosts` (each also a granted `network` host) and `schemes`; `credentials.has` for the slot |
| `notifications` | | `notifications` | `notify` |
| `storage` | `quotaKb` (257-10,240) | `storage` | Raises the storage quota from 256 KiB to `quotaKb` KiB. Storage itself needs no permission |
| `profile` | `read` (`profile`, `project`) | `profile` | Nothing yet: no ABI 1 capability reads the profile or project |
| `variables` | `read` (variable ids of other owners, a final `.*` allowed) | `variables:<entry>` | Nothing yet: ABI 1 has no capability to read variables. Own variables need no permission |
| `openUrl` | `hosts` (1-16 exact DNS names, optional `:port`) | `openUrl:<host>` per host | `url.open` ([`Context::open_url`](capabilities.md#open-url)) for `https://<host>` URLs: no other scheme, no userinfo, at most 2,048 bytes, 1 per 2 s and 20 per 10 minutes, 3 per invocation, and only in a run the user started (a key, a gesture, the app, or a workflow the user started; never a schedule, a lifecycle event, the agent's start or a work session). Exact hosts only: **wildcard hosts validate but are never granted**. It gives no `network` access to the host, and `network` gives no `openUrl` |
| `background` | `minIntervalSeconds` (15-86,400) | `background` | Nothing yet: no background work or timers in ABI 1 |
| `device` | `state`, `icons` (booleans) | `device:state`, `device:icons` | Nothing yet |

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
- **Credential slots depend on their hosts' network grants.** The broker
  refuses a grant set in which a granted credential slot names a host
  without a network grant (`GrantError::CredentialOriginNotGranted`), and
  the agent then gives the plugin **no grants at all** ("the broker refused
  the grants; the plugin gets none" in the agent log): revoking
  `network:api.github.com` from Developer Tools denies its settings and
  storage too, until the network permission is granted again (tested in
  `apps/agent/tests/plugins_examples.rs`). This fails closed; it is
  coarser than a revoke of one permission suggests.
- **Updates** that add or broaden a permission (`PermissionChanges`:
  `added`, `broadened`) are staged and need consent for the escalated keys
  (`approvePluginUpdate`); declining keeps the old version.

## What the broker checks for `http.fetch`

`Session::authorize` (`crates/broker/src/lib.rs`) for every request:

- `https` only; the origin (scheme, host, port) must be a granted
  `network` origin and the method one of its methods;
- headers are tokens without CR, LF or NUL, and never `Host`,
  `Authorization`, `Proxy-Authorization`, `Cookie`, `Set-Cookie`,
  `Connection`, `Transfer-Encoding`, `Content-Length`, `Origin`, `Forwarded`,
  `X-Forwarded-*`, method overrides, or anything starting `proxy-` or
  `sec-` (`FORBIDDEN_HEADERS`): credentials go through slots only;
- sizes within ABI 1 limits;
- every redirect is re-authorized as a new request (`authorize_redirect`,
  at most 5), and a credential never follows a redirect to another origin;
- the agent's executor connects only to public addresses it checked
  (`check_connect_addr`).

A denial reaches the plugin as `CapabilityErrorKind::NotPermitted`
(`?` turns it into the shared code `not-permitted`) with a message that
names the operation, the origin and the permission to declare.
