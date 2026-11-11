# Integration modules

Modules that connect DeckPilot to tools on this computer and elsewhere. They
are ordinary feature modules (see [Feature modules](modules.md)): compiled
in, listed in Settings › Modules, each with its own manifest, capabilities,
settings and action types. Their action types appear in the Action Library
and the workflow editor's catalog automatically; nothing is added to the
Action Library until you create an action.

Every action resolves machine-specific values (applications, folders,
programs, files) through logical resources bound on this computer (see
[Machine resources](data-model.md#machine-resources)), so an exported workflow works on
another machine once its resources are bound there.

Common to all of them:

- **Effects.** Each action type declares whether it only reads (`readOnly`),
  opens something (`opens`) or changes state (`modifies`); the catalog and
  the inspector show it.
- **Error codes.** A failed action reports a stable code with its message
  (`ActionResult.code`, `StepError.actionCode` in workflows). Codes shared
  by every module: `not-permitted` (capability missing), `no-active-project`,
  `resource-role-missing`, `resource-unbound`, `resource-kind-mismatch`,
  `program-not-found`, `spawn-failed`, `cancelled`, `timed-out`.
- **Sensitive outputs.** Outputs such as a program's output or a response
  body are passed to later nodes but never stored in execution history or
  sent in live run events.
- **Programs** are started without a shell, with an argument list, a
  minimal environment (`PATH`, `HOME`, locale and the system variables
  Windows needs; nothing else of the agent's environment), bounded output
  and a time limit, through `ExecutionContext::run_process`. Cancelling the
  run or reaching the time limit stops the program and everything it
  started.

## System (`system`)

Opens applications, web pages, files and folders, runs commands and checks
whether programs are running.

Capabilities: machine resources, open applications/files/pages, run
programs, see running programs.

Setting: **Allow running commands** (`allowCommands`, off by default). It
is machine-specific: never exported, and an import cannot turn it on.

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `system.open-app` | opens | `application` (application resource), `arguments` (one per line) | `application` |
| `system.open-url` | opens | `url` (`http`, `https` or `mailto`) | `url` |
| `system.open-file` | opens | `location` (resource), `path` (relative), `application` (optional application resource) | `path` |
| `system.open-directory` | opens | `location` (folder resource), `path` (optional relative subfolder) | `path` |
| `system.run-command` | modifies | `program` (resource), `arguments` (one per line), `workingDirectory` (optional folder resource; home folder otherwise), `timeoutSeconds` (1–3600, 60), `failOnNonZero` (on) | `exitCode`, `stdout`*, `stderr`*, `outputTruncated`, `durationMs` |
| `system.process-status` | read-only | `name` (program name; case and `.exe` ignored) | `running`, `count`, `pids` |

\* sensitive: available to later nodes, not kept.

Opening uses the platform's launcher, never a shell: `open` on macOS
(`-a` for an application path or name, `-b` for a bundle identifier),
`xdg-open` on Linux (an application is started directly, a `.desktop` id
with `gtk-launch`), and on Windows `rundll32 url.dll,FileProtocolHandler`
for pages and files, `explorer` for folders and the executable itself for
applications. Other URL schemes (`file:`, `javascript:`, custom handlers)
are refused because they can start programs. Relative paths cannot leave
their resource (`..`, absolute paths and drive letters are refused). An
opened application is independent of the workflow: cancelling does not
close it.

`system.run-command` runs only a program bound as a resource — an absolute
path or a name found on `PATH` — so an imported workflow cannot choose what
runs. Arguments are passed as they are, one per line: nothing is split on
spaces, quoted or expanded. Output is kept up to 64 KiB per stream
(`outputTruncated` tells when more was produced). The node is cancellable
and not retry-safe.

Error codes: `invalid-url`, `invalid-path`, `invalid-arguments`,
`not-found`, `commands-disabled`, `exit-nonzero`, `unsupported-target`,
`open-failed`, and the shared codes.

Opening cannot detect whether the opened program then shows what was
asked (a launcher reports only that it started); a launcher still running
after 10 s counts as success.

## Git (`git`)

Reads the state of project repositories and opens them. It never changes a
repository: there are no commit, reset, clean, checkout or push actions.

Capabilities: read profile, machine resources, run programs (`git`), open
folders.

Settings: **Git program** (`gitExecutable`; empty uses `git` on `PATH`;
machine-specific), **Repository resource** (`repositoryRole`, default
`repository`), **Network time limit** (`networkTimeoutSeconds`, 5–120, 20).

**Which repository.** Every action takes an optional `repository` (a folder
resource) and an optional `project_id`. Without a repository, the action
uses the `repository` resource of the chosen project, or of the workflow's
current project — the run's own project context, so parallel runs on
different projects each read their own repository. An explicit project must
belong to the action's profile (checked when saving and when running).
Choosing a project never changes DeckPilot's active project.

| Action type | Effect | Extra parameters | Outputs |
| ----------- | ------ | ---------------- | ------- |
| `git.status` | read-only | — | `branch`, `head`, `detached`, `unborn`, `upstream`, `clean`, `staged`, `unstaged`, `untracked`, `conflicts`, `ahead`, `behind` |
| `git.branch` | read-only | — | `branch`, `head`, `detached`, `unborn`, `upstream` |
| `git.changes` | read-only | `limit` (1–500, 100) | `changed`, `staged`, `unstaged`, `untracked`, `conflicts`, `files` (one `XY path` per line, as `git status --short`), `truncated` |
| `git.last-commit` | read-only | — | `hash`, `shortHash`, `author` (name only), `date` (ISO 8601), `subject` |
| `git.remote-status` | read-only | `refresh` (off) | `hasUpstream`, `source` (`cached`/`network`), `branch`, `upstream`, `remote`, `ahead`, `behind`, `trackingHead`, `remoteHead`, `stale` |
| `git.open-repository` | opens | `application` (optional application resource) | — |

Every action also outputs `repository` (the folder) and `projectKey` when
the repository came from a project.

**Detached HEAD, unborn branches, missing remotes.** A detached HEAD has
`detached` set and no `branch`; a branch without commits has `unborn` set
and no `head` (`git.last-commit` fails with `no-commits`). Without an
upstream, `git.remote-status` succeeds with `hasUpstream` false; when the
branch tracks a remote that is no longer configured, a refresh fails with
`no-remote`.

**Cached and fresh remote state.** By default `git.remote-status` reports
what was last fetched (`source: cached`): ahead/behind against the
tracking branch, without the network. With `refresh`, it also asks the
remote for its branch with `git ls-remote` (`source: network`), which
changes nothing locally: `remoteHead` is the remote's commit and `stale`
says whether it moved since the last fetch. Fetching itself is not done.

**Safety.** Git runs as `git -C <repository> --no-pager --no-optional-locks
-c core.fsmonitor=false …`, without a shell, with `GIT_TERMINAL_PROMPT=0`
(so credentials are never asked for) and the minimal environment plus
`SSH_AUTH_SOCK`. `--no-optional-locks` keeps `git status` from refreshing
the index, so reading never writes to the repository; turning off
`core.fsmonitor` keeps a repository's configuration from starting a
program. Credentials in URLs are removed from error messages. Local
commands are limited to 30 s; actions are cancellable.

Error codes: `not-found`, `not-a-repository`, `unsafe-repository`,
`git-not-found`, `git-failed`, `project-not-in-profile`, `no-commits`,
`no-remote`, `network-failed`, and the shared codes.

## Docker (`docker`)

Docker availability, containers, Compose projects and bounded logs. Uses
the Docker CLI with Compose v2 (`docker compose`).

Capabilities: read profile, machine resources, run programs (`docker`).

Settings: **Docker program** (`dockerExecutable`, machine-specific),
**Docker context** (`context`, machine-specific; empty uses Docker's
current context), **Allow starting and stopping services**
(`allowStateChanges`, off; machine-specific: never exported, an import
cannot turn it on), **Compose resource** (`composeRole`, default
`compose`), **Time limit for Compose changes** (`timeoutSeconds`, 10–1800,
300).

**Contexts.** Every command is `docker --context <name> …`: the action's
`context`, else the module's, else the context `docker context show`
reports. The name is part of every result (`context`), so it is visible
which engine was used; remote contexts (`ssh://`, TCP with TLS) work like
the local one. `DOCKER_HOST`, `DOCKER_CONTEXT`, `DOCKER_CONFIG`, TLS
variables and `SSH_AUTH_SOCK` are passed on.

**Compose projects** come from a resource bound to the Compose file or its
folder: the action's `compose` resource, else the `compose` resource of an
explicitly chosen project of the same profile, else of the workflow's
current project. `projectName` is passed as `-p` when set; the context and
the Compose project are named in every message.

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `docker.status` | read-only | `context` | `available`, `reason`, `clientVersion`, `serverVersion`, `serverOs`, `composeVersion`, `context` |
| `docker.containers` | read-only | `all` (off), `name` (filter), `context` | `count`, `running`, `names`, `containers` (name, state, image, status per line), `context` |
| `docker.compose-status` | read-only | compose target | `services`, `running`, `stopped`, `allRunning`, `containers`, `project`, `projectKey`, `context` |
| `docker.compose-up` | modifies | compose target, `services` (one per line) | `output`*, `project`, `projectKey`, `context` |
| `docker.compose-down` | modifies | compose target, `removeOrphans` (off) | same |
| `docker.compose-restart` | modifies | compose target, `services` | same |
| `docker.logs` | read-only | `container`, or `service` with a compose target; `lines` (1–2000, 100), `since` | `logs`*, `lineCount`, `truncated`, `context` |

The compose target is `compose`, `project_id`, `projectName` and `context`.
\* sensitive: available to later nodes, not kept.

`docker.status` succeeds when Docker cannot be used — that is its answer:
`available` is false and `reason` is `docker-not-found`,
`daemon-unavailable` or `permission-denied`. Other actions fail with
those codes.

**Safety.** The three Compose changes are refused with `changes-disabled`
until allowed on this computer. `compose up` runs `up --detach`;
`compose down` runs `down` and never removes volumes (`-v`) or images
(`--rmi`); `--remove-orphans` is added only when asked for. Contexts,
containers, services and project names are validated, so none can be
taken for an option, and services follow `--`. Logs are never followed:
the last lines up to 64 KiB. Labels and environments of containers are
never read into results. There are no Swarm operations.

**Cancellation and timeouts.** All actions are cancellable: the Docker CLI
is stopped with its process group. Cancelling `compose up` or `down`
partway leaves the project as far as Compose got; running the action again
completes it (both are retry-safe and idempotent). Reading commands are
limited to 30 s, changes to the module's time limit.

Error codes: `docker-not-found`, `daemon-unavailable`, `permission-denied`,
`context-not-found`, `compose-unavailable`, `compose-file-not-found`,
`not-found`, `invalid-name`, `changes-disabled`, `project-not-in-profile`,
`docker-failed`, and the shared codes.
