# @deckpilot/plugin-sdk (JavaScript, skeleton)

**Requires the QuickJS runtime, which is not yet available.** No DeckPilot
host can run JavaScript plugins today; this package is a versioned,
unpublished skeleton (`private: true`, version 0.1.0) that holds the type
declarations of plugin API 1.0 from the #191 design (`sdk.d.ts`).

Write plugins in Rust with `deckpilot-plugin-sdk`
(`plugins/runtime/guest/plugin-sdk`, sample in `plugins/examples/hello-weather`)
until the QuickJS engine guest exists (see `plugins/runtime/README.md`,
"QuickJS").

## Differences from ABI 1

The declarations are the #191 design, kept unchanged as the target of the
future QuickJS shim. What ABI 1 (and the Rust SDK) implements today
differs; a shim would have to bridge or drop these:

| Declaration | ABI 1 today |
| --- | --- |
| `ctx.settings` (synchronous object) | `settings.get`, a capability call per invocation; the shim would call it before `run` |
| `ctx.storage.usage()` → `{ used, quota }` | `storage.usage` → `{ usedBytes, keys, quotaBytes, maxKeys }` |
| `ctx.notify({ title, body?, severity?, desktop?, statusBar? })` → `"shown" \| "suppressed"` | `notify { title, body }` (title ≤ 64, body ≤ 300), no severity or result |
| `ctx.log.*(message, fields?)` | `log { level, message }`, no fields |
| `ctx.http.fetch` with `timeoutMs`, `maxBytes`, `followRedirects`, `signal`, binary bodies; `HttpResponse.url`, `truncated`, `redirects`, `bytes()` | `http.fetch { method, url, headers, body (text), auth? }` → `{ status, headers, body }`; 15 s, 512 KiB, redirects always followed and re-checked; response headers filtered |
| `ctx.variables.publish(id, value, scope?)`, `get(id)` | `variables.publish { id, value }` (machine scope only), `variables.clear`; no `get` |
| `onActivate`/`onDeactivate` with a context, `ctx.timers`, `onSettingsChanged` | `dp_init`/`dp_shutdown` without capabilities; no timers, no settings notification |
| `ActionDefinition.state`, `ctx.device`, `ctx.open`, `ctx.profile`, `ctx.project`, `ctx.progress`, `ctx.sleep`, `ctx.invocation`, `ctx.host` | Not in ABI 1 |
| Globals `crypto.randomUUID`, `setTimeout` | No clock or random source in a guest |

The Rust SDK's documentation lists what exists:
[`docs/marketplace/sdk/`](../../docs/marketplace/sdk/README.md).

## Files

| File | Purpose |
| --- | --- |
| `index.d.ts` | The declarations (`definePlugin`, contexts, results, `PluginError`, shared codes) |
| `tsconfig.json` | Type check (`pnpm exec tsc -p plugins/sdk-js`) and the ESLint project for this directory |

Not part of the pnpm workspace (`pnpm-workspace.yaml` lists `apps/*`), so it
is never installed, built or published. The repository's `eslint .` and
`prettier --check .` cover it.
