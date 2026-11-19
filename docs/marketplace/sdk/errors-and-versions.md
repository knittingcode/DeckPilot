# Errors and versions

## Error codes

A failed action carries a **code** (`[a-z][a-z0-9]*(-[a-z0-9]+)*`, at most
64 characters), a message for the user (at most 1,024 characters, never a
secret) and whether a retry may help.

```rust
return Err(PluginError::new("repo-not-found", "GitHub does not know this repository"));
return Err(PluginError::new(codes::THROTTLED, "GitHub's rate limit").retryable());
```

`PluginError::new(code, message)`, `.retryable()` (honoured only for
actions whose node is `retrySafe`), `code()`, `message()`,
`is_retryable()` (`src/output.rs`).

### Shared codes

Defined by the design (§6.3 `SharedCode`), the same list in the SDK
(`deckpilot_plugin_sdk::codes::SHARED`) and the manifest crate
(`SHARED_ERROR_CODES`). A plugin may return them without declaring them
and must not redeclare them in `errorCodes`.

| Code | SDK constant | Produced by | Use it for |
| --- | --- | --- | --- |
| `not-permitted` | `NOT_PERMITTED` | SDK, from a broker denial (`CapabilityErrorKind::NotPermitted` via `?`) | A capability the user did not grant |
| `invalid-parameters` | `INVALID_PARAMETERS` | Agent (manifest check) and SDK (`Param` check) | Input the parameters' kinds cannot express (a blank name) |
| `credential-missing` | `CREDENTIAL_MISSING` | | A required credential slot without a secret (`has_credential` is false) |
| `request-failed` | `REQUEST_FAILED` | SDK, from `Failed`, `Unavailable`, `BadRequest`, `MalformedReply` without a network kind | |
| `throttled` | `THROTTLED` | SDK, from `Throttled` (retryable) | A remote rate limit |
| `storage-quota` | `STORAGE_QUOTA` | (the storage error's detail starts with it) | |
| `timeout` | `TIMEOUT` | | A remote timeout |
| `unsupported` | `UNSUPPORTED` | SDK: an unknown action id; `Unsupported` | |
| `plugin-error` | `PLUGIN_ERROR` | SDK: invalid definition, init failure, an undeclared parameter read | Bugs |
| `plugin-unavailable` | `PLUGIN_UNAVAILABLE` | Agent (keys, workflows): the plugin cannot run | |
| `invalid-result` | `INVALID_RESULT` | SDK: a result that breaks ABI 1 bounds or has a malformed code | |
| `blocked-address`, `dns-failed`, `connection-failed`, `tls-failed`, `too-many-redirects`, `insecure-redirect` | only in `SHARED`; `NetworkErrorKind::code` | SDK, from a failed `http.fetch`'s network kind ([capabilities.md](capabilities.md#errors)); `timed-out` (not shared) likewise | Network failures, as the built-in modules report them |
| `invalid-url`, `host-not-allowed`, `no-active-project`, `reference-missing`, `reference-type`, `reference-denied`, `insecure-credentials`, `sensitive-header` | `INVALID_URL`, `HOST_NOT_ALLOWED`; the rest only in `SHARED` | Reserved by the design; ABI 1 reports these situations as `not-permitted` or `request-failed` with a detail | Where they fit |

The agent adds codes of its own for keys, `runAction` and workflow nodes
(`apps/agent/src/plugins/engine.rs`, `service.rs` `engine_code`):
`cancelled` (the run was cancelled or passed its deadline; the plugin's
result is discarded), `plugin-host-failed` (crash, timeout, a refused
host), `plugin-busy` (8 invocations already queued), `plugin-revoked`,
`plugin-unavailable`. Through `invokePluginAction` these situations are
IPC errors with a `reason` instead (`hostFailed`, `busy`, `notEnabled`,
…; [agent-integration.md](../agent-integration.md#ipc-protocol-10-capability-plugins)).

### Your own codes

Declare every other code in `contributes.errorCodes` with a meaning:

```json
"errorCodes": {
  "repo-not-found": "GitHub does not know the repository or branch, or the token cannot see it.",
  "github-error": "GitHub answered with an error status."
}
```

Keep a `PLUGIN_CODES` constant and a unit test that each is declared (as
both examples with own codes do,
`permissions_codes_and_slots_are_declared`). The declaration documents the
code for users and the registry; **this version of the agent does not
refuse an undeclared code at run time**, only a malformed one (which the
SDK turns into `invalid-result`).

### Guidelines

- Map remote statuses to codes the user can act on (Developer Tools:
  401 → `github-unauthorized`, 403/429 → `throttled` retryable, 404 →
  `repo-not-found`, 5xx → `github-error` retryable).
- Messages name what failed and what to do, not values that might be
  sensitive. Never include a secret, a token or a full URL with a query.
- Never panic: a panic ends the host process (`plugin-host-failed`) and
  counts towards the crash-loop breaker.

## Versions

| Number | Where | Today | Compatibility |
| --- | --- | --- | --- |
| Guest ABI | `deckpilot_plugin_sdk::ABI_VERSION`, `dp_abi_version()`, manifest `runtime.abi` | **1** | The host loads only ABI 1 modules; the SDK asserts it at compile time. Additions to ABI 1 are optional exports and new capability ops; a module that does not use them is unchanged, and an op an older host or agent does not know is refused with an error, never misread. A field added to what the host **sends** (the network kind of a failed fetch) reaches only modules that report reading it from the optional export `dp_features`; modules without it get the replies they were built for |
| Plugin API level | `deckpilot_plugin_sdk::API_LEVEL` (1.0, what a plugin needs by default), `LATEST_API_LEVEL` (what the SDK implements), `Plugin::api_level()` (what the plugin needs), manifest `engines.pluginApi`, agent `PLUGIN_API` | **1.3** (1.1: `time.now`, `random.bytes`; 1.2: key state, `dp_state`; 1.3: action `controls`, plugin navigation) | `^MAJOR.MINOR`: same major, at least this minor. Declare `"^1.0"` unless the plugin cannot work without a later addition: then `Plugin::requires_api(..)` and the same level in the manifest (`manifest_mismatches` compares them), and an agent below it reports the version as `incompatible`. An operation an agent does not know is answered `Unsupported`, so a plugin that can do without keeps `"^1.0"` |
| DeckPilot | manifest `engines.deckpilot` | `>=0.5.0 <1.0.0` in the examples | A `VersionRange` matched against the agent's version, pre-releases included ([manifest.md](manifest.md#compatibility-engines-and-platforms)); an incompatible version is not offered for install (`incompatible`) |
| SDK crate | `deckpilot_plugin_sdk::SDK_VERSION`, manifest `engines.sdk` (informational) | **0.1.0** | Own semver; while 0.x, a minor bump may break the Rust API. Not published: depend on it by path inside `plugins/runtime` |
| Manifest | `manifestVersion` | **1** | The only version the agent reads (`MANIFEST_VERSION`) |
| Plugin | manifest `version`, `Plugin::new` | yours | `X.Y.Z[-alpha.N\|-beta.N\|-rc.N]`; updates compare by semver; a version that asks for more permissions needs consent |

The descriptor the module reports (`dp_describe`) carries the SDK
(`deckpilot-plugin-sdk/0.1.0`) and the API level; the supervisor uses it
to check the action ids.
