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
| `profile` | `read` (`profile`, `project`) | `profile` | Plugin API 1.9: `profile.read` ([`Context::profile`](capabilities.md#plugin-api-19-resources-tools-and-the-project-context)), the profile's name with `profile` and the active project's id, key and name with `project`. Medium tier, every publisher. The broker decides it; this agent answers it as not implemented yet |
| `variables` | `read` (variable ids of other owners, a final `.*` allowed) | `variables:<entry>` | Nothing yet: ABI 1 has no capability to read variables. Own variables need no permission |
| `openUrl` | `hosts` (1-16 exact DNS names, optional `:port`; may be empty with `userOrigins`), `userOrigins` (settings of the `network` permission's origin slots, [below](#pages-of-a-server-the-user-entered)) | `openUrl:<host>` per host; `openUrl:user-origin:<setting>` per origin setting | `url.open` ([`Context::open_url`](capabilities.md#open-url)) for `https://<host>` URLs: no other scheme, no userinfo, at most 2,048 bytes, 1 per 2 s and 20 per 10 minutes, 3 per invocation, and only in a run the user started (a key, a gesture, the app, or a workflow the user started; never a schedule, a lifecycle event, the agent's start or a work session). Exact hosts only: **wildcard hosts validate but are never granted**. It gives no `network` access to the host, and `network` gives no `openUrl` |
| `background` | `minIntervalSeconds` (15-86,400) | `background` | Nothing yet: no background work or timers in ABI 1 |
| `dialogs` | | `dialogs` | Plugin API 1.5 (needs `"engines": { "pluginApi": "^1.5" }`), medium tier: `dialog.open` ([`Context::dialog`](dialogs.md)), standard confirm, alert, form and selection dialogs that DeckPilot draws, titled with the plugin's name, at most one open and 6 a minute, only in a run the user started. Declarative data only: no plugin UI code, no markup, no secret fields. `ui` and `customUi` stay never grantable |
| `device` | `state`, `icons` (booleans) | `device:state`, `device:icons` | `device:state` (low tier): the agent asks the plugin what the keys bound to its own actions show ([key state](actions.md#key-state), plugin API 1.2): a typed description the agent draws, never pixels, and nothing of a device reaches the plugin. Without it the keys are static. `device:icons`: nothing yet |

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
`projectContext` left that list with plugin API 1.9, behind the
first-party gate ([below](#plugin-api-19-first-party-bundled-plugins-only)).
Declaring a kind that does nothing yet is allowed, but it still costs the
user a consent decision: declare only what you call.

## Plugin API 1.9: first-party bundled plugins only

The kinds below are reviewed in
[module-migration.md §7](../module-migration.md#plugin-api-19-security-reviews-2026-11-20)
(X1 to X13). They need `"engines": { "pluginApi": "^1.9" }` (a `^1.8`
manifest that declares one is refused with the level to set), and they
pass **only** the rules for a plugin shipped inside the app, whose every
file matched the pins compiled into the agent
(`Manifest::validate_bundled`, used by `verify_bundled_files`). A
third-party manifest (`Manifest::validate`), a `deckpilot.*` package from a
registry (`Manifest::validate_first_party`), a sideloaded and a developer
package are refused (`first-party-only`), and the plugin store refuses to
install such a manifest from any source but the bundle. Process execution
in general, a shell, the file system, the workflows engine and plugin UI
stay never grantable for every publisher.

**Status:** the manifest, the grants, the ABI, the SDK and the broker are
in place; the agent carries out `resource` parameters,
`resource.resolve` and `tool.run`, and answers the other operations as not implemented
until their host side lands (WP-A4 to WP-A6).

| Kind | Fields | Grant keys and tiers | Allows |
| --- | --- | --- | --- |
| `tools` | `tools`: 1-2 of `{executable, subcommands, network?, modify?}` ([below](#tools)) | `tools:git`, `tools:docker` (**high**); `tools:git:network` (**high**); `tools:docker:modify` (**high**, off by default) | `tool.run` of the declared subcommands and options |
| `resources` | `paths`, `open` (booleans) | `resources` (medium); `resources:paths` (medium); `resources:open` (**high**) | `resource.resolve` (labels only; with `paths` the bound path); `resource.open` in runs the user started |
| `openUrl` with `fromParams: true` | `hosts` may then be empty | `openUrl:from-params` (medium) | `url.openParam`: the literal http or https URL the user stored in the invocation's own parameter, in runs the user started |
| `projectContext` | `select` (boolean) | `projectContext` (medium); `projectContext:select` (**high**) | `project.current`, `project.list`; with `select`, `project.select` and `project.cycle` in runs the user started |
| `hostViews` | | `hostViews` (low) | The view kinds `workflows` and `projects`, `contributes.statusItems: ["activeProject"]` and `contributes.dashboardCards: ["activeProject"]`, all drawn by the app from core data: placement only, no data |

A `resource` parameter (`{"type": "resource", "kinds": ["directory",
"file", "application", "dockerEnvironment"]}`) is gated the same way; it
has no default and is not a setting. High tiers are always asked for,
never derived by a module migration (which derives at most medium) and
never seeded with a bundled plugin (which seeds low tiers only).

### `tools`

```json
{ "kind": "tools",
  "tools": [
    { "executable": "git", "network": true, "subcommands": [
        { "name": "status", "options": ["--porcelain=v2", "--branch", "-z"] },
        { "name": "log", "options": ["-1", "--format"] },
        { "name": "ls-remote" } ] }
  ],
  "justification": "Reads the status of the repositories you bind to keys." }
```

- `executable` is `git` or `docker`, nothing else (no path: the path is the
  user's own host-held setting). There is no argv, shell string or
  environment in the manifest.
- Each subcommand and option must be one of the host's policy
  (`deckpilot_plugin_manifest::tools`): git `status`, `log`, `config`,
  `remote`, `rev-parse`, `ls-remote`; docker `context show`, `version`,
  `compose version`, `ps`, `compose ps`, `compose logs`, `logs`,
  `compose up`, `compose down`, `compose restart`, `start`, `stop`,
  `restart`, each with its own options (what the built-in modules pass:
  for example `status --untracked-files`, `log --no-show-signature`,
  `compose ps --all`, `compose logs --no-color`). That allowlist is the
  boundary; the agent checks every run against it and the declaration
  again. The compose file, folder and project name are the host's, from
  the binding: a plugin never declares or passes `-f`, `-p` or
  `--project-name`.
- The denylists are checked first for a precise error: git's `-c`,
  `--config-env`, `-C`, `--git-dir`, `--work-tree`, `--namespace`,
  `--exec-path`, `--upload-pack`/`-u`, `--receive-pack`, `--output`/`-o`,
  `--ext-diff`, `--textconv`, `--exec`, `--super-prefix` and any `core.*`
  or `alias.*` key; docker's `run`, `exec`, `create`, `cp`, `attach`,
  `build`, `push`, `pull`, `login`, `logout`, `inspect`, `kill`, `system`,
  `volume`, `network`, `plugin`, `swarm`, `secret`, `image`,
  `compose config|run|exec|cp|build|push|pull|kill`, and the options `-v`,
  `--volume(s)`, `--mount`, `--privileged`, `--cap-add`, `--device`,
  `--pid`, `--network`, `--security-opt`, `-H`/`--host`, `--context`,
  `--config`, `--tls*`, `-e`/`--env`, `--env-file`,
  `--project-directory`, `-p`/`--project-name`, `-f`/`--file`, `--rmi`, `--build`,
  `--force-recreate`, `--renew-anon-volumes`.
- `network` (git only) is declared exactly when `ls-remote` is; `modify`
  (docker only) exactly when a subcommand that starts, stops or restarts
  is.
- A plugin with `tools` declares no `network`, `credentials` or `openUrl`
  (no exfiltration channel next to process authority).

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

### Pages of a server the user entered

`openUrl.hosts` are fixed names, so a plugin for a self-managed GitLab or a
Jira site could not open a merge request or an issue: the web host is the
server only the user knows. The `openUrl` permission may name the origin
settings whose server's pages it opens:

```json
{ "kind": "openUrl", "hosts": ["gitlab.com"], "userOrigins": ["apiUrl"],
  "justification": "Opens a project or a merge request on GitLab in your browser." }
```

- Each entry must be the `setting` of a `network.userOrigins` slot
  (`unknown-reference` otherwise); at most 4; `hosts` may then be empty.
- It is a declared permission of its own, `openUrl:user-origin:<setting>`
  (medium tier), shown and granted like every other. An update that adds
  it is an escalation.
- By itself it opens nothing. Pages open only on the server the user
  entered in that setting **and allowed** (the consent above), while this
  permission is granted: `https` pages of exactly that scheme, host and
  port. The consent dialog says so ("It may also open pages of
  `https://code.example.com` in your browser").
- It is by server, not by the slot's path prefix, like every `openUrl`
  grant is by host: a server's pages are usually not under its API's path
  (`https://code.example.com/gitlab/api/v4` and
  `https://code.example.com/gitlab/group/project`). It allows no request:
  `http.fetch` stays within the methods and path prefix the user allowed.
- A server on a private network (an IP literal or a local name) is opened
  only when it is exactly the one the user allowed on a private network;
  every other private or local host, `localhost` and link-local addresses
  stay refused by the URL check.
- Everything else of [`url.open`](capabilities.md#open-url) applies: only
  in a run the user started, 1 per 2 s, 20 per 10 minutes, 3 per
  invocation, audited by origin.
- Revoking the permission, revoking the consent to the server, changing
  the setting or uninstalling ends it on the next call. Revoking this
  permission does not end the consent to the server (requests still go
  there); revoking the slot's own permission ends both.
- An agent older than this refuses the manifest (unknown field).

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
