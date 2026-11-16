# Developer Tools

An example DeckPilot plugin for developers: small utilities that run
entirely inside the plugin, and GitHub checks that go through the agent's
permission broker. It uses each brokered capability of ABI 1, and
only what its manifest declares. Built with
the Rust plugin SDK (`deckpilot-plugin-sdk`, plugin API 1.0, guest ABI 1).
Experimental; not published.

## Actions

| Action | Parameters | Outputs | Capabilities |
| --- | --- | --- | --- |
| `timestamp` | `value` (Unix seconds or milliseconds, or RFC 3339), `from` (`auto`, `seconds`, `milliseconds`, `rfc3339`) | `iso`, `unixSeconds`, `unixMillis`, `readAs` | none |
| `base64` | `text`, `mode` (`encode`, `decode`), `alphabet` (`standard`, `url-safe`) | `result` | none |
| `json-format` | `json`, `style` (`pretty`, `compact`; keys come out sorted) | `result` | none |
| `uuid-v5` | `name`, `namespace` (`dns`, `url`, `oid`, `x500`) | `uuid` | none |
| `open-prs` | `repo` (`owner/name`; default: the `defaultRepo` setting) | `count`, `repo`, `checkedAt` | settings, credential check, one GET to `api.github.com/search/issues`, storage, variable `openPrs`, log |
| `build-status` | `repo`, `branch` (default: the `defaultBranch` setting, `main`) | `status`, `repo`, `branch`, `changed`, `workflow`, `url`, `checkedAt` | settings, credential check, one GET to `api.github.com/repos/…/actions/runs`, storage, variable `buildStatus`, a notification when the status changed and `notifyOnChange` is on |
| `last-values` | none | what the last two checks found, `storageBytes` | storage only |

`build-status` reports the latest GitHub Actions run of the branch: its
conclusion when finished (`success`, `failure`, `cancelled`, …), its
status otherwise (`queued`, `in_progress`, …), `none` without runs.

A plugin has no clock and no random source, so there is no "now" in
`timestamp` and no random (version 4) UUID or ULID; the time of a check
(`checkedAt`) is GitHub's `Date` header.

## Permissions

| Permission | Grant key | Why |
| --- | --- | --- |
| `network`: `api.github.com`, `GET` | `network:api.github.com` | The two GitHub checks; nothing else is reachable |
| `credentials`: `githubToken` | `credentials:githubToken` | An optional token, sent by the agent as `Authorization: Bearer` to `api.github.com` only |
| `notifications` | `notifications` | A notification when the build status changes, only with the `notifyOnChange` setting |

The token is a credential slot: the plugin checks whether one is stored
(`Context::has_credential`) and asks the agent to inject it
(`Request::bearer(&handle)`); the secret never enters the module, and the
agent redacts it from anything the plugin gets back. Without a token the
requests go out unauthenticated (public repositories, GitHub's lower rate
limit).

Revoking `network:api.github.com` also makes the credential slot that
names that host unusable: every call that uses it (`credentials.has`, a
request with the token) is refused as `not-permitted`, and no secret can be
injected, until the network permission is granted again. Everything else
keeps working: the settings, the storage (`last-values`), the
notifications grant and the four utilities.

## Settings, variables, storage

| | Key | Type | Default |
| --- | --- | --- | --- |
| Setting | `defaultRepo` | text, `owner/name` | none |
| Setting | `defaultBranch` | text | `main` |
| Setting | `notifyOnChange` | boolean | `false` |
| Variable | `plugins.acme.developerTools.openPrs` | integer | |
| Variable | `plugins.acme.developerTools.buildStatus` | string | |
| Storage | `last.openPrs`, `last.buildStatus` | JSON | |

Variables are machine-scoped and live in the agent's memory (gone after a
restart or a disable); the storage keeps the last results across both.

## Errors

Besides the shared codes: `invalid-input`, `repo-required`,
`repo-not-found`, `github-unauthorized`, `github-error` (retryable for
5xx), `invalid-response`, `output-too-large` (`manifest.json`
`contributes.errorCodes`). GitHub's 403 and 429 become the shared
`throttled`, retryable.

## Build, test, package

```sh
rustup target add wasm32-unknown-unknown                         # once
(cd plugins/runtime && cargo test --locked -p developer-tools)   # native tests, GitHub scripted
plugins/examples/build.sh developer-tools                        # -> plugins/runtime/target/examples/developer_tools.wasm
cargo run --locked -p deckpilot-plugin-registry --features publish --bin deckpilot-registry -- \
  package --plugin-dir plugins/examples/developer-tools \
  --module plugins/runtime/target/examples/developer_tools.wasm --out developer-tools-0.1.0.dpplugin
```

`apps/agent/tests/plugins_examples.rs` runs every action through the real
agent and the sandboxed host, with canned GitHub answers (never the real
API): settings, the token on the wire and never in an answer or a log,
both variables, storage, a revoke and a grant.

The SDK documentation is in [`docs/marketplace/sdk/`](../../../docs/marketplace/sdk/README.md).
