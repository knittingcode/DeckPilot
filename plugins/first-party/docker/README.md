# deckpilot.docker

The first-party Docker plugin: the replacement of the former built-in
`docker` module (removed in WP-C6) and now its only implementation,
extracted under the owner's decision D1 of
2026-11-20 (`docs/marketplace/module-migration.md` §11, WP-B2 of the
extraction plan). Bundled with class `migration` (`../bundled.json`);
plugin API `^1.9`, so it validates only as a bundled, pinned first-party
plugin.

**Routed since WP-C1.** The alias block `docker.<action>` →
`deckpilot.docker.<action>` and the migration plan (`allowStateChanges`
→ offer `tools:docker:modify`, never derived) run every stored
`docker.*` key here once the user consented; until then, and with the
plugin disabled or uninstalled, they are unavailable with the reason (no
built-in fallback since WP-C6).

It never runs a process itself: every docker is the agent's `tool.run`
(§7 X3b, X3c) of the user's own `docker`. The agent adds the context (a
bound Docker environment's, else DeckPilot's setting, else Docker's
own), and for Compose the bound folder, its compose file and its project
name; the plugin only picks the subcommand and options its manifest
declares and parses the JSON.

## Permissions

| Kind | Declared | Tier | Why |
| --- | --- | --- | --- |
| `tools` | docker `context show`, `version` (`--format`), `compose version` (`--short`), `ps` (`--no-trunc`, `--format`, `--all`, `--filter`), `compose ps` (`--format`, `--all`), `compose logs` (`--tail`, `--since`, `--no-color`), `logs` (`--tail`, `--since`); with `modify: true` only `compose up` (`--detach`), `compose down` (`--remove-orphans`), `compose restart` | high (`tools:docker`), high (`tools:docker:modify`, optional: only Compose up, down and restart name it in `requiresPermissions`; off until the user allows it) | what the built-in runs; never `--project-name`, `-f`, `--context` (the agent's), never `start`/`stop`/`restart` of single containers (the built-in has no such action) |
| `resources` | (labels only) | medium | the Compose folder or file and the Docker environment handles; the plugin never sees a path |
| `profile` | `read: ["project"]` | medium | the active project's key for `projectKey` |
| `device` | `state: true` | low | key states |

Optional dependency on `deckpilot.projects` (the compose role).

## The contract

| Built-in | Plugin | Definition | Results |
| --- | --- | --- | --- |
| `docker.status` | `deckpilot.docker.status` | `context`; new optional `environment` (a Docker environment or folder resource) | identical outputs and messages; `available: false` with the built-in's reasons; without `environment` no resource is needed |
| `docker.containers` | `deckpilot.docker.containers` | `all`, `name`, `context`; new `environment` | identical; a `name` filter must be a valid container name (`invalid-name`) |
| `docker.compose-status` | `deckpilot.docker.compose-status` | `compose` (folder or file), `projectId` (was `project_id`), `projectName`, `context` | identical |
| `docker.compose-up` | `deckpilot.docker.compose-up` | as `compose-status`, `services` | identical; `changes-disabled` without `tools:docker:modify`, `not-user-initiated` outside a run the user or their work session started |
| `docker.compose-down` | `deckpilot.docker.compose-down` | as `compose-status`, `removeOrphans` | identical; never removes volumes or images |
| `docker.compose-restart` | `deckpilot.docker.compose-restart` | as `compose-up` | identical |
| `docker.logs` | `deckpilot.docker.logs` | as built-in, new `environment` | identical, 64 KiB (the agent's cap); `projectKey` declared (the built-in returned it undeclared) |
| settings | `composeRole`, `timeoutSeconds` | `dockerExecutable` and `context` are the agent's; `allowStateChanges` is the `tools:docker:modify` grant | `timeoutSeconds` can only shorten the agent's own limit |

**Differences, stated plainly**

- **Where Docker runs.** `status`, `containers` and container logs run in
  the `environment` parameter (its context is used), else, as with the
  built-in, without any resource or project: the agent runs them with its
  context in an empty folder of its own (`tool.run` without a handle).
  Compose's commands run in their Compose project.
- **`context` parameter.** The agent decides the context; an action's
  `context` must be the one Docker runs in there, else
  `context-unsupported` (bind a Docker environment for it instead).
- **`projectName`.** The agent pins the Compose project name of the bound
  folder or file (its `name:`, else the folder's); a `projectName` fails
  with `project-name-unsupported` rather than acting on another project.
  The `project` output stays declared but is never set.
- **Compose resources** must be a folder with a `compose.yaml`
  (`compose.yml`, `docker-compose.y(a)ml`) or a compose file binding, also
  bound as a Docker environment whose value is that folder or file (as the
  built-in's compose role allowed).
- **Logs** read at most 1,000 lines (the agent's `--tail` bound), also
  when `lines` asks for up to 2,000.
- **Key state.** No `tool.run` in a state call: keys show what the last
  run of the action found (`src/state.rs`), read back every 5 s, per key
  (the agent's `keyScope` of the key's resources). A Compose change reads
  `compose ps` afterwards so the project's status, up, down and restart
  keys follow it; up and down keys show the built-in's toggle.
- **The modify sub-grant is optional and off by default** (X3c). The
  plugin is enabled without `tools:docker:modify`; until the user allows
  it, Compose up, down and restart are listed as unavailable ("needs the
  permission `tools:docker:modify`") and never reach docker, while every
  read works; revoking it again stops only them
  (`apps/agent/tests/plugins_docker.rs`).

## Tests

- `src/tests.rs`, `src/cli.rs`: the built-in's CLI and state fixtures and
  every scenario of the former built-in's action tests on its recorded
  docker output with `MockHost`, the manifest against the definition, the
  schema and both locales, and the tools policy against what the actions
  run.
- `apps/agent/tests/plugins_docker.rs`: every action through the real
  agent, host and `tool.run` with a fake `docker` first on the agent's
  `PATH` (Unix): consent, the agent's context, compose folder, file and
  project name, a scheduled Compose down refused (`not-user-initiated`)
  before docker sees it, nothing running without consent.
