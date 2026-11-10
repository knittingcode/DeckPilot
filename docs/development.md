# Development

## Prerequisites

- Rust via [rustup](https://rustup.rs/). The version is pinned in
  `rust-toolchain.toml` (currently 1.99.1, with `rustfmt` and `clippy`); rustup
  installs it automatically, and CI uses the same file
- Node.js 22 or newer and pnpm (version pinned in `package.json`)
- Tauri system dependencies: <https://tauri.app/start/prerequisites/>
  - Debian/Ubuntu: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`
  - macOS: Xcode Command Line Tools
  - Windows: Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 10/11)

## Setup

```sh
pnpm install
```

## Running

```sh
pnpm dev
```

This starts Vite on port 1420 and the Tauri app in development mode. To keep
development data separate from your regular configuration, set
`DECKPILOT_DATA_DIR`:

```sh
DECKPILOT_DATA_DIR=/tmp/deckpilot-dev pnpm dev
```

Without it, the database is stored in the platform's application data
directory (`io.github.knittingcode.deckpilot`).

## Building

```sh
pnpm --filter @deckpilot/desktop tauri build              # release bundle
pnpm --filter @deckpilot/desktop tauri build --debug --no-bundle
```

The desktop crate embeds `apps/desktop/dist` at compile time, so run
`pnpm --filter @deckpilot/desktop build` before running Cargo commands that
include the desktop crate on a fresh checkout.

## Checks

| Purpose               | Command                                                     |
| --------------------- | ----------------------------------------------------------- |
| Rust formatting       | `cargo fmt --all --check`                                   |
| Rust lints            | `cargo clippy --workspace --all-targets -- -D warnings`     |
| Rust tests            | `cargo test --workspace`                                    |
| SQLCipher storage     | `cargo test -p deckpilot-storage --features sqlcipher`      |
| Frontend formatting   | `pnpm format:check` (fix with `pnpm format`)                |
| Frontend lint         | `pnpm lint`                                                 |
| TypeScript            | `pnpm typecheck`                                            |
| Frontend tests        | `pnpm test`                                                 |

CI (`.github/workflows/ci.yml`) runs the frontend checks on Linux and the Rust
checks on Linux, macOS and Windows.

## Conventions

- Clippy runs with `pedantic` lints as warnings; CI treats warnings as errors.
- `unsafe` code is forbidden workspace-wide.
- TypeScript runs in strict mode with `typescript-eslint` strict type-checked
  rules.
- shadcn/ui components live in `apps/desktop/src/components/ui` and are added
  with `pnpm dlx shadcn@latest add <component>` from `apps/desktop`. They import
  `cn` from `@/lib/utils`.
- Serialized Rust types are mirrored by hand in `apps/desktop/src/lib/types.ts`.
  The IPC tests in `apps/desktop/src-tauri/src/tests.rs` exercise the commands
  with the same argument shapes as `src/lib/api.ts`.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/).

## Adding an action type

1. Implement `ActionHandler` (definition + `execute`) in `deckpilot-core`, or in
   a separate crate if it needs platform-specific dependencies.
2. Register it in `builtin::handlers()` or on the `ActionRegistry` used by the
   application.
3. Return `ContextEffect`s instead of mutating runtime state.
4. Add unit tests that run the handler with an `ExecutionContext`.
