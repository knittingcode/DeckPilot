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
their resource (`..`, absolute paths and drive letters are refused), and
what they lead to must be inside it too: a link (symlink) inside the
resource that points outside it is refused with `invalid-path`, a link to
nothing is `not-found`; links that stay inside, and a resource folder that
is itself a link, are fine. An
opened application is independent of the workflow: cancelling does not
close it.

`system.run-command` runs only a program bound as a resource — an absolute
path or a name found on `PATH` — so an imported workflow cannot choose what
runs. Arguments are passed as they are, one per line: nothing is split on
spaces, quoted or expanded. Output is kept up to 64 KiB per stream
(`outputTruncated` tells when more was produced). The node is cancellable
and not retry-safe. In a workflow a node runs at most 10 minutes, so a
longer `timeoutSeconds` only applies when the action runs from a key.

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

**Key state.** Keys running `git.status` show the branch and a green
(clean), amber (uncommitted changes) or red (conflicts) mark; `git.branch`
keys show the branch. The repository is read with the same local `git
status` at most every 30 s while the key is shown (see
[Button state](modules.md#button-state)).

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
which engine was used — except a `docker.status` that found no Docker and
no context was chosen, since then no current context can be asked; remote contexts (`ssh://`, TCP with TLS) work like
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
limited to 30 s, changes to the module's time limit — in a workflow at most
10 minutes, the longest any node runs.

**Key state.** Keys show whether Docker is available (`docker.status`),
how many containers run (`docker.containers`) and how many of a Compose
project's containers are up (`docker.compose-*`; up and down keys also as a
toggle), read with `docker version`, `docker ps` or `docker compose ps` at
most every 30 s while the key is shown. Reading the state never starts or
stops anything and needs no permission to change containers.

Error codes: `docker-not-found`, `daemon-unavailable`, `permission-denied`,
`context-not-found`, `compose-unavailable`, `compose-file-not-found`,
`not-found`, `invalid-name`, `changes-disabled`, `project-not-in-profile`,
`docker-failed`, and the shared codes.

## HTTP (`http`)

HTTP requests and health checks.

Capabilities: send network requests, use stored credentials.

Settings: **Allow this computer and private networks** (`allowPrivateNetworks`,
off), **Private hosts allowed** (`privateHosts`, one per line, e.g.
`localhost` or `*.internal`) — both machine-specific: never exported, an
import cannot open them — and **Largest response kept** (`maxResponseKb`,
1–10240 KiB, 1024). Settings › HTTP also manages **credentials**.

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `http.request` | modifies | `method` (GET, POST, PUT, PATCH, DELETE), `url`, `query` (`name=value` per line), `headers` (`Name: value` per line), `body`, `bodyType` (json, text), credential, `timeoutSeconds` (1–300, 30), `followRedirects` (on), `failOnHttpError` (on), `extract` (JSON pointer) | `status`, `ok`, `contentType`, `headers`, `body`*, `value`*, `size`, `truncated`, `url`, `redirects`, `durationMs` |
| `http.health-check` | read-only | `url`, `expectedStatus` (`200`, `2xx`, `200-299`, `200,204`; `2xx`), `contains`, `timeoutSeconds` (1–120, 10), `failWhenUnhealthy` (on), credential | `healthy`, `status`, `reason`, `durationMs` |

The credential parameters are `credential` (a stored credential's name),
`authScheme` (`bearer`, `basic` with `user:password`, `header`, `query`) and
`authName` (header or parameter name; `X-API-Key`, `api_key`).
\* sensitive: available to later nodes, not kept.

A JSON body is checked before sending and sent as `application/json`;
`extract` reads one value from a JSON response with a JSON pointer (RFC
6901, e.g. `/data/0/id`), so later nodes can use it without any expression
language.

**Network boundary (SSRF).** Only `http` and `https`. Every address a host
resolves to is checked: loopback, private (RFC 1918), shared (100.64/10),
link-local (169.254/16, including cloud metadata services), unique local and
link-local IPv6, IPv4-mapped and NAT64 forms of those, multicast, reserved
and documentation addresses are refused unless allowed for all hosts or the
host is listed. A name resolving to both a public and a private address is
refused. The connection then goes to exactly the checked addresses, so DNS
cannot change between the check and the connection. Redirects (at most 5)
are followed one at a time and checked the same way; a redirect from
`https` to `http` is refused. No proxy is used.

**Credentials** are stored per computer (Settings › HTTP › Credentials) in
the encrypted database, listed by name and description only, and never in a
workflow or an export. A request sends one only over `https` (or to this
computer), never along a redirect to another origin. Headers and query
parameters that hold secrets (`Authorization`, `Cookie`, names containing
`token`, `secret`, `api-key`, `password`, `auth`, …) cannot be written into
a workflow, and URLs with a user name or password are refused, so secrets
always go through a stored credential. Secret response headers
(`Set-Cookie`, tokens) are shown as `[redacted]`; messages and the `url`
output leave out the query string. Nothing logs headers or tokens.

**TLS** uses the operating system's certificate store (rustls with the
platform verifier). Certificate verification cannot be turned off; a
refused certificate fails with `tls-failed`.

**Limits, timeouts and cancellation.** Responses are read up to the
module's limit (`truncated` tells when cut). `timeoutSeconds` bounds the
whole request including redirects; cancelling the run stops the request at
once. `http.request` is not retry-safe (it may change data);
`http.health-check` is retry-safe, idempotent and by default fails when
unhealthy (`unhealthy`), or reports `healthy: false` when told not to fail.

Error codes: `invalid-url`, `credentials-in-url`, `invalid-header`,
`invalid-query`, `sensitive-header`, `invalid-body`, `blocked-address`,
`dns-failed`, `connection-failed`, `tls-failed`, `too-many-redirects`,
`insecure-redirect`, `insecure-credentials`, `request-failed`,
`http-status`, `invalid-response`, `unhealthy`, `credential-missing`, and
the shared codes.

## GitHub (`github`)

Reads the signed-in user, repositories, open pull requests, review requests
and checks from GitHub or GitHub Enterprise, and opens GitHub pages. It is
read-only: only `GET` requests are sent, and there are no actions that
comment, approve, merge or change anything.

Capabilities: send network requests, use stored credentials, open pages.

Settings: **API URL** (`apiUrl`, `https://api.github.com`; for GitHub
Enterprise Server `https://<host>/api/v3`; machine-specific), **Token
credential** (`credential`, the *name* of a stored credential, default
`github.token`; portable), **Time limit** (`timeoutSeconds`, 1–120, 20; for
all requests of one action) and **API host may be on a private network**
(`allowPrivateHost`, off; machine-specific).

**Token.** Store a GitHub token (a fine-grained token with read access is
enough) as a credential in Settings › HTTP › Credentials and name it in the
module's settings, or per action in `credential`. The token is read through
the credential store only, sent as `Authorization: Bearer` to the API host,
and never appears in parameters, settings, exports, outputs, messages or
logs; every result is also checked for it and any appearance is replaced by
`[redacted]` (even when a server repeats it).

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `github.current-user` | read-only | — | `login`, `name`, `url`, `rateLimitRemaining` |
| `github.repository` | read-only | `repository` (owner/name) | `fullName`, `description`*, `defaultBranch`, `private`, `archived`, `openIssues` (issues and pull requests), `stars`, `forks`, `url`, `pushedAt` |
| `github.pull-requests` | read-only | `repository` (optional), `filter` (`all`, `author-me`, `review-requested`; `all` needs a repository), `limit` (1–100, 20) | `count`, `listed`, `items`*, `truncated`, `firstUrl` |
| `github.review-requests` | read-only | `repository` (optional), `limit` (1–100, 20) | same as `github.pull-requests` |
| `github.checks` | read-only | `repository`, `ref` (branch, tag or commit) or `pullRequest` (number) | `state` (`ok`, `failing`, `pending`, `none`), `sha`, `total`, `passed`, `failed`, `pending`, `failing` (names, one per line), `truncated`, `url` |
| `github.open` | opens | `url`, or `repository` with optional `pullRequest` | `url` |

Every reading action also takes `credential` (a stored credential's name;
empty uses the module's). \* sensitive: available to later nodes, not kept.

**Pull requests** come from GitHub's search (`is:pr is:open`, with
`repo:`, `author:@me` or `review-requested:@me`; across repositories
archived ones are left out). `count` is GitHub's total; `items` lists at
most `limit` of them, most recently updated first, one per line as
`owner/name#12 Title (@author)` (with `[draft]`), titles reduced to one
plain line. Messages give counts only, never titles. Review requests
include requests to your teams.

**Checks** combine the commit's check runs (latest per check) and commit
statuses. A check run passes when it succeeded, was neutral or skipped,
is pending until completed, and fails otherwise (failure, cancelled, timed
out, action required, stale); a status passes on success, is pending on
pending and fails on failure or error. `state` is `failing` when anything
failed, else `pending` when anything is pending, else `ok`, or `none` when
the commit has no checks. With `pullRequest`, the pull request's latest
commit is checked and `url` is the pull request's page.

**Opening** accepts only `https` pages on the GitHub web host that belongs
to the API (`github.com` for `api.github.com`, `<x>.ghe.com` for
`api.<x>.ghe.com`, the same host for GitHub Enterprise Server); other hosts,
schemes and URLs with user names fail with `url-not-allowed`.

**Network.** Requests go through the HTTP module's client and network
policy: the API host's addresses are checked before connecting and
redirects hop by hop; the token is sent only over `https` (or to this
computer), never to another origin. A host on a private network (GitHub
Enterprise Server inside a company network) needs **API host may be on a
private network**, which allows only that host. Pages of a list are
followed only on the API itself.

**Limits.** Each answer is read up to 4 MiB; check runs and statuses are
read for at most 5 pages of 100 (`truncated` tells when there were more).
The time limit covers all requests of one action; cancelling the run stops
the request at once.

**Failures.** `unauthorized` (401: the token is wrong, expired or revoked),
`forbidden` (403: e.g. a missing permission or SSO), `not-found` (404, which
GitHub also answers for private repositories the token cannot see),
`rate-limited` (primary or secondary rate limit; the result's data has
`retryAfterSeconds`, from `Retry-After` or the limit's reset time, else 60
for a 429 or a 403 whose message is about the rate limit), `github-error` (another status, with GitHub's message), and
`credential-missing` when no token is stored under the name.

Error codes: `invalid-api-url`, `invalid-repository`, `invalid-ref`,
`unauthorized`, `forbidden`, `not-found`, `rate-limited`, `github-error`,
`invalid-response`, `url-not-allowed`, `credential-missing`, the HTTP
module's network codes (`blocked-address`, `dns-failed`,
`connection-failed`, `tls-failed`, `insecure-credentials`,
`too-many-redirects`, `insecure-redirect`, `request-failed`), and the
shared codes.

## GitLab (`gitlab`)

Reads the signed-in user, projects, open merge requests, review requests
and pipelines from GitLab.com or a self-managed GitLab, and opens GitLab
pages. It is read-only: only `GET` requests are sent, and there are no
actions that comment, approve, merge, run pipelines or change anything.

Capabilities: send network requests, use stored credentials, open pages.

Settings: **API URL** (`apiUrl`, `https://gitlab.com/api/v4`; for a
self-managed GitLab `https://<host>/api/v4`, also under a path such as
`https://<host>/gitlab/api/v4`; machine-specific), **Token credential**
(`credential`, the *name* of a stored credential, default `gitlab.token`;
portable), **Time limit** (`timeoutSeconds`, 1–120, 20; for all requests of
one action) and **API host may be on a private network**
(`allowPrivateHost`, off; machine-specific).

**Token.** Store a GitLab personal, project or group access token with the
`read_api` scope as a credential in Settings › HTTP › Credentials and name
it in the module's settings, or per action in `credential`. It is read
through the credential store only, sent in the `PRIVATE-TOKEN` header to
the API host, and never appears in parameters, settings, exports, outputs,
messages or logs; every result is also checked for it and any appearance
is replaced by `[redacted]`.

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `gitlab.current-user` | read-only | — | `username`, `name`, `url`, `rateLimitRemaining` |
| `gitlab.project` | read-only | `project` (`group/project` with any subgroups, or the numeric id) | `path`, `description`*, `defaultBranch`, `visibility`, `archived`, `openIssues`, `stars`, `forks`, `url`, `lastActivityAt` |
| `gitlab.merge-requests` | read-only | `project` (optional), `filter` (`all`, `author-me`, `review-requested`; `all` needs a project), `limit` (1–100, 20) | `count`, `countExact`, `listed`, `items`*, `truncated`, `firstUrl` |
| `gitlab.review-requests` | read-only | `project` (optional), `limit` (1–100, 20) | same as `gitlab.merge-requests` |
| `gitlab.pipeline` | read-only | `project`, `ref` (branch, tag or commit) or `mergeRequest` (the number after `!`) | `state` (`ok`, `failing`, `pending`, `none`), `status` (GitLab's), `pipelineId`, `sha`, `total`, `passed`, `failed`, `pending`, `skipped`, `failing` (job names, one per line), `truncated`, `url` |
| `gitlab.open` | opens | `url`, or `project` (path) with optional `mergeRequest` | `url` |

Every reading action also takes `credential` (a stored credential's name;
empty uses the module's). \* sensitive: available to later nodes, not kept.

**Merge requests** are open ones, most recently updated first; `author-me`
and `review-requested` (and `gitlab.review-requests`) first read your user
id and filter by `author_id` or `reviewer_id`, across all projects you can
see when no project is given. `count` comes from GitLab's `X-Total`; when
GitLab leaves it out (very large lists), up to 5 pages of 100 are counted
and `countExact` is false if there were more. `items` lists at most
`limit`, one per line as `group/project!12 Title (@author)` (with
`[draft]`), titles reduced to one plain line. Messages give counts only.

**Pipelines.** With `ref`, the newest pipeline of that branch or tag (or
commit, for a 40-character SHA); with `mergeRequest`, its head pipeline.
`state` follows the pipeline's status: `success` is `ok`; `failed` and
`canceled` are `failing`; `skipped` is `none`; everything else (created,
pending, running, manual, scheduled, …) is `pending`; no pipeline is `none`.
Jobs are counted from the pipeline's latest jobs: success passes, a failure
that is allowed passes, failed and canceled fail, skipped and manual are
`skipped`, the rest are pending.

**Opening** accepts only `https` pages on the GitLab web address that
belongs to the API (the API URL without `/api/v4`, including any path it is
installed under); other hosts, paths, schemes and URLs with user names fail
with `url-not-allowed`. A project is opened by its path.

**Network and limits** are those of the GitHub module: the HTTP module's
client and network policy, the token only over `https` (or to this
computer) and never to another origin, next pages only on the API itself,
answers up to 4 MiB, jobs for at most 5 pages of 100 (`truncated`), one time
limit for the whole action, and cancellation at once.

**Failures.** `unauthorized` (401), `forbidden` (403, e.g. a token without
`read_api`), `not-found` (404, also for private projects the token cannot
see), `rate-limited` (429; `retryAfterSeconds` from `Retry-After` or
`RateLimit-Reset`), `gitlab-error` (another status, with GitLab's message),
and `credential-missing` when no token is stored under the name.

Error codes: `invalid-api-url`, `invalid-project`, `invalid-ref`,
`unauthorized`, `forbidden`, `not-found`, `rate-limited`, `gitlab-error`,
`invalid-response`, `url-not-allowed`, `credential-missing`, the HTTP
module's network codes, and the shared codes.

## Jira (`jira`)

Reads the signed-in user, single issues, and the issues assigned to you or
in a saved filter from Jira Cloud or Jira Data Center, and opens Jira pages.
It is read-only: only `GET` requests are sent, and there are no actions
that create, transition, assign, comment on or change anything.

Capabilities: send network requests, use stored credentials, open pages.

Settings: **Site** (`siteUrl`, e.g. `https://your-team.atlassian.net` or a
Data Center address, also under a path such as `https://<host>/jira`; no
default; machine-specific), **Jira** (`deployment`: `cloud` or
`data-center`, `cloud`; machine-specific), **Credential** (`credential`, the
*name* of a stored credential, default `jira.token`; portable), **Time
limit** (`timeoutSeconds`, 1–120, 20; for all requests of one action) and
**Site may be on a private network** (`allowPrivateHost`, off;
machine-specific).

**Credential.** Stored in Settings › HTTP › Credentials: for Jira Cloud
`email:api-token` (an Atlassian API token, sent as Basic authentication to
REST API 3); for Data Center a personal access token (sent as Bearer to REST
API 2). It is read through the credential store only and never appears in
parameters, settings, exports, outputs, messages or logs; every result is
also checked for the whole credential, its token part and its Basic
encoding, and any appearance is replaced by `[redacted]`.

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `jira.myself` | read-only | — | `displayName`, `account` (Cloud account id or Data Center user name), `timeZone` |
| `jira.issue` | read-only | `issue` (key such as `ABC-123`) | `key`, `summary`*, `status`, `statusCategory` (`todo`, `in-progress`, `done`, `unknown`), `type`, `priority`, `assignee`, `updated`, `url` |
| `jira.issues` | read-only | `source` (`assigned-to-me`, `filter`), `filterId` (the saved filter's number), `project` (key, optional), `includeDone` (off), `limit` (1–100, 20) | `count`, `countExact`, `listed`, `items`*, `truncated`, `firstKey`, `firstUrl` |
| `jira.open` | opens | `url`, or `issue` | `url` |

Every reading action also takes `credential` (a stored credential's name;
empty uses the module's). \* sensitive: available to later nodes, not kept.
The e-mail address of the account is never an output.

**Searches** are built from fixed templates only — `assignee =
currentUser()` or `filter = <number>`, optionally `AND project = "<KEY>"`
(project keys are checked to be letters, digits and `_`), `AND
statusCategory != Done` unless `includeDone`, ordered by last update — so
no search text from a workflow reaches Jira. On Data Center `count` is
Jira's `total`; Jira Cloud's search gives no total, so up to 5 pages of 100
keys are counted and `countExact` is false if there were more. `items`
lists at most `limit`, one per line as `ABC-1 Summary [Status] (@Assignee)`
(or `(unassigned)`), summaries reduced to one plain line. Messages give
keys, statuses and counts only, never summaries.

**Opening** accepts only `https` pages on the configured site (including
its path); an issue opens at `<site>/browse/<KEY>`. Other hosts, paths,
schemes and URLs with user names fail with `url-not-allowed`.

**Network and limits** are those of the GitHub and GitLab modules: the
HTTP module's client and network policy, the credential only over `https`
(or to this computer) and never to another origin, answers up to 4 MiB, one
time limit for the whole action, and cancellation at once.

**Failures.** `unauthorized` (401), `forbidden` (403), `not-found` (404,
also for issues the account may not see), `rate-limited` (429;
`retryAfterSeconds` from `Retry-After`), `jira-error` (another status, with
Jira's first error message, e.g. a filter that does not exist), and
`credential-missing` when nothing is stored under the name or a Cloud
credential is not `email:api-token`.

Error codes: `invalid-site-url`, `invalid-issue-key`, `invalid-project`,
`unauthorized`, `forbidden`, `not-found`, `rate-limited`, `jira-error`,
`invalid-response`, `url-not-allowed`, `credential-missing`, the HTTP
module's network codes, and the shared codes.

## Workspace (`workspace`)

Opens a project's working set: the repository, or a file in it at a line,
in an editor, and web pages in the browser. It only opens things — there is
no shell, nothing runs in a terminal and nothing is changed. Editors and
browsers are application resources bound on each computer, never program
names written into a workflow.

Capabilities: read profile, machine resources, open applications, files and
pages. No settings.

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `workspace.open-in-editor` | opens | `application` (the editor, an application resource), `editor` (`vscode`, `cursor`, `jetbrains`, `zed`, `sublime`, `other`; `other`), `location` (folder resource; empty: the project's `repository`), `path` (relative file or folder), `line`, `column` (1–10 000 000) | `path`, `application`, `located` |
| `workspace.open-urls` | opens | `urls` (one http/https address per line, at most 10), `browser` (optional application resource) | `opened`, `failed`, `urls`* |
| `workspace.open-project` | opens | `editor`, `editorRole` (`editor`), `repositoryRole` (`repository`), `docsRole` (`docs`), `urlRoles` (resource names, one per line, at most 10), `browser` | `projectKey`, `opened`, `openedRoles`, `missingCount`, `missing` |

\* sensitive: available to later nodes, not kept (addresses may carry
tokens). `urls` and `urlRoles` are lists (edited item by item, stored one per line).

**Editors.** Without a line the folder or file is opened with the editor
(`open -a <editor> <path>` on macOS, `<editor> <path>` elsewhere). With a
line, known editors get their own arguments: Visual Studio Code and Cursor
`--goto <file>:<line>[:<column>]`, JetBrains IDEs `--line N [--column M]
<file>`, Zed and Sublime Text `<file>:<line>[:<column>]`; `other` opens the
file without the line (`located` is false). A line needs a file, and a
column needs a line. When the editor resource is a command-line tool (a
path such as `/usr/local/bin/code`, `idea`, `zed` or `subl`), it is started
directly with these arguments on every system, so the line also reaches an
editor that is already running; with a macOS application bundle the
arguments go through `open -a … --args`, which only reaches an editor that
is not running yet. A tool given by path must exist.

**Paths** follow the System module's rules: relative to the folder, no
`..`, no absolute paths or drive letters, no links that lead outside the
folder, and the file or folder must exist (`invalid-path`, `not-found`).

**Pages** follow the System module's URL rules, limited to `http` and
`https` (no `mailto`, `file`, `javascript:` or application schemes:
`invalid-url`). Every address is checked before any is opened. They open in
the default browser, or in the `browser` resource (`open -a <browser> <url>`
on macOS, `<browser> <url>` elsewhere). If some pages cannot be opened the
node still succeeds with `failed`; it fails only when none opened.

**Project workspace.** `workspace.open-project` uses the run's current
project: it opens the `repositoryRole` folder in the `editorRole`
application, the `docsRole` resource (a page in the browser, or a folder in
the file manager) and each resource in `urlRoles` as a page. Empty role
names are skipped. Resources the project does not define, that are not
bound on this computer, are of the wrong kind or are not valid pages are
listed in `missing` (`<role>: <reason>`, one per line) and do not stop the
others; the node fails with `nothing-opened` only when nothing could be
opened.

Error codes: `invalid-position`, `no-urls`, `too-many-urls`,
`nothing-opened`, the System module's `invalid-url`, `invalid-path` and
`not-found`, the launch codes (`unsupported-target`, `program-not-found`,
`spawn-failed`, `open-failed`), and the shared codes.

## Notifications (`notifications`)

Shows notifications from workflows: as native desktop notifications and in
the configuration app's status bar.

Capabilities: read profile (for the workflow's name), show notifications.

Settings (portable): **Desktop notifications** (on), **Status bar** (on),
**Show from** (`minimumSeverity`: info, success, warning, error; info).

| Action type | Effect | Parameters | Outputs |
| ----------- | ------ | ---------- | ------- |
| `notifications.show` | opens | `title` (one line, ≤100), `body` (≤500), `severity` (info, success, warning, error), `critical` (off) | `shown`, `desktop`, `statusBar`, `reason` |
| `notifications.workflow-summary` | opens | `title` (default: the workflow's name), `details` (lines added below), `onlyOnFailure` (off), `critical` (off) | same, plus `summary` |

Title and text can contain outputs of earlier nodes (`{{ steps.<node>.output.<key> }}`).
They are plain text: control characters and bidirectional overrides are
removed, lengths are bounded, and nothing is rendered as HTML (markup is
escaped for Linux notification services, React escapes it in the app).
The node's message names the title only, so the text is not kept in
history.

`workflow-summary` reports how the run is going when the node runs: actions
that succeeded, failed (with their node ids), were skipped or cancelled, and
the time so far — never messages or outputs. Its severity follows the
result (error when something failed). With `onlyOnFailure` a clean run
shows nothing.

**Delivery.** The agent shows a notification in the status bar of every
open app window (protocol 6 `notification` event) and natively:
`osascript` on macOS (`display notification`, text passed as arguments),
`notify-send` on Linux, a PowerShell toast on Windows (text passed in
environment variables and inserted as XML text nodes). A notification that
cannot be shown anywhere — the app is closed and desktop notifications are
off, denied or unavailable — does not stop the workflow: the node succeeds
with `shown: false` and the `reason`, unless it is `critical`
(`notification-not-shown`). Notifications less severe than **Show from**
are not shown and do not fail. macOS cannot report that notifications are
denied for the helper; they are then counted as shown.

Error codes: `notification-not-shown`.
