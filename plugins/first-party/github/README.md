# deckpilot.github

The first-party GitHub plugin: the replacement of the built-in `github`
module (`modules/github`), the first module to move
(`docs/marketplace/module-migration.md` §6, PR P8). Bundled with class
`migration` (`../bundled.json`) and pinned in builds that enable bundled
plugins. **Not routed yet**: `ACTION_ALIASES` has no `github` entry until
the cut-over (P9), so `github.*` instances keep running the built-in.

Read-only. It talks to `https://api.github.com` only (GET), with the
user's token from the credential slot `token` (bearer), and opens pages on
`https://github.com` only.

## The contract

Every action of the built-in, under the same name, so the alias
`github.<action>` → `deckpilot.github.<action>` is valid. Two tests keep it
that way:

- `crates/deckpilot-modules/tests/contracts.rs` (`first_party`): the
  manifest, converted as the agent registers plugin actions, against the
  built-in's definitions: `alias::incompatibilities` (§5.3) for every
  alias, and beyond it the definitions must be identical but for the
  documented differences below. Also the variables (none on either side)
  and the settings.
- `apps/agent/tests/plugins_github.rs`: every action of the plugin through
  the real agent and sandboxed host, on canned GitHub answers, against the
  built-in on the same answers served from 127.0.0.1: the same status,
  message, outputs and error code, 30 scenarios (success and every error
  code), the same opened pages, and the same requests but for the page
  size.

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
| `apiUrl` | `apiUrl` (same kind, default, portability) | only `https://api.github.com`; anything else fails with `invalid-api-url`. GitHub Enterprise and other origins keep the built-in (`not_eligible`) until G2 |
| `timeoutSeconds` | `timeoutSeconds` (same) | the time limit of one action, counted with GitHub's `Date` headers (a guest has no clock) |
| `credential` (a credential id, default `github.token`) | the credential slot `token` (bearer, api.github.com only) | the migration copies the secret into the slot after the user's consent (§5.5); the per-action `credential` override is dropped when it is empty or the module's own (`ParamRule::DropIfDefault`), any other value keeps the built-in (`not_eligible`) until G5 |
| `allowPrivateHost` | not declared | plugins never reach private networks; a computer with it on keeps the built-in (G3) |

## What the plugin does not reproduce

Only GitHub Enterprise or another API origin, a private API host and a
per-action credential other than the module's keep the built-in. The
rest works the same, with these differences, all on failure paths, limits
or the editor:

1. **Checks when an action is saved.** The built-in's `check` reports an
   invalid repository, ref, credential name or page as soon as an instance
   is saved. Plugin API 1 has no such hook: the same errors, with the same
   codes and messages, come when the action runs.
2. **Two failure messages.** `unauthorized` (401) and `credential-missing`
   name the plugin's slot and Settings › Plugins › GitHub instead of the
   credential id `github.token` and Settings › HTTP › Credentials. Codes are
   the same.
3. **`rate-limited` carries no data.** The built-in's failure has
   `retryAfterSeconds`; a plugin failure has a code, a message and
   `retryable` only. The wait is in the message ("try again in 30 s") and
   the failure is retryable. Nothing reads the field today.
4. **Network failures.** Where the built-in reports the transport's code
   (`dns-failed`, `connection-failed`, `tls-failed`, …) the plugin reports
   `request-failed` with the agent's reason; likewise when the browser
   cannot be started for `open` (the launcher's code in the built-in, the
   agent's `request-failed` here). Answers over 512 KiB (the
   agent's limit for plugins; the built-in reads up to 4 MiB) are
   `invalid-response`; with 50 items per page GitHub's answers stay far
   below it.
5. **Time.** The time limit is counted from GitHub's first answer with its
   `Date` headers (one-second steps), and each request has the agent's
   15 s; rate-limit waits are counted from the answer's `Date`.
6. **Agent limits for plugins.** 60 requests per minute (a `checks` of 500
   items needs up to 21), one opened page per 2 s and 20 per 10 minutes;
   an output text holds at most 16 KiB, so a list of pull requests or
   failing checks longer than that is cut at a line (`listed` and
   `truncated` say so for pull requests, a last line "… and N more" for
   checks).

## Build

```sh
plugins/first-party/build.sh --out target/bundled-plugins   # with its pins
cd plugins/runtime && cargo test -p deckpilot-github         # unit tests
```

No `url` crate in the module: its IDNA tables would double the module,
which ships in every installer (`src/urls.rs` builds and checks the few
URLs it needs; its tests compare it with the `url` crate).
