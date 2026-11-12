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
   *Empty* profile to check empty states. The Work profile has the
   integration examples *Start Work* (resolve the project, check Git and
   Docker, start SHOP's Compose services when Docker runs, open the editor,
   show a summary) and *End Work* (list uncommitted changes, stop the
   project's services when it has them, show a summary). They use only
   logical resources — `repository`, `compose` and `editor` of the current
   project — bound in the dev machine configuration (`.dev/workspace/shop`
   with a `compose.yaml`; TextEdit on macOS, Notepad on Windows; bind
   `dev.editor` yourself on Linux). Starting and stopping services only
   happens after Settings › Docker allows it. Demonstration data exists only in
   this seed; a production database starts with no profiles and an empty
   Action Library. The seed runs once per data directory, so demo data you
   delete does not come back (`make seed` with `--reset` starts over).
   A new development database is created by the agent like a fresh
   installation: encrypted, with a system-managed key in the credential
   store under the separate service `DeckPilot Development` (never the
   production `DeckPilot` entries); the demonstration data is written with
   the key the agent hands out, so only the agent reads the credential
   store. `deckpilot-dev seed --reset --password` creates a
   password-protected database instead, with the development-only password
   `deckpilot-dev`, which the supervisor uses to unlock the agent. `make up
   LOCKED=1` locks the configuration after the start to try the unlock
   screens (with a system key, unlocking uses Touch ID on macOS). The
   password fixture lives only in `tools/dev` and is never used by the
   application. Rebuilding the agent changes its code signature, so macOS
   asks for the login keychain password before the rebuilt agent may read
   its key ("Always Allow" holds for that build); `make up` points this out
   and waits. Frontend changes do not rebuild the agent. To develop the
   agent without these prompts, use a password-protected development
   database (`deckpilot-dev seed --reset --password`). An unencrypted
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
- **Window configuration** (`tauri.conf.json`: titlebar style, traffic-light
  position, window sizes) and **capabilities** are read when the window is
  created. Change them, then `make restart`; HMR cannot apply them.
- **Closing the app window** ends `tauri dev` and its Vite server; the agent
  keeps running. The tray's **Open DeckPilot** (or `deckpilot-dev open-ui`)
  starts the app again with its server: the supervisor gives its agent
  `DECKPILOT_DESKTOP_BIN` pointing at `deckpilot-dev` itself and
  `DECKPILOT_DEV_UI_LAUNCHER` naming the session's directory, so the agent
  asks *that* session's supervisor to run `tauri dev` again instead of
  starting the bare debug app, which has no server to load from. While the
  app runs, further clicks only bring its window to the front.
  `deckpilot-dev show` sends the same request as the tray.
- A debug build of the app started **without its Vite server** (for example
  `target/debug/deckpilot-desktop` on its own) shows a built-in page naming
  the server address, the data directory and how to start the server,
  instead of a blank window. Its Retry button checks again and opens the
  interface once the server answers. Release builds embed the frontend and
  never show this page.

| Command        | Purpose                                                        |
| -------------- | -------------------------------------------------------------- |
| `make up`      | Start (or report) the session; `DEVICES=mk2` limits simulation, `DEVICES=none` uses hardware only |
| `make down`    | Stop the session; data in `.dev/data` is kept                  |
| `make restart` | Stop, rebuild the agent, start again                           |
| `make status`  | Session processes and the live agent state                     |
| `make logs`    | Follow the agent and app logs (timestamped, colours removed)   |
| `make agent`   | Run only the agent in the foreground (dev data)                |
| `make ui`      | Run only the app with HMR in the foreground (dev data)         |
| `cargo run -p deckpilot-dev -- open-ui` | Start the session's app again if it is not running (what the tray does) |
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
  with a timestamp per line. Requests to start the app again are noted in
  `ui.log` (`[launcher]`) and `supervisor.log`.
- A second session next to the regular one (the integration preview, or a
  throwaway session for tests) needs its own directory and port:
  `DECKPILOT_DEV_DIR` moves `.dev` (data, logs, session file),
  `DECKPILOT_VITE_PORT` moves the Vite server, and `DECKPILOT_TAURI_CONFIG`
  names an extra Tauri configuration passed to `tauri dev --config`, whose
  `devUrl` must use that port (a separate `identifier` also keeps the
  WebView storage apart). Set them for `up`; the supervisor keeps using
  them when the tray restarts the app. The agent additionally keeps its own rotating
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

The required checks (Frontend, Storage with SQLCipher, Rust on ubuntu, macos
and windows, Updater signing keys match) and the Developer ID check run on
every pull request. The heavier optional jobs run on every push to `main`,
but on a pull request only when it changes files they depend on; otherwise
they report as skipped:

| Job | Runs on a pull request that changes |
| --- | ----------------------------------- |
| Package check (Linux .deb), Package check (Windows installers), in the Packaging workflow | files that affect bundling: `apps/desktop/src-tauri/` `tauri*.json`, `build.rs`, `Cargo.toml`, `capabilities/`, `icons/`, `app-icon.svg`; `apps/agent/` `build.rs`, `Cargo.toml`, `Info.plist`; `packaging/`, `tools/ci/`, `.github/workflows/`, `Cargo.lock`, `Cargo.toml`, `apps/desktop/package.json` |
| Credential store (3 platforms) | `crates/deckpilot-keystore/`, `tools/ci/with-keyring.sh`, `.github/workflows/ci.yml` |
| Release manifest assembly | `tools/ci/`, `.github/workflows/release.yml`, `.github/workflows/ci.yml` |
| Compatibility (own workflow) | its path filter in `compatibility.yml` |

The "Changed paths" job decides this with `tools/ci/changed-paths.sh` from
the files between the pull request's merge commit and its base, and tests
its rules first (`tools/ci/test-changed-paths.sh`). If the changed files
cannot be determined, every optional job runs. A skipped job can always be
run locally with the commands of its steps in `ci.yml`.

The Packaging workflow (`packaging.yml`) builds the Linux .deb and .rpm and
the Windows NSIS and MSI installers with the release configuration and
updater artifacts signed by a throwaway key, and checks them
(`tools/ci/check-packages.sh`). Besides pull requests that change the files
above, it runs on pushes to `main` that change anything under
`apps/desktop/src-tauri/`, `apps/agent/`, `packaging/` or `tools/ci/`, the
Cargo manifests or `apps/desktop/package.json` (so plain source changes are
packaged after they merge), nightly, for version
tags, by hand (Actions → Packaging → Run workflow) and when another workflow
calls it (`uses: ./.github/workflows/packaging.yml`; output `version`, the
checked version). A release should only be made from a commit with a green
Packaging run.

Runs and caches:

- A new push to a pull request cancels its obsolete run. Runs for commits on
  `main` (after a merge) are never cancelled; each commit has its own
  concurrency group, so they queue for runners.
- Only runs on `main` save Rust build caches; pull requests restore the cache
  of `main`. Per-branch caches of about 0.8–1.2 GB each pushed the cache of
  `main` out of the repository's 10 GB cache storage.
- CI builds with `CARGO_PROFILE_DEV_DEBUG=line-tables-only` (and the same for
  the test profile): backtraces keep file and line numbers, while builds,
  links and caches are smaller. `rust-cache` already sets
  `CARGO_INCREMENTAL=0`.

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
