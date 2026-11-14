# @deckpilot/plugin-sdk (JavaScript, skeleton)

**Requires the QuickJS runtime, which is not yet available.** No DeckPilot
host can run JavaScript plugins today; this package is a versioned,
unpublished skeleton (`private: true`, version 0.1.0) that holds the type
declarations of plugin API 1.0 from the #191 design (`sdk.d.ts`).

Write plugins in Rust with `deckpilot-plugin-sdk`
(`plugins/runtime/guest/plugin-sdk`, sample in `plugins/examples/hello-weather`)
until the QuickJS engine guest exists (see `plugins/runtime/README.md`,
"QuickJS").

| File | Purpose |
| --- | --- |
| `index.d.ts` | The declarations (`definePlugin`, contexts, results, `PluginError`, shared codes) |
| `tsconfig.json` | Type check (`pnpm exec tsc -p plugins/sdk-js`) and the ESLint project for this directory |

Not part of the pnpm workspace (`pnpm-workspace.yaml` lists `apps/*`), so it
is never installed, built or published. The repository's `eslint .` and
`prettier --check .` cover it.
