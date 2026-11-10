# Development

## Prerequisites

- Rust via [rustup](https://rustup.rs/). The version is pinned in
  `rust-toolchain.toml` (currently 1.99.1, with `rustfmt` and `clippy`);
  rustup installs it automatically and CI uses the same file.
- Node.js 22 or newer and pnpm (version pinned in `package.json`).
- GNU Make (optional; every target is a thin wrapper around `cargo` and
  `pnpm`).
- Tauri system dependencies: <https://tauri.app/start/prerequisites/>
  - Debian/Ubuntu: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libudev-dev`
  - macOS: Xcode Command Line Tools
  - Windows: Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 10/11)

`make setup` installs the JavaScript dependencies, checks the toolchain
(`deckpilot-dev doctor`) and builds the agent once.

## Live development

```sh
make up        # or: pnpm dev (same session, in the foreground; Ctrl+C stops it)
```

`make up` starts a development session in the background and returns:

1. checks the toolchain;
2. creates `.dev/` (ignored by Git) with an **isolated data directory**
   `.dev/data` and seeds it with demonstration data on first use: a *Work*
   profile (SHOP → CMR → CMS, actions, a workflow and layouts for the MK.2,
   XL and Stream Deck +), a *Personal* profile with its own projects, and an
   *Empty* profile to check empty states. Demonstration data exists only in
   this seed; a production database starts with no profiles and an empty
   Action Library. The seed runs once per data directory, so demo data you
   delete does not come back (`make seed` with `--reset` starts over).
   The development database is encrypted like a real one, with the
   development-only password `deckpilot-dev`. The supervisor unlocks the
   agent with it, so the app opens without a prompt; `make up LOCKED=1`
   leaves it locked to try the unlock screens. The fixture lives only in
   `tools/dev` and is never used by the application. An unencrypted
   development database from an earlier version is left alone so that the
   app's migration can be tried;
3. builds and starts the background agent with simulated MK.2, XL and
   Stream Deck + devices next to any connected hardware;
4. starts the configuration app with `tauri dev` and Vite HMR.

Both processes run with `DECKPILOT_DATA_DIR=.dev/data`, so they use their
own database, socket and logs and never touch your real configuration. The
supervisor refuses to start if that directory would be the production one.

While the session runs:

- **Frontend changes** (React, TypeScript, Tailwind, shadcn/ui) appear
  immediately through HMR. Components keep their state where React Fast
  Refresh allows it; build and runtime errors are shown in an overlay in the
  window.
- **Changes to the app's Rust code** (or crates it uses) make `tauri dev`
  rebuild and restart the window. The agent keeps running.
- **Changes to the agent** need `make restart`, which rebuilds it. Frontend
  edits never restart the agent.

| Command        | Purpose                                                        |
| -------------- | -------------------------------------------------------------- |
| `make up`      | Start (or report) the session; `DEVICES=mk2` limits simulation, `DEVICES=none` uses hardware only |
| `make down`    | Stop the session; data in `.dev/data` is kept                  |
| `make restart` | Stop, rebuild the agent, start again                           |
| `make status`  | Session processes and the live agent state                     |
| `make logs`    | Follow the agent and app logs (timestamped, colours removed)   |
| `make agent`   | Run only the agent in the foreground (dev data)                |
| `make ui`      | Run only the app with HMR in the foreground (dev data)         |
| `make seed`    | Seed an empty dev database (`cargo run -p deckpilot-dev -- seed --reset` starts over) |
| `make clean`   | Remove build output, logs and session files (keeps dev data)   |

### How the session is managed

The session is run by `deckpilot-dev` (`tools/dev`), a small Rust program, so
it behaves the same on Linux, macOS and Windows:

- `.dev/session.json` records a session ID and, for every process it
  started, the PID **and the OS start time**. A process only counts as part
  of the session if both match, so a recycled PID is never signalled.
- A second `make up` reports the running session instead of starting
  duplicates. A session whose supervisor died is detected as stale and its
  verified processes are stopped before a new one starts.
- `make down` asks the agent to shut down over its IPC socket (it blanks the
  devices and releases them), then stops the app's process tree with a
  grace period. No pattern-based process killing is used.
- Logs are written to `.dev/logs/agent.log`, `ui.log` and `supervisor.log`
  with a timestamp per line. The agent additionally keeps its own rotating
  log in `.dev/data/logs/`. Credentials are not logged.

### Windows

The Makefile only calls `cargo` and `pnpm`, so it works with GNU Make from
Git for Windows or MSYS2. Without Make, run the supervisor directly:

```powershell
cargo run -p deckpilot-dev -- up
cargo run -p deckpilot-dev -- down
```

### Production builds

Development tooling is not part of release builds: HMR and the Vite server
exist only under `tauri dev`, simulated devices are only enabled by the
`--mock-devices` flag, and the WebView developer tools are only available in
debug builds.

## Building and packaging

```sh
make build     # debug build of the frontend and all Rust crates
make package   # installers for this platform, including the agent
```

`make package` builds the agent in release mode, places it in
`apps/desktop/src-tauri/binaries/deckpilot-agent-<target>` (ignored by Git)
and runs `tauri build` with `tauri.release.conf.json`, which bundles it next to
the application executable. Without `TAURI_SIGNING_PRIVATE_KEY` no updater
artifacts are produced. Releases are built by CI; see
[releasing.md](releasing.md).

The desktop crate embeds `apps/desktop/dist` at compile time, so run
`pnpm --filter @deckpilot/desktop build` before Cargo commands that include
the desktop crate on a fresh checkout (`make build` does this).

## Checks

| Purpose               | Command                                                     |
| --------------------- | ----------------------------------------------------------- |
| Everything            | `make check`                                                |
| Rust formatting       | `cargo fmt --all --check`                                   |
| Rust lints            | `cargo clippy --workspace --all-targets -- -D warnings`     |
| Rust tests            | `cargo test --workspace`                                    |
| SQLCipher storage     | `cargo test -p deckpilot-storage --features sqlcipher`      |
| Version consistency   | `cargo run -p deckpilot-dev -- check-versions`              |
| Frontend formatting   | `pnpm format:check` (fix with `pnpm format`)                |
| Frontend lint         | `pnpm lint`                                                 |
| TypeScript            | `pnpm typecheck`                                            |
| Frontend tests        | `pnpm test`                                                 |

CI (`.github/workflows/ci.yml`) runs the frontend checks on Linux and the
Rust checks on Linux, macOS and Windows. Agent tests use simulated devices and
real local IPC; no hardware is needed.

## Conventions

- Clippy runs with `pedantic` lints; CI treats warnings as errors.
- `unsafe` code is forbidden workspace-wide.
- TypeScript runs in strict mode with `typescript-eslint` strict type-checked
  rules. Files that export React components export nothing else, so Fast
  Refresh keeps component state.
- shadcn/ui components live in `apps/desktop/src/components/ui` and are added
  with `pnpm dlx shadcn@latest add <component>` from `apps/desktop`. They import
  `cn` from `@/lib/utils`.
- Serialized Rust types are mirrored in `apps/desktop/src/lib/types.ts`. The
  IPC tests in `apps/desktop/src-tauri/src/tests.rs` call the commands with the
  same argument shapes as `src/lib/api.ts`.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/).

## Adding an action type

Action types belong to a feature module (see [Feature modules](modules.md)).

1. Implement `ActionHandler` (definition + `execute`) in the module's crate
   under `modules/`, with an id of the form `<module id>.<name>`.
2. Return it from the module's `FeatureModule::action_handlers`.
3. Return `ContextEffect`s instead of mutating runtime state, and declare the
   capability the effect needs in the module manifest.
4. Add unit tests that run the handler with an `ExecutionContext`.

The Action Library and Workflow editor generate their forms from the type's
parameter schema, so no UI code is needed for a new type.
