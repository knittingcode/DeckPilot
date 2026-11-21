# deckpilot.git

The first-party Git plugin: the replacement of the former built-in `git`
module (removed in WP-C6) and now its only implementation, extracted under the owner's decision D1 of 2026-11-20
(`docs/marketplace/module-migration.md` §11, WP-B1 of the extraction
plan). Bundled with class `migration` (`../bundled.json`); plugin API
`^1.9`, so it validates only as a bundled, pinned first-party plugin.

**Routed since WP-C1.** The alias block `git.<action>` →
`deckpilot.git.<action>` and the migration plan run every stored `git.*`
key here once the user consented; until then, and with the plugin
disabled or uninstalled, they are unavailable with the reason (no
built-in fallback since WP-C6).

Read-only. It never runs a process itself: every git is the agent's
`tool.run` (§7 X3a) of the user's own `git`, with the agent's fixed prefix
(`-C <bound folder> --no-pager --no-optional-locks -c core.fsmonitor=false
…`), a cleared environment, the agent's check that the work tree is the
bound folder, and its time and output limits. The plugin only picks the
subcommand and options its manifest declares and parses the output.

## Permissions

| Kind | Declared | Tier | Why |
| --- | --- | --- | --- |
| `tools` | git `status` (`--porcelain=v2`, `--branch`, `-z`, `--untracked-files`), `log` (`-1`, `--no-show-signature`, `--format`), `config` (`--get`), `remote`, `rev-parse` (`--verify`, `-q`), `ls-remote`; `network: true` | high (`tools:git`), high (`tools:git:network`) | exactly what the former built-in ran; `ls-remote` only for `remote-status` with `refresh`, the only network use |
| `resources` | `paths: true`, `open: true` | medium, medium, high | the repository folder handle; its path for the `repository` output and messages (as the built-in); `open-repository` |
| `profile` | `read: ["project"]` | medium | the active project's key for `projectKey` |
| `device` | `state: true` | low | `status` and `branch` keys |

Optional dependency on `deckpilot.projects`: without a `repository`
parameter the actions use the project's repository role, which only
resolves with a project context.

## The contract

| Built-in | Plugin | Definition | Results |
| --- | --- | --- | --- |
| `git.status` | `deckpilot.git.status` | same parameters (`project_id` is `projectId`: a manifest key is camel case; WP-C1's alias needs that rename) | identical outputs, messages, codes |
| `git.branch` | `deckpilot.git.branch` | as `status` | identical |
| `git.changes` | `deckpilot.git.changes` | `limit` optional with the same default (a manifest has defaults only on optional parameters) | identical |
| `git.last-commit` | `deckpilot.git.last-commit` | as `status` | identical; `log --format=format:…` (the agent accepts only explicit formats) |
| `git.remote-status` | `deckpilot.git.remote-status` | `refresh` optional with the same default | identical; `no-remote` also when the agent's own `git remote` check refuses the remote |
| `git.open-repository` | `deckpilot.git.open-repository` | as `status`, `application` an application resource | identical message; opened through `resource.open` (the system's handler without an application), in runs the user started only |
| settings | `repositoryRole`, `networkTimeoutSeconds` | `gitExecutable` is gone: the executable is the agent's (its `PATH`, later DeckPilot's own setting), never the plugin's | `networkTimeoutSeconds` can only shorten the agent's own network limit |

Error codes: the built-in's (`not-found`, `not-a-repository`,
`unsafe-repository`, `git-not-found`, `git-failed`,
`project-not-in-profile`, `no-commits`, `no-remote`, `network-failed`),
`no-active-project`, the agent's resource codes it passes on
(`resource-unbound`, `resource-role-missing`, `resource-kind-mismatch`),
`timeout` and `cancelled`.

**Differences, stated plainly**

- **Key state.** The agent allows no `tool.run` in a state call and gives
  it no resource handle, so a `status` or `branch` key no longer reads the
  repository every 30 s by itself. Every run of the action (a press, a
  workflow node) stores what it found (`src/state.rs`), and the key shows
  that, read back every 5 s; a failed run shows as an error, a key that
  has not run yet shows its own design. State is kept per key: under the
  agent's `keyScope` of the key's `repository` binding (and the active
  project where it follows it) and `projectId`.
- **`projectKey`** comes from the active project (`profile.read`): a role
  resolved for another project named in `projectId`, and a `repository`
  parameter that is itself a project role, report none.
- **A repository inside a larger one**: the built-in read the enclosing
  repository; the agent refuses any repository whose work tree or git
  directory is outside the bound folder (`not-a-repository`).
- Credentials in URLs are removed from git's messages by the agent
  (`https://host/…`) instead of masked (`https://***@host/…`).
- "Try it" in the plugin's page (`invokePluginAction`) cannot name a
  resource: only keys and workflow nodes run the actions.

## Tests

- `src/tests.rs`, `src/porcelain.rs`: the built-in's porcelain and state
  fixtures, every action on recorded git output with the agent replaced by
  `MockHost` (the built-in's scenarios), the manifest against the
  definition, the schema and both locales, and the tools policy against
  what the actions run.
- `apps/agent/tests/plugins_git.rs`: every action through the real agent,
  host and `tool.run` with a fake `git` first on the agent's `PATH`
  (Unix): consent, the host's prefix, the SSH agent only for `ls-remote`,
  the work tree check, and nothing running without each sub-grant.
