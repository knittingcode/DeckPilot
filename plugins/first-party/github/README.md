# deckpilot.github

The first-party GitHub plugin: the replacement of the former built-in
`github` module (removed in WP-C5), the first module to move
(`docs/marketplace/module-migration.md` §6, PRs P8 and P9), and now its
only implementation. Bundled with
class `migration` (`../bundled.json`) and pinned in builds that enable
bundled plugins.

**Routed since the cut-over (P9)**, in builds with plugins only:
`ACTION_ALIASES` (`crates/deckpilot-core/src/alias.rs`) maps every
`github.<action>` to `deckpilot.github.<action>`, and the agent's
migration plan for `github` (`apps/agent/src/plugins/migration.rs`)
installs this plugin, copies the module's settings and, once the user
approves, imports the token. From then on the stored `github.*` keys,
workflow nodes and actions run here; nothing stored is rewritten. Until
the user approves, after a decline, for every computer that is not
eligible (below) and in a build or start without plugins, they are
unavailable with the reason: there is no built-in to fall back to since
WP-C5. An older app on the same data still runs its built-in.
`apps/agent/tests/plugins_cutover.rs` runs that upgrade end to end.

Read-only. It talks to `https://api.github.com` (GET), or, since 0.2.0, to
the GitHub Enterprise Server API the user enters in `apiUrl`
(`https://<host>/api/v3`, the setting's user-origin slot, like
`deckpilot.gitlab`'s), with the user's token from the credential slot
`token` (bearer), and opens pages on `https://github.com` or on that
server. An entered server is reached only once the user allowed exactly
it; a server on a private network needs its own private-network consent.

## The contract

Every action of the built-in, under the same name, so the alias
`github.<action>` → `deckpilot.github.<action>` is valid. Two tests keep it
that way:

- `crates/deckpilot-modules/tests/contracts.rs` (`first_party`): the
  manifest, converted as the agent registers plugin actions, against the
  built-in's frozen 0.9.0 definitions
  (`crates/deckpilot-modules/legacy/github.json`):
  `alias::incompatibilities` (§5.3) for every alias, and beyond it the
  definitions must be identical but for the documented differences
  below. Also the variables (none on either side)
  and the settings.
- `apps/agent/tests/plugins_github.rs`: every action of the plugin through
  the real agent and sandboxed host, on canned GitHub answers. It compared
  the plugin with the built-in on the same answers (the same status,
  message, outputs and error code, 30 scenarios, the same opened pages
  and requests but for the page size) until WP-C5; now it is a
  plugin-only regression test holding the plugin to those answers; and
  two network failures (a name that does not resolve, a
  certificate that is not trusted) with the same transport codes,
  `dns-failed` and `tls-failed`.

| Built-in | Plugin | Definition | Results |
| --- | --- | --- | --- |
| `github.current-user` | `deckpilot.github.current-user` | compatible: no `credential` parameter (below) | identical |
| `github.repository` | `deckpilot.github.repository` | compatible: no `credential` parameter | identical |
| `github.pull-requests` | `deckpilot.github.pull-requests` | compatible: no `credential`; `filter` and `limit` optional with the same defaults (a manifest has defaults only on optional parameters; the agent fills them in either way) | identical; more than 50 listed are read in two pages of 50 (GitHub's `next` link) |
| `github.review-requests` | `deckpilot.github.review-requests` | compatible: as `pull-requests` | identical |
| `github.checks` | `deckpilot.github.checks` | compatible: no `credential` | identical; pages of 50, at most 10 (the built-in's 500 items in 5 pages of 100) |
| `github.open` | `deckpilot.github.open` | compatible: `cancellable` (the agent can stop every plugin action; the built-in's cannot be interrupted) | identical; opened through `url.open`, the same page |
| variables | | identical: none | |

Parameters, labels, descriptions, kinds, defaults, output keys, labels,
kinds, `always` and `sensitive` flags, icons, effects, retry and
idempotency flags and timeouts are identical.

### Settings and credentials

| Built-in setting | Plugin | Rule |
| --- | --- | --- |
| `apiUrl` | `apiUrl` (same kind, default, portability; its user-origin slot) | `https://api.github.com` needs no consent of its own; a GitHub Enterprise Server API (`https://<host>/api/v3`) is reached only after the user allowed exactly it. The migration carries the server the built-in used into this setting and asks for it (G2); a plain `http` address fails with `invalid-api-url` and is not eligible (`not_eligible`: the keys stay unavailable) |
| `timeoutSeconds` | `timeoutSeconds` (same) | the time limit of one action, counted with GitHub's `Date` headers (a guest has no clock) |
| `credential` (a credential id, default `github.token`) | the credential slot `token` (bearer, api.github.com or the allowed server in `apiUrl`) | the migration copies the secret into the slot after the user's consent (§5.5); the per-action `credential` override is dropped when it is empty or the module's own (`ParamRule::DropIfDefault`), any other value is not eligible (`not_eligible`) until G5 |
| `allowPrivateHost` | not declared | the `apiUrl` slot may be allowed on a private network, only with its own consent: the migration asks for it for the server the built-in used; with api.github.com the computer is not eligible (G3) |

## What the plugin does not reproduce

Only a private api.github.com, an API address no plugin can reach (plain
`http`) and a per-action credential other than the module's are not
eligible; since WP-C5 their keys are unavailable with the reason. On GitHub Enterprise Cloud with data residency
(`https://api.<name>.ghe.com`) the API works, but pages open on
`<name>.ghe.com`, which is not the allowed server: `open` fails there
with `not-permitted`. The
rest works the same, with these differences, all on failure paths, limits
or the editor. The numbers are P8's; its difference 4 (transport failures
reported as `request-failed`) is gone: the plugin is built with the SDK
of #257, exports `dp_features`, reads the agent's network kind and so
reports the built-in's transport code (`dns-failed`,
`connection-failed`, `timed-out`, `tls-failed`, `too-many-redirects`,
`insecure-redirect`, `blocked-address`) with the same retryability.

1. **Checks when an action is saved.** The built-in's `check` reports an
   invalid repository, ref, credential name or page as soon as an instance
   is saved. Since #257 the agent and the app check every plugin action
   on save against its manifest's schema (kinds, required parameters,
   choices, ranges, lengths), and a legacy `github.*` type served by the
   plugin refuses a `credential` override it cannot be given; but the
   plugin-specific semantic checks (repository `owner/name`, a ref
   name, a GitHub page URL) cannot be derived from a schema and plugin
   API 1 has no validation hook. They stay **run-time only**: the same
   errors, with the same codes and messages, come when the action runs.
2. **Failure messages.** `unauthorized` (401) and `credential-missing`
   name the plugin's slot and Settings › Plugins › GitHub instead of the
   credential id `github.token` and Settings › HTTP › Credentials; a
   network failure's message is the agent's explanation
   (`http.fetch to https://api.github.com failed (…)`) rather than the
   built-in's. Codes are the same. When the browser cannot be started for
   `open`, the code differs too: the launcher's in the built-in, the
   agent's `request-failed` here.
3. **`rate-limited` carries no data.** The built-in's failure has
   `retryAfterSeconds`; a plugin failure has a code, a message and
   `retryable` only. The wait is in the message ("try again in 30 s") and
   the failure is retryable. Nothing reads the field today.
5. **Time.** The time limit is counted from GitHub's first answer with its
   `Date` headers (one-second steps), and each request has the agent's
   15 s; rate-limit waits are counted from the answer's `Date`.
6. **Agent limits for plugins.** 60 requests per minute (a `checks` of 500
   items needs up to 21), one opened page per 2 s and 20 per 10 minutes;
   an output text holds at most 16 KiB, so a list of pull requests or
   failing checks longer than that is cut at a line (`listed` and
   `truncated` say so for pull requests, a last line "… and N more" for
   checks). Answers over 512 KiB (the agent's limit for plugins; the
   built-in reads up to 4 MiB) are `invalid-response`; with 50 items per
   page GitHub's answers stay far below it.

## Build

```sh
plugins/first-party/build.sh --out target/bundled-plugins   # with its pins
cd plugins/runtime && cargo test -p deckpilot-github         # unit tests
```

No `url` crate in the module: its IDNA tables would double the module,
which ships in every installer. The few URLs it needs are built and
checked by `deckpilot-first-party-support` (`../support/src/server.rs`,
whose tests compare it with the `url` crate), shared with the other
plugins for a server the user enters.

A changed plugin gets a new version (`manifest.json` and `Cargo.toml`,
0.2.0 for the GitHub Enterprise slot): the agent refuses a bundled copy
whose files differ from the pins of the same version. The pins are
written by the release build (`../build.sh`), never by hand.
