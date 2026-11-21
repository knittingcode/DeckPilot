# deckpilot.gitlab

The first-party GitLab plugin: the replacement of the former built-in
`gitlab` module (removed in WP-C5), part of P11
(`docs/marketplace/module-migration.md` §9), and now its only
implementation. Bundled with class `migration`
(`../bundled.json`) and pinned in builds that enable bundled plugins.

**Routed since the cut-over (P11)**, in builds with plugins only:
`ACTION_ALIASES` (`crates/deckpilot-core/src/alias.rs`) maps every
`gitlab.<action>` to `deckpilot.gitlab.<action>`, and the agent's migration
plan for `gitlab` (`apps/agent/src/plugins/migration.rs`) installs this
plugin, copies the module's settings with the server you had entered and,
once you approve, imports the credential and allows exactly that server.
From then on the stored `gitlab.*` keys, workflow nodes and actions run
here; nothing stored is rewritten. Until you approve, after a decline,
for every computer that is not eligible (a server a plugin cannot reach,
a per-action credential other than the module's) and in a build or start
without plugins, they are unavailable with the reason: there is no
built-in to fall back to since WP-C5. An older app on the same data still
runs its built-in.
`apps/agent/tests/plugins_cutover_servers.rs` runs that upgrade end to
end.

Read-only. It talks to GitLab.com (`https://gitlab.com`, GET) or to the
self-managed GitLab **you enter** in its `API URL` setting and allow, with
your token from the credential slot `token` (the `PRIVATE-TOKEN` header),
and opens pages on that same GitLab only.

## A GitLab you enter

`API URL` is a user-entered origin
([permissions.md](../../../docs/marketplace/sdk/permissions.md#user-entered-origins)):

- The default, `https://gitlab.com/api/v4`, is the plugin's declared host
  and needs no consent of its own.
- Any other server (`https://code.example.com/gitlab/api/v4`) is saved
  normalized, and nothing is sent to it, no token and no request, until
  you allow exactly that server in Settings › Plugins › GitLab ›
  Configure. Requests are then limited to the path you entered and below.
- A server on a private network (a LAN address, a name such as
  `gitlab.internal`, or a name that resolves to a private address) needs
  the separate private-network consent for that server. This computer
  (`localhost`, loopback) and link-local addresses are never reachable.
- Changing the setting ends the consent at once; the new server asks
  again.
- `Open on GitLab` opens `https` pages of the same server only (its web
  address is the API URL without `/api/v4`), and only once you allowed the
  server.

## The contract

Every action of the built-in, under the same name, so the alias
`gitlab.<action>` → `deckpilot.gitlab.<action>` is valid. Two tests
keep it that way:

- `crates/deckpilot-modules/tests/contracts.rs` (`first_party`): the
  manifest, converted as the agent registers plugin actions, against the
  built-in's frozen 0.9.0 definitions
  (`crates/deckpilot-modules/legacy/gitlab.json`):
  `alias::incompatibilities` (§5.3) for every alias, and beyond it the
  definitions must be identical but for the documented differences
  below. Also the variables (none on either side),
  the settings and that the permissions are exactly the three below.
- `apps/agent/tests/plugins_gitlab.rs`: every action of the plugin through
  the real agent and sandboxed host, on canned GitLab answers. Until WP-C5
  it compared the plugin with the built-in on the same answers served
  from 127.0.0.1; now it is a plugin-only regression test holding the
  plugin to those answers: the same status, message, outputs and error
  code, 31 readings (success and every error
  code) and 5 pages to open on GitLab.com; 11 readings and 4 pages again
  on a GitLab under a path that the user enters (refused before the
  consent and after its revocation); the same requests but for the page
  size; and two network failures (`dns-failed`, `tls-failed`).

| Built-in | Plugin | Definition | Results |
| --- | --- | --- | --- |
| `gitlab.current-user` | `deckpilot.gitlab.current-user` | compatible: no `credential` parameter (below) | identical |
| `gitlab.project` | `deckpilot.gitlab.project` | compatible: no `credential` parameter | identical |
| `gitlab.merge-requests` | `deckpilot.gitlab.merge-requests` | compatible: no `credential`; `filter` and `limit` optional with the same defaults (a manifest has defaults only on optional parameters; the agent fills them in either way) | identical; more than 50 listed are read in two pages of 50 (GitLab's `next` link); a list GitLab does not count is counted in pages of 50, at most 10 (the built-in's 500 items in 5 pages of 100) |
| `gitlab.review-requests` | `deckpilot.gitlab.review-requests` | compatible: as `merge-requests` | identical |
| `gitlab.pipeline` | `deckpilot.gitlab.pipeline` | compatible: no `credential` | identical; jobs in pages of 50, at most 10 (the same 500) |
| `gitlab.open` | `deckpilot.gitlab.open` | compatible: `cancellable` (the agent can stop every plugin action; the built-in's cannot be interrupted) | identical; opened through `url.open`, the same page |
| variables | | identical: none | |

Parameters, labels, descriptions, kinds, defaults, output keys, labels,
kinds, `always` and `sensitive` flags, icons, effects, retry and
idempotency flags and timeouts are identical.

### Permissions

| Permission | What for |
| --- | --- |
| `network`: `gitlab.com`, GET; the origin slot `apiUrl` (a path, a private network) | reading from GitLab.com, or from the server you enter and allow |
| `credentials`: `token` | the token, sent to GitLab.com or the server you allowed, in `PRIVATE-TOKEN`, and nowhere else |
| `openUrl`: `gitlab.com`; the server of `apiUrl` | opening GitLab pages in the browser |

### Settings and credentials

| Built-in setting | Plugin | Rule |
| --- | --- | --- |
| `apiUrl` | `apiUrl` (same kind, default, portability) | an `https` address; the server is reached only with your consent to it (above). A plain `http` address on this computer, which the built-in accepts, is not reachable for a plugin and fails with `invalid-api-url` |
| `timeoutSeconds` | `timeoutSeconds` (same) | the time limit of one action, counted with GitLab's `Date` headers (a guest has no clock) |
| `credential` (a credential id, default `gitlab.token`) | the credential slot `token` (`PRIVATE-TOKEN`; GitLab.com or the allowed server only) | the per-action `credential` override is dropped by the alias when it is empty or the module's own (`ParamRule::DropIfDefault`); any other value is not eligible (the keys stay unavailable) until G5 |
| `allowPrivateHost` | not declared | replaced by the private-network consent per server, which is never assumed |

## What the plugin does not reproduce

All on failure paths, limits or the editor:

1. **Checks when an action is saved.** The built-in's `check` reports an
   invalid project, ref, credential name or page as soon as an instance is
   saved. The agent and the app check every plugin action on save against
   its manifest's schema (kinds, required parameters, choices, ranges,
   lengths); the plugin-specific checks (a project path, a ref name, a
   GitLab page URL) cannot be derived from a schema and plugin API 1 has
   no validation hook. They stay **run-time only**: the same errors, with
   the same codes and messages, come when the action runs.
2. **Failure messages.** `unauthorized` (401) and `credential-missing`
   name the plugin's slot and Settings › Plugins › GitLab instead of the
   credential id `gitlab.token` and Settings › HTTP › Credentials;
   `invalid-api-url` says what an API URL must be instead of naming the
   parser's reason; a network failure's message is the agent's explanation
   rather than the built-in's (also for `blocked-address`, where the
   built-in points to its own setting). Codes are the same. A server that
   is entered but not allowed, or whose consent was revoked, fails with the
   agent's `not-permitted`, which the built-in has no equivalent of. When
   the browser cannot be started for `open`, the code differs too: the
   launcher's in the built-in, the agent's `request-failed` here.
3. **`rate-limited` carries no data.** The built-in's failure has
   `retryAfterSeconds`; a plugin failure has a code, a message and
   `retryable` only. The wait is in the message ("try again in 30 s") and
   the failure is retryable. Nothing reads the field today.
4. **Time.** The time limit is counted from GitLab's first answer with its
   `Date` headers (one-second steps), and each request has the agent's
   15 s; rate-limit waits are counted from the answer's `Date`.
5. **Agent limits for plugins.** 60 requests per minute (counting a list
   GitLab does not count, or the jobs of a pipeline with 500 of them, needs
   up to 11), one opened page per 2 s and 20 per 10 minutes; an output
   text holds at most 16 KiB, so a list of merge requests or failing jobs
   longer than that is cut at a line (`listed` and `truncated` say so for
   merge requests, a last line "… and N more" for jobs). Answers over
   512 KiB (the agent's limit for plugins; the built-in reads up to 4 MiB)
   are read again 10 items per page, which merge requests with very long
   descriptions need; a page of 10 that is still too large is
   `invalid-response`.
6. **Pages given with a non-ASCII host name.** `open` compares a page's
   host with the saved server as text (the agent stores it in its ASCII
   form); a page URL written with the Unicode form of an internationalized
   host name is refused with `url-not-allowed` where the built-in would
   accept it. Pages GitLab itself returns use the ASCII form.

## Build

```sh
plugins/first-party/build.sh --out target/bundled-plugins   # with its pins
cd plugins/runtime && cargo test -p deckpilot-gitlab         # unit tests
```

No `url` crate in the module: its IDNA tables would double the module,
which ships in every installer. The few URLs it needs are built and
checked by `../support` (`deckpilot-first-party-support`), shared with the
other plugins for a server the user enters; its tests compare it with the
`url` crate.

## Versions

A changed plugin gets a new version (`manifest.json`, `Cargo.toml` and
`plugins/runtime/Cargo.lock`): an installation keeps its copy unless the
bundled version is higher, and refuses a copy whose files differ from the
pins of the same version (`../README.md`, "Versions").

| Version | Shipped in | What changed |
| --- | --- | --- |
| 0.1.0 | DeckPilot 0.7.0 to 0.9.0 | The first release |
| 0.1.1 | the next DeckPilot release | Rebuilt with the current plugin SDK and `deckpilot-first-party-support`; the manifest and the actions are unchanged |
