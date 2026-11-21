# deckpilot.jira

The first-party Jira plugin: the replacement of the former built-in `jira`
module (removed in WP-C5), part of P11
(`docs/marketplace/module-migration.md` §9), and now its only
implementation.
Bundled with class `migration` (`../bundled.json`) and pinned in builds that
enable bundled plugins.

**Routed since the cut-over (P11)**, in builds with plugins only:
`ACTION_ALIASES` (`crates/deckpilot-core/src/alias.rs`) maps every
`jira.<action>` to `deckpilot.jira.<action>`, and the agent's migration
plan for `jira` (`apps/agent/src/plugins/migration.rs`) installs this
plugin, copies the module's settings with the server you had entered and,
once you approve, imports the credential and allows exactly that server.
From then on the stored `jira.*` keys, workflow nodes and actions run
here; nothing stored is rewritten. Until you approve, after a decline,
for every computer that is not eligible (a server a plugin cannot reach,
a per-action credential other than the module's) and in a build or start
without plugins, they are unavailable with the reason: there is no
built-in to fall back to since WP-C5. An older app on the same data still
runs its built-in.
`apps/agent/tests/plugins_cutover_servers.rs` runs that upgrade end to
end.

Read-only. It talks only to the Jira site **you enter** in its `Site`
setting and allow (GET), with your credential from the slot `credential`
(Basic for Jira Cloud, Bearer for Data Center), and opens pages on that
same site only. It declares no host of its own: Jira has none.

## The site you enter

`Site` is a user-entered origin
([permissions.md](../../../docs/marketplace/sdk/permissions.md#user-entered-origins)):

- It is saved normalized (`https://your-team.atlassian.net`, or a Data
  Center address with its path, `https://jira.example.com/jira`), and
  nothing is sent to it, no credential and no request, until you allow
  exactly that server in Settings › Plugins › Jira › Configure. Requests
  are then limited to the path you entered and below.
- A Data Center on a private network (a LAN address, a name such as
  `jira.internal`, or a name that resolves to a private address) needs the
  separate private-network consent for that server. This computer
  (`localhost`, loopback) and link-local addresses are never reachable.
- Changing the setting ends the consent at once; the new site asks again.
- Until the site is allowed every reading fails with `not-permitted` and
  says where to allow it.
- `Open in Jira` opens `https` pages under the same site only, and only
  once you allowed it.

## The contract

Every action of the built-in, under the same name, so the alias
`jira.<action>` → `deckpilot.jira.<action>` is valid. Two tests keep
it that way:

- `crates/deckpilot-modules/tests/contracts.rs` (`first_party`): the
  manifest, converted as the agent registers plugin actions, against the
  built-in's frozen 0.9.0 definitions
  (`crates/deckpilot-modules/legacy/jira.json`):
  `alias::incompatibilities` (§5.3) for every alias, and beyond it the
  definitions must be identical but for the documented differences
  below. Also the variables (none on either side),
  the settings and that the permissions are exactly the three below.
- `apps/agent/tests/plugins_jira.rs`: every action of the plugin through
  the real agent and sandboxed host, on canned Jira answers. Until WP-C5
  it compared the plugin with the built-in on the same answers served
  from 127.0.0.1; now it is a plugin-only regression test holding the
  plugin to those answers: the same status, message, outputs and error
  code, and **the same requests in the same
  order** (the plugin's canned answers are exactly what the built-in asked
  for): 19 readings (success and every error code) and 5 pages to open on
  Jira Cloud; 8 readings and 3 pages again on a Data Center under a path;
  refused before the consent, after the site changed and after the
  consent's revocation; and two network failures (`dns-failed`,
  `tls-failed`).

| Built-in | Plugin | Definition | Results |
| --- | --- | --- | --- |
| `jira.myself` | `deckpilot.jira.myself` | compatible: no `credential` parameter (below) | identical |
| `jira.issue` | `deckpilot.jira.issue` | compatible: no `credential` parameter | identical |
| `jira.issues` | `deckpilot.jira.issues` | compatible: no `credential`; `source`, `includeDone` and `limit` optional with the same defaults (a manifest has defaults only on optional parameters; the agent fills them in either way) | identical, with the same requests: the fixed searches, the list in one page, Cloud counted in up to 5 pages of 100 keys |
| `jira.open` | `deckpilot.jira.open` | compatible: `cancellable` (the agent can stop every plugin action; the built-in's cannot be interrupted) | identical; opened through `url.open`, the same page |
| variables | | identical: none | |

Parameters, labels, descriptions, kinds, defaults, output keys, labels,
kinds, `always` and `sensitive` flags, icons, effects, retry and
idempotency flags and timeouts are identical.

### Permissions

| Permission | What for |
| --- | --- |
| `network`: no host; GET; the origin slot `siteUrl` (a path, a private network) | reading from the site you enter and allow |
| `credentials`: `credential` | the credential, sent to the site you allowed and nowhere else |
| `openUrl`: no host; the server of `siteUrl` | opening Jira pages in the browser |

### Settings and credentials

| Built-in setting | Plugin | Rule |
| --- | --- | --- |
| `siteUrl` | `siteUrl` (same kind, no default, portability) | an `https` address; the server is reached only with your consent to it (above). A plain `http` address on this computer, which the built-in accepts, is not reachable for a plugin and fails with `invalid-site-url` |
| `deployment` | `deployment` (same choices, default, portability) | `cloud`: REST API 3, the credential as Basic; `data-center`: REST API 2, as Bearer |
| `timeoutSeconds` | `timeoutSeconds` (same) | the time limit of one action, counted with Jira's `Date` headers (a guest has no clock) |
| `credential` (a credential id, default `jira.token`) | the credential slot `credential` (Basic or Bearer; the allowed site only) | the per-action `credential` override is dropped by the alias when it is empty or the module's own (`ParamRule::DropIfDefault`); any other value is not eligible (the keys stay unavailable) until G5 |
| `allowPrivateHost` | not declared | replaced by the private-network consent per server, which is never assumed |

## What the plugin does not reproduce

All on failure paths, limits or the editor:

1. **Checks when an action is saved.** The built-in's `check` reports an
   invalid issue key, project key, credential name or page, and a filter
   without its number, as soon as an instance is saved. The agent and the
   app check every plugin action on save against its manifest's schema
   (kinds, required parameters, choices, ranges, lengths); the
   plugin-specific checks cannot be derived from a schema and plugin API 1
   has no validation hook. They stay **run-time only**: the same errors,
   with the same codes and messages, come when the action runs.
2. **The credential's form is not checked.** The built-in refuses a Jira
   Cloud credential that is not `email:api-token` (`credential-missing`)
   before it sends anything, and trims a Data Center token. A plugin never
   sees its secret, so neither is possible: a Cloud credential without the
   colon is sent as it is, and Jira answers 401 (`unauthorized`).
3. **Failure messages.** `unauthorized` (401) and `credential-missing`
   name the plugin's slot and Settings › Plugins › Jira instead of the
   credential id `jira.token` and Settings › HTTP › Credentials;
   `invalid-site-url` names Settings › Plugins › Jira and says what a site
   must be instead of naming the parser's reason; a network failure's
   message is the agent's explanation rather than the built-in's (also for
   `blocked-address`, where the built-in points to its own setting). Codes
   are the same. A site that is entered but not allowed, or whose consent
   was revoked, fails with `not-permitted`, which the built-in has no
   equivalent of. When the browser cannot be started for `open`, the code
   differs too: the launcher's in the built-in, the agent's
   `request-failed` here.
4. **`rate-limited` carries no data.** The built-in's failure has
   `retryAfterSeconds`; a plugin failure has a code, a message and
   `retryable` only. The wait is in the message ("try again in 30 s") and
   the failure is retryable. Nothing reads the field today.
5. **Time.** The time limit is counted from Jira's first answer with its
   `Date` headers (one-second steps), and each request has the agent's
   15 s; a `Retry-After` given as a date is counted from the answer's
   `Date`.
6. **Agent limits for plugins.** 60 requests per minute (a Cloud list of
   more than 500 issues needs 6), one opened page per 2 s and 20 per 10
   minutes; an output text holds at most 16 KiB, so a list of issues longer
   than that is cut at a line (`listed` and `truncated` say so). An answer
   over 512 KiB (the agent's limit for plugins; the built-in reads up to
   4 MiB) is `invalid-response`: a list of 100 issues with the three
   fields read is about 200 KiB.
7. **Pages given with a non-ASCII host name.** `open` compares a page's
   host with the saved site as text (the agent stores it in its ASCII
   form); a page URL written with the Unicode form of an internationalized
   host name is refused with `url-not-allowed` where the built-in would
   accept it.

## Build

```sh
plugins/first-party/build.sh --out target/bundled-plugins   # with its pins
cd plugins/runtime && cargo test -p deckpilot-jira           # unit tests
```

No `url` crate in the module: the few URLs it needs are built and checked
by `../support` (`deckpilot-first-party-support`), whose tests compare it
with the `url` crate.

## Versions

A changed plugin gets a new version (`manifest.json`, `Cargo.toml` and
`plugins/runtime/Cargo.lock`): an installation keeps its copy unless the
bundled version is higher, and refuses a copy whose files differ from the
pins of the same version (`../README.md`, "Versions").

| Version | Shipped in | What changed |
| --- | --- | --- |
| 0.1.0 | DeckPilot 0.7.0 to 0.9.0 | The first release |
| 0.1.1 | the next DeckPilot release | Rebuilt with the current plugin SDK and `deckpilot-first-party-support`; the manifest and the actions are unchanged |
