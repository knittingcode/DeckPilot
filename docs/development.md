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
   XL and Stream Deck +; the XL and Stream Deck + are layout design only,
   with no driver), a *Personal* profile with its own projects, and an
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
   Stream Deck + devices next to any connected hardware (XL and Stream
   Deck +: layout design only; no driver);
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

| Purpose             | Command                                                          |
| ------------------- | ---------------------------------------------------------------- |
| Everything          | `make check`                                                     |
| Rust formatting     | `cargo fmt --all --check`                                        |
| Rust lints          | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Rust tests          | `cargo test --workspace --locked`                                |
| SQLCipher storage   | `cargo test -p deckpilot-storage --features sqlcipher --locked`  |
| Version consistency | `cargo run --locked -p deckpilot-dev -- check-versions`          |
| Frontend formatting | `pnpm format:check` (fix with `pnpm format`)                     |
| Frontend lint       | `pnpm lint`                                                      |
| TypeScript          | `pnpm typecheck`                                                 |
| Frontend tests      | `pnpm test`                                                      |

CI (`.github/workflows/ci.yml`) runs the frontend checks on Linux and the
Rust checks on Linux, macOS and Windows. Agent tests use simulated devices and
real local IPC; no hardware is needed.

The required checks (Frontend, Storage with SQLCipher, Rust on ubuntu, macos
and windows, Updater signing keys match) run on every pull request; none of
them uses a secret. The Developer ID check needs the release environment
and runs only on a release tag ([releasing.md](releasing.md#checking-the-signing-configuration)). The heavier optional jobs run on every push to `main`,
but on a pull request only when it changes files they depend on; otherwise
they report as skipped:

| Job | Runs on a pull request that changes |
| --- | ----------------------------------- |
| Package check (Linux .deb), Package check (Windows installers), in the Packaging workflow | files that affect bundling: `apps/desktop/src-tauri/` `tauri*.json`, `build.rs`, `Cargo.toml`, `capabilities/`, `icons/`, `app-icon.svg`; `apps/agent/` `build.rs`, `Cargo.toml`, `Info.plist`; `packaging/`, `tools/ci/`, `.github/workflows/`, `Cargo.lock`, `Cargo.toml`, `apps/desktop/package.json` |
| Credential store (3 platforms) | `crates/deckpilot-keystore/`, `tools/ci/with-keyring.sh`, `.github/workflows/ci.yml` |
| Release manifest assembly | `tools/ci/`, `.github/workflows/release.yml`, `.github/workflows/ci.yml` |
| Dev tooling scripts (Linux, macOS) | `tools/dev/*.sh`, `tools/dev/tests/`, `.github/workflows/ci.yml` |
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
packaged after they merge), nightly, by hand (Actions → Packaging → Run
workflow) and when another workflow calls it
(`uses: ./.github/workflows/packaging.yml`; output `version`, the checked
version). The release workflow calls it before drafting a release, so tags do
not start it on their own.

Pre-release versions (`X.Y.Z-alpha.N`, `-beta.N`, `-rc.N`) build no .rpm
(RPM does not allow "-" in versions), and the MSI gets a numeric version
(WiX accepts only numbers): `alpha.N` → `X.Y.Z.N`, `beta.N` →
`X.Y.Z.(20000+N)`, `rc.N` → `X.Y.Z.(40000+N)` (`tools/ci/msi-version.sh`).
Callers may pass both as inputs (`prerelease`, `msi-version`); otherwise they
are derived from the version. `tools/ci/test-check-packages.sh` tests this on
fake packages in the "Release manifest assembly" job.

Merge queue (prepared, not enabled): `ci.yml` and `signing-check.yml` also
run for `merge_group` events, so the required checks would report for queue
groups under their usual names. The optional jobs then compare the group's
base commit with its head (every pull request in the group). The signing
job holds no secret and runs the group's own scripts. GitHub offers merge queues only for repositories owned by an
organization, so this needs the repository to move to one first.

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

## Maintainer workflow

`main` is protected: changes arrive only as pull requests, merged by squash
merge (no merge commits, no rebase merges) after their required checks
passed on a branch that is up to date with `main`. Contributors open pull
requests from a fork, as described in [CONTRIBUTING.md](../CONTRIBUTING.md).
The maintainer, who has write access, works on branches of this repository
with this cycle:

1. Push a feature branch and open a pull request against `main`.
2. Wait for the required checks; to update a branch, merge `main` into it
   (no rebase or force-push on shared branches).
3. Squash-merge: `gh pr merge <number> --squash --delete-branch`.
4. Check that the remote branch is gone (the repository also deletes head
   branches on merge): `git ls-remote --heads origin <branch>` prints nothing.
5. Remove the branch's worktree if one was used and nothing in it is
   uncommitted or unpushed: `git worktree remove <path>` (it refuses a dirty
   tree; never force it).
6. Delete the local branch with `git branch -d <branch>`, which refuses a
   branch with work that is not on its upstream.
7. Prune remote-tracking references and update `main`:
   `git fetch --prune origin && git switch main && git merge --ff-only origin/main`.

A branch that another open pull request uses as its base is deleted only
after that pull request has been retargeted to `main` and `main` has been
merged into it. Branches of pull requests closed without merging are kept
until their owner decides.

### After merges: worktrees

Parallel agents (and the maintainer) work in git worktrees: agents under
`.claude/worktrees/agent-*`, others as sibling checkouts such as
`../DeckPilot-preview`. Each one builds its own `target/`, so finished
worktrees fill the disk quickly. `tools/dev/worktree-maintenance.sh` finds
the ones that can go and removes only those. After a round of merges:

1. `tools/dev/worktree-maintenance.sh audit --fetch-pr-heads` lists every
   worktree with its class (read-only; the fetch only updates
   `refs/remotes/origin/pr/*`, so commits of squash-merged pull requests
   count as preserved). `report` sums it up, `disk-usage` shows the sizes.
2. `tools/dev/worktree-maintenance.sh prune --dry-run`, then `prune`:
   drops the metadata of worktrees whose directory is already gone.
3. `tools/dev/worktree-maintenance.sh cleanup` (a dry run by default) shows
   what would be removed. Only when no agent is running:
   `cleanup --apply`, optionally with `--artifacts`.

The classes:

| Class | Meaning | cleanup |
| ----- | ------- | ------- |
| ACTIVE | Locked (Claude Code locks an agent's worktree while the agent runs, and the lock stays after it finishes), or a process other than a macOS indexer (`com.apple.*`, `mds`, `mdworker`) has a file or its working directory inside | never touched |
| REQUIRED | Uncommitted changes, untracked files that are not ignored, commits on no remote ref that are not patches already in `main` (`git cherry`), an open pull request (`gh`), another worktree inside it, or the main checkout | kept; with `--artifacts`, build directories go if it is inactive |
| UNCERTAIN | Something could not be determined: `gh` unavailable (pull request state unknown), a lock whose recorded process is gone, a git error, a missing directory | kept |
| SAFE | None of the above | removed with `git worktree remove` (never `--force`) |

Rules the tool keeps:

- `cleanup --apply` refuses to run (exit 3) while any worktree is ACTIVE,
  because agents may be running. `--allow-active-elsewhere` lets it proceed,
  but even then ACTIVE worktrees are never touched.
- Each worktree is re-checked (lock, processes, git state, pull requests)
  right before it is removed; `git worktree remove` itself refuses a dirty
  or locked tree.
- Locked worktrees are never unlocked on a guess. A finished agent's lock is
  lifted only with `--unlock-finished <name>`, given per worktree, and only
  right before removal (the lock is restored if removal fails).
- `--artifacts` deletes only `target`, `node_modules` and
  `apps/desktop/dist` directories that git ignores and that hold no tracked
  file, only in worktrees that are not locked and have no process, and never
  in the main checkout.
- Branches are never deleted (see the cycle above for that). Every deletion
  is logged with path, branch, HEAD and size to
  `~/.cache/deckpilot/worktree-maintenance.log` (`--log` to change), and a
  second run is a no-op.

There is no daemon or scheduled cleanup: run it by hand when no agents are
active. The tests (`tools/dev/tests/worktree-maintenance.test.sh`) build a
throwaway repository with one worktree per case; CI runs them on Linux and
macOS.

## Worktrees and build caches

Cargo puts build output in `<workspace>/target`. This repository has two
workspaces (the root and `plugins/runtime`), so every worktree gets two
target directories, 1–18 GB together, and a debug build of the main checkout
reaches 55 GB, of which `target/debug/incremental` alone can take 5 GB or
more. With a dozen worktrees that is the whole disk. The options:

- **One shared `CARGO_TARGET_DIR`** for all worktrees: the least disk, but
  cargo takes an exclusive lock on the target directory for the whole build
  ("Blocking waiting for file lock on build directory"), so parallel agents
  wait for each other, and differing features or flags between worktrees
  rebuild crates back and forth. Worse, a worktree can silently run
  another worktree's code (see [Slots and worktrees](#slots-and-worktrees)).
  Do not use it.
- **A bounded pool of target directories**, one per concurrent build:
  `tools/dev/target-slot.sh` (below). Disk use is bounded by the number of
  slots instead of the number of worktrees, builds in different slots run
  at the same time, and dependencies are reused when a worktree gets a slot
  another one built in. Recommended.
- **sccache** (not installed here): caches compiled crates across
  directories, so a fresh target directory fills faster. It does not share
  incremental state, cannot cache everything (proc-macro and build-script
  heavy crates, linking), and adds a daemon of its own. Optional on top of
  the pool; not needed for it.
- **`CARGO_INCREMENTAL=0`** for agent and CI-like runs: no
  `target/debug/incremental`, at the cost of slower rebuilds while editing.
  CI already runs without it (`rust-cache` sets it).
- **`CARGO_PROFILE_DEV_DEBUG=line-tables-only`** (and `..._TEST_DEBUG`):
  smaller artifacts with file and line numbers kept in backtraces; CI uses
  it.
- **`cargo clean`** (or `cleanup --artifacts`) once a worktree is done.

`tools/dev/target-slot.sh` hands out slots under
`~/.cache/deckpilot/target-pool/slot-1` … `slot-N`
(`DECKPILOT_TARGET_SLOTS`, default 3; `DECKPILOT_TARGET_POOL`). A slot is
held by one process at a time with a lock directory (macOS has no `flock`;
the shell creates a file in it with `O_EXCL`, since not every `mkdir`
command is exclusive) that records the owner's PID and start time; a lock
whose owner is gone, or whose PID now belongs to another process, is taken
over. A worktree gets the slot it used last when that one is free, so its
incremental state is reused.

```sh
# One command in a slot, released when it ends (waits up to 10 min):
tools/dev/target-slot.sh run --wait 600 -- cargo test --workspace --locked

# Or for the current shell:
eval "$(tools/dev/target-slot.sh env --owner $$)"
cargo clippy --workspace --all-targets --locked -- -D warnings
tools/dev/target-slot.sh release --owner $$

tools/dev/target-slot.sh status   # holders and sizes
```

Two builds in different slots do not block each other, while two builds
sharing a target directory do: `tools/dev/tests/target-slot.test.sh`
builds a tiny crate whose build script sleeps 4 seconds, twice at once. In
one target directory the second build waits for the first (about 9–10 s in
total); in two slots both finish in about 4–5 s. The test also runs 2–6
concurrent acquisitions against 2 slots and checks that no slot is ever
held twice. Note that cargo's package cache in `~/.cargo` has a lock of its
own, held only briefly while dependencies are resolved or downloaded.

### Slots and worktrees

A target directory shared by two worktrees is not safe as such. Cargo
names the output of a path package (every crate of this repository) after
a hash that includes the package's path *relative to the workspace root*,
not the absolute path, so `crates/deckpilot-core` in worktree A and in
worktree B get the same file names (`deps/libdeckpilot_core-<hash>.rlib`,
`.fingerprint/deckpilot-core-<hash>`). Path packages are judged fresh by
mtime: their output is reused when it is newer than every source file. So
if B builds a crate after A last edited it, A's next build reports the
crate as `Fresh` and links B's version. A's tests then run against B's
code until `cargo clean -p`. Shown with a two-crate workspace copied to
two directories that share one `CARGO_TARGET_DIR` (cargo 1.99.1,
`CARGO_INCREMENTAL=0`): after building B, a build of A prints B's string
and `cargo build -v` in A says `Fresh` for both crates. Dependencies from
crates.io and git are not affected: their hash covers the version and the
source, and their fingerprint is that version, not an mtime.

So `target-slot.sh` gives each slot an affinity: it records the worktree
that used it last (`.deckpilot-slot-user`), and when another worktree
acquires it, it first removes the output of that worktree's own packages:
every `[package]` name of a `Cargo.toml` in the worktree (both workspaces,
the excluded `plugins/` crates, examples), in every profile and target
directory, namely fingerprints (which forces cargo to rebuild them),
artifacts in `deps`, build script output, incremental state, binaries
copied into the profile directory, `examples`, and docs. Dependencies stay,
so a slot that changes worktree rebuilds only this repository's crates. If
no package can be found, the slot is emptied; if a fingerprint could not be
removed, acquisition fails. A slot that the same worktree used last is not
touched. `tools/dev/tests/target-slot.test.sh` reproduces the bug with two
copies of a small workspace built alternately through one slot (a member
with a `rerun-if-changed` build script and an excluded path crate) and
checks that each build runs its own code.

The alternatives considered:

- **A target directory per worktree, keyed by a hash of its path, with LRU
  eviction**: correct, but shares nothing (every new worktree builds all
  dependencies again) and bounds disk only by evicting whole directories.
- **`build.build-dir` (`CARGO_BUILD_BUILD_DIR`)**, stable in this toolchain,
  with the `{workspace-path-hash}` template: the same as the previous point
  for intermediate files, and unbounded.
- **Removing only fingerprints**: enough to force rebuilds, but leaves the
  other worktree's binaries in `target/debug`, where a direct run would
  pick them up.

The affinity is keyed by the worktree of the current directory when the
slot is acquired. Build in that worktree: do not point cargo at another
worktree with `--manifest-path`, and do not `cd` into another worktree in a
shell that holds a slot from `env`; release it and acquire again there.
Removing worktrees (`worktree-maintenance.sh cleanup`) needs nothing extra:
a slot whose last user is gone is scrubbed by its next user.

Slots are opt-in and meant for agents' checks (`cargo check`, `clippy`,
`test`). Commands that expect `./target` keep the default: `make up` and
the hardware test (they start `target/debug/deckpilot-agent`),
`deckpilot-dev package`, and `tools/compat/matrix.sh`. Slots live outside
the repository; build output never goes into git.

Recommended settings for agents:

- `export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only`
- Run cargo through `tools/dev/target-slot.sh run --wait 600 -- …` (both
  workspaces may use the same slot; `plugins/runtime` builds land next to
  the root's).
- After merges, `worktree-maintenance.sh cleanup` as described in
  [After merges: worktrees](#after-merges-worktrees), with `--artifacts`
  for the inactive worktrees that must stay.

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
