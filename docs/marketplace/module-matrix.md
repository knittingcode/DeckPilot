# Built-in modules: classification matrix

Status: **audit**, the evidence for
[Moving built-in modules to plugins](module-migration.md). It changes no
code. Line numbers are from `origin/main` at `21fc2de2`; capability
statements also cover the open pull requests #231 (plugin variables,
cancellation) and #232 (storage, credentials as handles, settings), read
from their heads on 2026-11-15. Paths are relative to the repository root.

## What a plugin can do today

The baseline every row below is judged against: an ABI 1 wasm module in the
sandboxed `deckpilot-plugin-host`, with no WASI, no clock, no file and no
process access (`plugins/runtime/crates/abi/src/capability.rs:1-7`). Its only
way out is `host_call`, decided by the broker and executed by the agent.

| Operation | On main | With #231 / #232 | Limits that matter here |
| --- | --- | --- | --- |
| `http.fetch` | yes, exact `https` origins only; private networks blocked (`apps/agent/src/plugins/executor.rs:130-136`); auth refused (`executor.rs:181-186`) | #232: credential **handles** injected for a granted slot (bearer, basic, header, query) | 15 s for all hops (`executor.rs:29`), 512 KiB response (`executor.rs:32`), 32 KiB request body (`abi/src/limits.rs:52`), 1 MiB host-call reply (`limits.rs:26`), only 7 response headers (`executor.rs:34-42`: no `link`, `x-ratelimit-*`, `x-total`) |
| `notify` | yes | unchanged | title and body only; prefixed with the plugin name; 5 per minute (`broker/src/rate.rs:90`) |
| `log` | yes | unchanged | 50/s |
| `storage.*` | refused by the executor (`executor.rs:271-273`) | #232: get/set/delete/keys/usage within a quota | 256 KiB by default |
| `credentials.has`, `settings.get` | no | #232 | settings from `plugin_settings` in `plugins.db` |
| `variables.publish/clear` | no | #231: own `plugins.<publisher>.<name>.*` ids only | 10/s, 16 KiB |
| Manifest kinds `profile`, `variables` (read), `openUrl`, `background`, `device`, `network.privateNetwork` | declared in `crates/deckpilot-plugin-manifest/src/model.rs:204-258`, **not executed**: no ABI operation exists, and wildcard hosts and private network are dropped before the broker (`apps/agent/src/plugins/convert.rs:334`, test at `:388-423`) | unchanged | — |
| File system, processes, clipboard, machine resources, project context, raw device, workflows | never grantable (`plugins/runtime/crates/broker/src/permissions.rs:21-37`) | unchanged | — |

Parameters: plugins get every core parameter kind except `resource` and
`credential` (`crates/deckpilot-plugin-manifest/src/model.rs:405-460`
against `crates/deckpilot-core/src/action.rs:53-68`). A plugin action
receives only its parameters: no profile, project, run summary or machine
binding.

## Summary

Legend. **Sandbox today**: can run on main plus #231/#232 without new host
capabilities. **Verdict**: *core* (stays in the core, essential), *built-in*
(stays compiled in, optional, cannot be migrated securely), *default plugin*
(a bundled signed plugin), *plugin* (an optional plugin), *later* (plugin
once the named gaps close). Gaps (G…) are defined in
[module-migration.md § SDK gaps](module-migration.md#7-sdk-gaps).

*Verdicts updated 2026-11-20* by the owner's decision
([module-migration.md §11](module-migration.md#11-owner-decisions-of-2026-11-20)).
The rows for `projects`, `workspace`, `git`, `docker`, `http`, `github`,
`gitlab` and `jira`, and the new Workflows row, now carry the new verdicts.
The X entries are the plugin API 1.9 security reviews
([module-migration.md §7](module-migration.md#plugin-api-19-security-reviews-2026-11-20)).
Nothing in them is implemented yet. The evidence columns describe the code
as audited.

| Module | Actions / variables | Internal APIs | Essential | Sandbox today | New host capabilities | Config and DB | Complexity / risk | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `general` | 2 actions, 0 variables | profile and project (read), cancellation, progress | yes | no (no clock or sleep, no profile read) | G8 profile read, G10 sleep | none | low / high (every test and many workflows use `general.log`) | core |
| `projects` | 9 actions (3 deprecated), 3 variables + 3 `ALIASES` | profile, `ContextEffect`, faces, machine and project resources; agent and app call its crate directly | yes | no | project-context write (never grantable) | `wrapAround`; module schema 1 adopting core tables | high / high | **plugin** `deckpilot.projects` (2026-11-20) for actions, variables, views, cycling; the project data model stays a core primitive |
| `work-session` | 0 actions, 6 variables + 6 `ALIASES` | live session snapshot from the agent; page in the app | yes | no | work-session read/write (state is the agent's) | core schema 9 tables | high / high | core |
| `system` | 6 actions | OS launcher, `run_process`, `sysinfo`, file checks, `PATH`, machine resources | launcher actions: yes | no | G12 resource launch, G11 process inspect, `process.exec` (refused) | `allowCommands` | high / high | core (launchers) + built-in (`run-command`, `process-status`) |
| `notifications` | 2 actions | core notifier, run summary, workflow name | no | yes since P10 (G8 run summary, G9 severity, routing and delivery) | G9 notify severity, G8 run summary | 3 portable settings | low / medium (widely used) | default plugin (P10: `deckpilot.notifications`) |
| `workspace` | 3 actions | OS launcher, path checks and canonicalize, project resources; depends on the `system` crate | no | no | G12 resource launch | none | medium / medium | **plugin** `deckpilot.workspace` (2026-11-20; X1, X2, X4, X5) |
| `git` | 6 actions | `git` CLI with a curated environment, `is_dir`, launcher, 30 s button state | no | no | `process.exec` for `git` (refused), G7 button state | 3 settings | high / medium | **plugin** `deckpilot.git` (2026-11-20; X1, X2, X3a, X4) |
| `docker` | 7 actions | `docker` CLI with TLS, config and SSH-agent variables, path checks, 30 s button state | no | no | `process.exec` for `docker` (refused), G7 | 5 settings, `allowStateChanges` | high / medium | **plugin** `deckpilot.docker` (2026-11-20; X1, X2, X3b, X3c) |
| `http` | 2 actions | its own HTTP client and SSRF policy, which the plugin executor and three modules reuse; credentials by id | no (its transport is) | no | G2 user origins, G3 private network, G4 limits, G5 credential selection | 3 settings, any `credentials` row | high / high | **core** (2026-11-20, D1); transport already in `deckpilot-net` |
| `github` | 6 actions | HTTP client, credentials by id, launcher for `open` | no | partly (api.github.com reads work with #232; `open`, GHE, paging headers do not) | G1 openUrl, G2, G4, G6, G5 (credential import) | 4 settings, `github.token` | medium / low | plugin (first); built-in removed after GHE slot and WP-T1 (2026-11-20, D2) |
| `gitlab` | 6 actions | as github, `PRIVATE-TOKEN` header | no | partly (gitlab.com only) | G1, G2, G4, G6 (`x-total`), G5 | 4 settings, `gitlab.token` | medium / low | plugin; built-in removed after WP-T1 (2026-11-20, D2) |
| `jira` | 4 actions | as github; basic or bearer by deployment | no | no (no fixed host) | G1, G2 (always), G4, G5 | 5 settings, `jira.token` | medium / low | plugin; built-in removed after WP-T1 (2026-11-20, D2) |
| Workflows (not a module) | management UI only: list, editor, save/duplicate/delete (`apps/desktop/src-tauri/src/commands/workflows.rs`, `pages/workflows.tsx`, `workflow-builder/*`) | the core engine, execution IPC | engine yes, management no | n/a (no data ABI needed) | X8 `hostViews` | workflows stay in `profiles.document` | low / medium | **plugin** `deckpilot.workflows` (2026-11-20): manifest-only, activates the app-drawn `workflows` view; engine, `BindingTarget::Workflow` and run monitoring stay core |

Size of each crate and its exclusive dependencies:
[module-migration.md § Installer size](module-migration.md#8-installer-size).

## Where action type ids are stored

An action type id is stored in exactly one place: the `actionType` of an
action instance in a profile document (`ActionInstance.action_type`,
`crates/deckpilot-core/src/action.rs:652-666`; `Profile.actions`,
`crates/deckpilot-core/src/profile.rs:27`). Everything else references the
**instance** by `ActionInstanceId`:

- key, dial and gesture bindings of a layout: `ControlBinding { control,
  trigger, target }` with `BindingTarget::Action { action, overrides }`
  (`crates/deckpilot-core/src/layout.rs:100-160`; the trigger is the
  gesture),
- graph workflow nodes: `NodeKind::Action { action, overrides, mappings }`
  (`crates/deckpilot-core/src/graph/mod.rs:222-236`),
- step workflows (`crates/deckpilot-core/src/workflow.rs:209-227`).

Overrides and mappings are keyed by **parameter key** and node outputs are
read by **output key** (`steps.<node>.<output>`), so a replacement must keep
both. Exports carry the same profile documents, plus module settings keyed
by module id as text (`crates/deckpilot-portability/src/payload.rs:71-74`);
the import preview lists action types this build does not know
(`crates/deckpilot-portability/src/import.rs:317-326`). The only id rewrites
ever made are the schema 3 and 4 migrations (`crates/deckpilot-storage/src/legacy.rs:146-177`:
`deckpilot.core.log`, `deckpilot.project.cycle`, `deckpilot.project.select`
and `deckpilot.resource.resolve`; `crates/deckpilot-storage/src/project_actions.rs:1-16`
for parameters). Ids that other code names directly: `general.log` as the
probe action of the agent tests (`apps/agent/tests/agent.rs:133`), the
projects actions in the app's status and project views
(`apps/desktop/src/lib/status.ts`, `apps/desktop/src/modules/projects/`), and
the `http` frontend module that hosts the credentials settings
(`apps/desktop/src/modules/http/index.ts:6-11`).

Module data lives in the configuration database `deckpilot.db`: the
`modules` table (`id`, `enabled`, `config` JSON;
`crates/deckpilot-storage/src/legacy.rs:67-71`, read by
`crates/deckpilot-storage/src/sqlite.rs:1769-1792`, which skips ids it cannot
parse), the `credentials` table (`crates/deckpilot-storage/src/credentials.rs:14-19`)
and `module_schemas`. Plugin data lives in a separate SQLCipher database,
`plugins.db`, with its own schema (`crates/deckpilot-plugin-store/src/lib.rs:9-16`,
`db.rs:28-100`; #232 adds `plugin_storage`, `plugin_credentials`,
`plugin_settings` and `plugin_known` as schema 2). No built-in module
declares SQL: only `projects` declares a migration, and its SQL is empty.

## Action inventory

Every built-in action type id. *Params*: required in bold.

| Action type id | Params | Source |
| --- | --- | --- |
| `general.log` | **`message`** | `modules/general/src/lib.rs:13` |
| `general.wait` | **`seconds`** (1–300) | `modules/general/src/lib.rs:14` |
| `projects.cycle` | **`direction`** | `modules/projects/src/actions.rs:31` |
| `projects.select` | `project_id` | `modules/projects/src/actions.rs:32` |
| `projects.current` | — | `modules/projects/src/actions.rs:33` |
| `projects.repository` | `project_id` | `modules/projects/src/actions.rs:34` |
| `projects.documentation` | `project_id` | `modules/projects/src/actions.rs:35` |
| `projects.resolve-resource` | `project_id`, **`resource`** | `modules/projects/src/actions.rs:36` |
| `projects.next`, `projects.previous`, `projects.active` | — (deprecated) | `modules/projects/src/actions.rs:38-42` |
| `system.open-app` | **`application`**, `arguments` | `modules/system/src/actions.rs:17` |
| `system.open-url` | **`url`** | `modules/system/src/actions.rs:18` |
| `system.open-file` | **`location`**, **`path`**, `application` | `modules/system/src/actions.rs:19` |
| `system.open-directory` | **`location`**, `path` | `modules/system/src/actions.rs:20` |
| `system.run-command` | **`program`**, `arguments`, `workingDirectory`, **`timeoutSeconds`**, **`failOnNonZero`** | `modules/system/src/actions.rs:21` |
| `system.process-status` | **`name`** | `modules/system/src/actions.rs:22` |
| `notifications.show` | **`title`**, `body`, **`severity`**, **`critical`** | `modules/notifications/src/lib.rs:22` |
| `notifications.workflow-summary` | `title`, `details`, **`onlyOnFailure`**, **`critical`** | `modules/notifications/src/lib.rs:23` |
| `workspace.open-in-editor` | **`application`**, **`editor`**, `location`, `path`, `line`, `column` | `modules/workspace/src/actions.rs:19` |
| `workspace.open-urls` | **`urls`**, `browser` | `modules/workspace/src/actions.rs:20` |
| `workspace.open-project` | **`editor`**, **`editorRole`**, **`repositoryRole`**, **`docsRole`**, `urlRoles`, `browser` | `modules/workspace/src/actions.rs:21` |
| `git.status`, `git.branch` | `repository`, `project_id` | `modules/git/src/actions.rs:19-20` |
| `git.changes` | `repository`, `project_id`, **`limit`** | `modules/git/src/actions.rs:21` |
| `git.last-commit` | `repository`, `project_id` | `modules/git/src/actions.rs:22` |
| `git.remote-status` | `repository`, `project_id`, **`refresh`** | `modules/git/src/actions.rs:23` |
| `git.open-repository` | `repository`, `project_id`, `application` | `modules/git/src/actions.rs:24` |
| `docker.status` | `context` | `modules/docker/src/actions.rs:19` |
| `docker.containers` | **`all`**, `name`, `context` | `modules/docker/src/actions.rs:20` |
| `docker.compose-status` | `compose`, `project_id`, `projectName`, `context` | `modules/docker/src/actions.rs:21` |
| `docker.compose-up`, `docker.compose-restart` | compose set, `services` | `modules/docker/src/actions.rs:22,24` |
| `docker.compose-down` | compose set, **`removeOrphans`** | `modules/docker/src/actions.rs:23` |
| `docker.logs` | `container`, `service`, **`lines`**, `since`, compose set | `modules/docker/src/actions.rs:25` |
| `http.request` | **`method`**, **`url`**, `query`, `headers`, `body`, **`bodyType`**, `credential`, **`authScheme`**, `authName`, **`timeoutSeconds`**, **`followRedirects`**, **`failOnHttpError`**, `extract` | `modules/http/src/actions.rs:15` |
| `http.health-check` | **`url`**, **`expectedStatus`**, `contains`, **`timeoutSeconds`**, **`failWhenUnhealthy`**, `credential`, **`authScheme`**, `authName` | `modules/http/src/actions.rs:16` |
| `github.current-user` | `credential` | `modules/github/src/actions.rs:18` |
| `github.repository` | **`repository`**, `credential` | `modules/github/src/actions.rs:19` |
| `github.pull-requests`, `github.review-requests` | `repository`, **`filter`** (pull requests only), **`limit`**, `credential` | `modules/github/src/actions.rs:20-21` |
| `github.checks` | **`repository`**, `ref`, `pullRequest`, `credential` | `modules/github/src/actions.rs:22` |
| `github.open` | `url`, `repository`, `pullRequest` | `modules/github/src/actions.rs:23` |
| `gitlab.current-user` | `credential` | `modules/gitlab/src/actions.rs:18` |
| `gitlab.project` | **`project`**, `credential` | `modules/gitlab/src/actions.rs:19` |
| `gitlab.merge-requests`, `gitlab.review-requests` | `project`, **`filter`** (merge requests only), **`limit`**, `credential` | `modules/gitlab/src/actions.rs:20-21` |
| `gitlab.pipeline` | **`project`**, `ref`, `mergeRequest`, `credential` | `modules/gitlab/src/actions.rs:22` |
| `gitlab.open` | `url`, `project`, `mergeRequest` | `modules/gitlab/src/actions.rs:23` |
| `jira.myself` | `credential` | `modules/jira/src/actions.rs:19` |
| `jira.issue` | **`issue`**, `credential` | `modules/jira/src/actions.rs:20` |
| `jira.issues` | **`source`**, `filterId`, `project`, **`includeDone`**, **`limit`**, `credential` | `modules/jira/src/actions.rs:21` |
| `jira.open` | `url`, `issue` | `modules/jira/src/actions.rs:22` |

Variables (only two modules declare any; namespaces are fixed in
`crates/deckpilot-core/src/variables/registry.rs:53`, aliases in
`registry.rs:309-350`, all deprecated since 0.6.0):

| Variable | Type | Legacy alias | Owner |
| --- | --- | --- | --- |
| `projects.current.id`, `.key`, `.name` | ProjectId, String, String | `system.project.current.id`, `.key`, `.name` | `projects` (`modules/projects/src/variables.rs:14-45`) |
| `workSession.id`, `.state`, `.startedAt`, `.elapsedSeconds`, `.projectId`, `.canFinish` | String, String, String, Integer, ProjectId, Boolean | `system.work.session.*` | `work-session` (`modules/work-session/src/variables.rs:20-57`) |

## Modules

Common to all twelve: registered in `crates/deckpilot-modules/src/lib.rs:28-67`;
no module declares windows (`ModuleManifest.windows` is empty) or SQL
migrations; none uses the clipboard, key synthesis, media keys, the device
runtime or the OS keychain (secrets are in the encrypted `credentials`
table, not the keychain).

### `general`

1. **Actions.** `general.log` (`modules/general/src/lib.rs:13`, definition
   `:94-125`): `message` (multiline text, at most 1,000) with `{profile}`,
   `{project.key}`, `{project.name}` filled in (`:143-160`); fails with
   `no-active-project` when the message needs a project. `general.wait`
   (`:14`, `:173-213`): `seconds` 1–300, output `waitedMs`, waits in 1 s
   steps on the cancellation token (`:231-256`): the reference cancellable
   node. No variables.
2. **Internal APIs.** `context.profile()`, `context.project()` (gated by
   `ProfileRead`, `crates/deckpilot-core/src/execution.rs:329-338`),
   `context.wait()`, `report_progress`. No OS calls.
3. **Capabilities.** `ProfileRead` (`lib.rs:56`). As a plugin: `profile`
   read and a cancellable sleep, neither of which exists.
4. **Essential.** Yes. `wait` is the workflow engine's delay primitive;
   `log` is the diagnostic action and the probe of the agent's tests.
5. **Sandbox today.** No: a guest has no clock and cannot sleep without
   spinning on fuel, and cannot read the profile.
6. **New capabilities.** G10 (sleep), G8 (profile read). Both are cheap, but
   moving 113 KB of code out would add an IPC hop to every log line.
7. **Config and DB.** None. No variables.
8. **Referenced ids.** `general.log` (also the target of the legacy rewrite
   of `deckpilot.core.log`, `legacy.rs:146`), `general.wait`.
9. **Complexity and risk.** Low code, high blast radius. **Core.**

### `projects`

1. **Actions.** `projects.cycle`, `.select`, `.current`, `.repository`,
   `.documentation`, `.resolve-resource`, deprecated `.next`, `.previous`,
   `.active` (`modules/projects/src/actions.rs:31-53`). Variables
   `projects.current.id|key|name` (`src/variables.rs:14-45`).
2. **Internal APIs.** `ContextEffect::SelectProject` and `CycleProject`
   (`src/actions.rs:247,487`; `crates/deckpilot-core/src/execution.rs:385-402`),
   `decorate_face` (`src/actions.rs:200-211`), `ProjectSwitcher`,
   `resolve_project_resource` (`src/resources.rs:79`). The agent calls
   `switching` directly (`apps/agent/src/service.rs:1126-1127`), the app its
   `ProjectCatalog` (`apps/desktop/src-tauri/src/modules/projects.rs:19`).
3. **Capabilities.** `ProfileRead`, `ProjectContext`, `MachineResources`
   (`src/lib.rs:58-62`).
4. **Essential.** Yes: the core profile model, label templates, step
   conditions, IPC and three storage tables depend on projects
   ([modules.md](../modules.md), "Remaining dependencies of the core on
   projects").
5. **Sandbox today.** No.
6. **New capabilities.** Project-context write, never grantable
   (`permissions.rs:31`).
7. **Config and DB.** `wrapAround` (portable, `src/lib.rs:63-78`); module
   schema 1 adopting `projects`, `default_projects`, `active_projects`
   (`src/lib.rs:82-87`; tables from `crates/deckpilot-storage/src/legacy.rs:46-62`);
   namespace `projects` and three `ALIASES`.
8. **Referenced ids.** All nine; schema 3 and 4 rewrote older ids into them.
9. **Complexity and risk.** High and high. **Core.**

*Verdict (2026-11-20):* **plugin** `deckpilot.projects` for every
user-facing part: the nine actions (the three deprecated ones as same-name
deprecated actions), `projects.current.*` as host-published variables (X7),
`wrapAround`, the `projects` host view, the status item and the dashboard
card (X8), and select and cycle through `projectContext` (X6, high tier).
The data model (`Project`, `ActiveProjectContext`, the three tables, the
FKs, portability, IPC) stays a core primitive. `ProjectCatalog` moves into
the core when the crate is removed (WP-C6). The resource-binding editor
moves to core Settings › Resources.

### `work-session`

1. **Actions.** None, by design (decision D11, `modules/work-session/src/lib.rs:5-9,83-85`).
   Variables `workSession.*` (six, `src/variables.rs:20-57`), each with a
   `system.work.session.*` alias (`crates/deckpilot-core/src/variables/registry.rs:327-348`).
   One page (`src/lib.rs:50-55`).
2. **Internal APIs.** `environment.live.work_session()`, a snapshot the
   agent supplies, and `now_ms()` (`src/variables.rs:60-109`). The agent
   calls the module's functions directly (`apps/agent/src/service/variables.rs:73,107`);
   the app refuses to disable it during a session
   (`apps/desktop/src-tauri/src/commands/modules.rs:57-64`).
3. **Capabilities.** None declared; the state and its writers are the
   agent's.
4. **Essential.** Yes: core schema 9 tables, gestures and the time
   accounting.
5. **Sandbox today.** No (no variable source in plugins; values are pushed
   with `variables.publish`, which cannot reproduce on-demand resolution).
6. **New capabilities.** Work-session read and write; write is not
   offerable.
7. **Config and DB.** Core schema 9 (`work_session_*`), not module
   migrations; namespace `workSession` and six `ALIASES`.
8. **Referenced ids.** No actions; variable references in workflows.
9. **Complexity and risk.** High and high. **Core.**

### `system`

1. **Actions.** `system.open-app`, `.open-url` (http, https, mailto only,
   `src/support.rs:209-223`), `.open-file`, `.open-directory`,
   `.run-command`, `.process-status` (`modules/system/src/actions.rs:17-22`).
   `run-command` outputs `exitCode`, `stdout` and `stderr` (sensitive),
   `outputTruncated`, `durationMs`. No variables.
2. **Internal APIs.** `deckpilot_core::launch` (`SystemLauncher` spawns
   `open`, `xdg-open`, `rundll32`; `crates/deckpilot-core/src/launch.rs:83-118,214-240`);
   `std::process::Command` with `process::minimal_environment` and
   `context.run_process` (`src/actions.rs:539-555`); `sysinfo` process
   names and PIDs (`:657-670`); `is_file`, `is_dir`, `canonicalize`, a
   `PATH`/`PATHEXT` search and the home folder (`src/support.rs:89-206`,
   `src/actions.rs:533`); machine resource bindings.
3. **Capabilities.** `MachineResources`, `OpenExternal`, `ProcessExecution`,
   `ProcessInspect` (`src/lib.rs:86-91`).
4. **Essential.** The four launcher actions are what a Stream Deck app is
   for, and the launcher is already core code. `run-command` and
   `process-status` are optional. See
   [the core boundary](module-migration.md#3-the-core-boundary).
5. **Sandbox today.** No action can run.
6. **New capabilities.** G12 (launch a bound resource without seeing its
   path) for the launchers, G11 (process inspection by name) for
   `process-status`, `process.exec` for `run-command`, which is never
   offered: it is arbitrary code execution, and a sandbox around the
   caller adds nothing.
7. **Config and DB.** `allowCommands` (boolean, default false,
   machine-specific, never exported; `src/lib.rs:92-112`). No credentials.
   `modules/workspace` reuses its `support` code (`modules/workspace/src/actions.rs:13-14`).
8. **Referenced ids.** All six.
9. **Complexity and risk.** High and high. **Core** (launchers) and
   **built-in** (`run-command`, `process-status`).

### `notifications`

1. **Actions.** `notifications.show` (`title`, `body`, `severity`
   info/success/warning/error, `critical`) and
   `notifications.workflow-summary` (`title`, `details`, `onlyOnFailure`,
   `critical`), both with outputs `shown`, `desktop`, `statusBar`, `reason`
   (`modules/notifications/src/lib.rs:22-23,198-306`). No variables.
2. **Internal APIs.** `context.notify()` to the notifier the agent injects
   (`crates/deckpilot-core/src/execution.rs:196-205`), `context.run_summary()`
   and the profile, for the workflow's name (`lib.rs:357-360`). No OS calls.
3. **Capabilities.** `ProfileRead`, `Notifications` (`lib.rs:106`).
4. **Essential.** No. The notifier and its routing to desktop and status
   bar stay in the core; the module is a thin front.
5. **Sandbox today.** Partly. `notify` has no severity, no critical flag
   and no routing; it is prefixed with the plugin's name and limited to 5
   per minute; a plugin cannot read the run summary.
6. **New capabilities.** G9 (severity and critical in `notify`, routing by
   the agent), G8 (a run-scoped summary of the invocation's own workflow).
   Both are low risk.
7. **Config and DB.** `desktop`, `statusBar`, `minimumSeverity`, all
   portable (`lib.rs:107-143`), read when handlers are built.
8. **Referenced ids.** Both, frequently as the last node of a workflow.
9. **Complexity and risk.** Low and medium. **Default plugin** after G8 and
   G9.

*Status (P10):* `plugins/first-party/notifications`
(`deckpilot.notifications`) reproduces both actions with G8 (`run.context`)
and G9 (severity; the routing by the plugin's settings `desktop`,
`statusBar` and `minimumSeverity`, which the agent reads; the delivery
reply); bundled with class `default` and routed through the alias block
`notifications.*`. `critical` stays the plugin's decision on the delivery.
No manifest-declared higher rate: 5 per minute, as for every plugin.
Parity and the remaining differences:
`plugins/first-party/notifications/README.md`.

### `workspace`

1. **Actions.** `workspace.open-in-editor`, `.open-urls` (at most 10),
   `.open-project` (`modules/workspace/src/actions.rs:19-21,141-283`). No
   variables.
2. **Internal APIs.** `launch::plan` and `SystemLauncher` (`src/lib.rs:17,53`,
   `src/editor.rs:123-195`), `system::support::{existing, existing_inside,
   join_inside}` (`is_dir`, `exists`, `canonicalize`; `modules/system/src/support.rs:145-200`),
   project resources (`src/actions.rs:495-507`).
3. **Capabilities.** `ProfileRead`, `MachineResources`, `OpenExternal`
   (`src/lib.rs:68-72`).
4. **Essential.** No.
5. **Sandbox today.** No.
6. **New capabilities.** G12. The plan building (`src/editor.rs`) is pure
   and would port; the path checks must stay in the host.
7. **Config and DB.** None. Depends on the `deckpilot-module-system` crate.
8. **Referenced ids.** All three.
9. **Complexity and risk.** Medium and medium. **Built-in**, later with
   G12.

*Verdict (2026-11-20):* **plugin** `deckpilot.workspace`, with an optional
dependency on `deckpilot.projects` (`open-project` requires it). It uses
resource handles (X1, X2), `resource.open` (X4, high) and
`url.openParam` (X5, medium). The path helpers move from
`modules/system/src/support.rs` into the core (WP-A3).

### `git`

1. **Actions.** `git.status`, `.branch`, `.changes`, `.last-commit`,
   `.remote-status`, `.open-repository` (`modules/git/src/actions.rs:19-24`),
   each with optional `repository` (directory resource) and `project_id`
   (`:84-102`). No variables.
2. **Internal APIs.** Runs the `git` executable (`src/repo.rs:122`) with
   `-C <repo> --no-pager --no-optional-locks -c core.fsmonitor=false`, a
   cleared environment plus `SSH_AUTH_SOCK`, `GIT_EXEC_PATH`,
   `XDG_CONFIG_HOME` and `GIT_TERMINAL_PROMPT=0` (`:109-139`) through
   `context.run_process`; `ls-remote` reaches the network with the user's
   SSH agent and git credential helpers; `is_dir`; the launcher; button
   state refreshed every 30 s (`src/actions.rs:322-349`, `src/state.rs:15-38`).
3. **Capabilities.** `ProfileRead`, `MachineResources`, `ProcessExecution`,
   `OpenExternal` (`src/lib.rs:93-98`).
4. **Essential.** No.
5. **Sandbox today.** No.
6. **New capabilities.** Running `git`, which cannot be offered securely
   (see [G13](module-migration.md#g13-processexec-not-offered)); G7 for the
   key state. A brokered `git.*` that ran fixed subcommands host-side would
   leave every security-relevant line in the host and the plugin as a
   formatter.
7. **Config and DB.** `gitExecutable` (machine-specific), `repositoryRole`
   and `networkTimeoutSeconds` (portable) (`src/lib.rs:99-149`). No
   credentials, no SQL.
8. **Referenced ids.** All six.
9. **Complexity and risk.** High and medium. **Built-in.** It uses the git
   CLI, not `git2`.

*Verdict (2026-11-20):* **plugin** `deckpilot.git`, with an optional
dependency on `deckpilot.projects`. It runs git through `tool.run` (X3a:
`tools:git` and, for `remote-status` with refresh, `tools:git:network`,
both high tier). `gitExecutable` becomes a host-held setting of the grant.
`open-repository` uses `resource.open` (X4). The authority stays in the
host. The gain is removability, not isolation of the process run.

### `docker`

1. **Actions.** `docker.status`, `.containers`, `.compose-status`,
   `.compose-up`, `.compose-down`, `.compose-restart`, `.logs`
   (`modules/docker/src/actions.rs:19-25`); the three changing actions are
   refused unless `allowStateChanges` (`:720-728`). No variables.
2. **Internal APIs.** Runs the `docker` executable (`src/actions.rs:569`)
   with `DOCKER_HOST`, `DOCKER_CONTEXT`, `DOCKER_CONFIG`, `DOCKER_CERT_PATH`,
   `DOCKER_TLS_VERIFY`, `SSH_AUTH_SOCK` passed through (`:41-48`); compose
   file checks (`src/cli.rs:105-121`); button state every 30 s
   (`src/actions.rs:449-492`). It uses the CLI, not the socket or `bollard`.
3. **Capabilities.** `ProfileRead`, `MachineResources`, `ProcessExecution`
   (`src/lib.rs:141-145`).
4. **Essential.** No.
5. **Sandbox today.** No.
6. **New capabilities.** Docker access through the CLI or the socket.
   Access to the daemon is root-equivalent on the host (a compose file can
   mount `/` or run privileged containers), so a brokered `docker.*`
   capability would be the most dangerous grant in the system. Not offered.
7. **Config and DB.** `dockerExecutable`, `context`, `allowStateChanges`
   (machine-specific), `composeRole`, `timeoutSeconds` (portable)
   (`src/lib.rs:146-207`).
8. **Referenced ids.** All seven.
9. **Complexity and risk.** High and medium. **Built-in.**

*Verdict (2026-11-20):* **plugin** `deckpilot.docker`, with an optional
dependency on `deckpilot.projects`. Reads go through `tools:docker` (X3b,
high). `compose-up`, `compose-down` and `compose-restart` go through
`tools:docker:modify` (X3c, high, off by default), which replaces
`allowStateChanges`. `dockerExecutable` and `context` become host-held
settings of the grant.

### `http`

1. **Actions.** `http.request` and `http.health-check`
   (`modules/http/src/actions.rs:15-16,146-320`). Outputs of `request`:
   `status`, `ok`, `contentType`, `headers`, `body` and `value` (sensitive),
   `size`, `truncated`, `url`, `redirects`, `durationMs`. No variables.
2. **Internal APIs.** Its own client (`src/client.rs:62-305`: a
   single-thread runtime per request, DNS resolved and checked against
   `NetworkPolicy`, connections pinned to checked addresses, manual
   redirects, at most 5, auth dropped across origins) and SSRF policy
   (`src/policy.rs:35-105`). Credentials by id through
   `ExecutionContext::credential` (`crates/deckpilot-core/src/execution.rs:225`).
   **The plugin executor uses this client and policy**
   (`apps/agent/src/plugins/executor.rs:110-111`), as do github, gitlab and
   jira.
3. **Capabilities.** `Network`, `Credentials` (`src/lib.rs:127`).
4. **Essential.** The transport is (the broker depends on it); the actions
   are not.
5. **Sandbox today.** No. The URL is free text and may be a template
   (`src/actions.rs:128-139,378`), so no origin is known at install; private
   and loopback hosts are an opt-in feature (`src/lib.rs:36-38,128-167`);
   any stored credential can be named per action; 10 MiB bodies, 300 s
   timeouts, 64 KiB request bodies and all response headers exceed the
   broker's limits.
6. **New capabilities.** G2 (user-entered origins, per action), G3 (private
   network), G4 (limits), G5 (choosing among user-created credential slots),
   G6 (response headers). Together these reproduce the built-in's authority
   with extra moving parts; the security gain is small.
7. **Config and DB.** `allowPrivateNetworks`, `privateHosts`
   (machine-specific), `maxResponseKb` (portable). Any row of the
   `credentials` table; the credential editor lives in the app's `http`
   frontend module.
8. **Referenced ids.** Both, often with a `credential` parameter.
9. **Complexity and risk.** High and high. Transport to a **core** crate
   first; actions **built-in**, later.

*Verdict (2026-11-20):* **core** (D1). The actions stay, and the transport
is already `crates/deckpilot-net`.

### `github`

1. **Actions.** `github.current-user`, `.repository`, `.pull-requests`
   (`filter` all, author-me, review-requested), `.review-requests`,
   `.checks` (paged through `Link`, at most 5 pages), `.open`
   (`modules/github/src/actions.rs:18-23`); every action but `open` takes an
   optional `credential` that overrides the setting (`:89-97`).
2. **Internal APIs.** `deckpilot_module_http::client::send`, credentials by
   id, `Authorization: Bearer` with fixed `Accept` and API version headers
   (`src/api.rs:405-417`), rate limits from `Retry-After` and
   `x-ratelimit-*` (`:213-232`), the launcher for `open` (https pages on the
   web host only, `src/api.rs:154-178`).
3. **Capabilities.** `Network`, `Credentials`, `OpenExternal` (`src/lib.rs:141-145`).
   As a plugin: `network` for `https://api.github.com` (GET), a `token`
   credential slot (bearer) for that origin, `openUrl` for
   `https://github.com`.
4. **Essential.** No.
5. **Sandbox today.** Partly. With #232 the five read actions work against
   api.github.com; `open` needs G1; `Link` and `x-ratelimit-*` are dropped
   (G6); a 4 MiB response cap (`src/api.rs:29`) exceeds 512 KiB, so pages
   must be smaller (G4 or a plugin-side change); GitHub Enterprise and
   `allowPrivateHost` need G2 and G3.
6. **New capabilities.** G1, G6, G2 (GHE only), G3 (private GHE only), G5
   (the one-time credential import).
7. **Config and DB.** `apiUrl` (machine-specific, default
   `https://api.github.com`), `credential` (default `github.token`,
   portable), `timeoutSeconds`, `allowPrivateHost` (`src/lib.rs:146-198`).
   One `credentials` row, usually `github.token`.
8. **Referenced ids.** All six.
9. **Complexity and risk.** Medium and low (opt-in, niche, read-only).
   **Plugin, first.**

*Status (P8):* `plugins/first-party/github` (`deckpilot.github`) reproduces
the six actions with G1 and G6, pages of 50 and the credential slot
`token`; bundled with class `migration`, not routed yet. Parity and the
remaining differences: `plugins/first-party/github/README.md`.

*Verdict (2026-11-20):* plugin only. The built-in is removed (WP-C5) once
`deckpilot.github` has a user-origin slot for GitHub Enterprise (WP-B0) and
WP-T1 passes. The §5.7 two-release rule is waived (D2).

### `gitlab`

1. **Actions.** `gitlab.current-user`, `.project`, `.merge-requests`,
   `.review-requests`, `.pipeline`, `.open` (`modules/gitlab/src/actions.rs:18-23`),
   with the same optional `credential` override.
2. **Internal APIs.** As github; `PRIVATE-TOKEN` header (`src/api.rs:35,394`);
   counts from `X-Total` or up to 5 pages (`src/actions.rs:595-605`);
   rate limit from `Retry-After` and `RateLimit-Reset`.
3. **Capabilities.** `Network`, `Credentials`, `OpenExternal`. As a plugin:
   `https://gitlab.com` (GET), a `token` slot with the `header` scheme named
   `PRIVATE-TOKEN`, `openUrl` for `https://gitlab.com`.
4. **Essential.** No.
5. **Sandbox today.** Partly, gitlab.com only; `X-Total` is dropped (G6);
   self-managed instances (often under a sub-path, `src/api.rs:137-145`)
   need G2.
6. **New capabilities.** G1, G2, G4, G6, G5.
7. **Config and DB.** As github, default `https://gitlab.com/api/v4` and
   `gitlab.token` (`src/lib.rs:146-209`).
8. **Referenced ids.** All six.
9. **Complexity and risk.** Medium and low. **Plugin**, after G2 and G6.

*Status (P11):* `plugins/first-party/gitlab` (`deckpilot.gitlab`)
reproduces the six actions for GitLab.com and for a self-managed GitLab
the user enters (G2, with its path and, by its own consent, a private
network), with the credential slot `token` (`PRIVATE-TOKEN`) and pages of
50; bundled with class `migration` and routed by the `gitlab.*` alias
block once a computer's migration is cut over (the server the user had
entered is asked for). Parity and the remaining differences:
`plugins/first-party/gitlab/README.md`.

*Verdict (2026-11-20):* plugin only. The built-in is removed (WP-C5) after
WP-T1 (D2).

### `jira`

1. **Actions.** `jira.myself`, `.issue`, `.issues` (fixed JQL templates only,
   `src/actions.rs:453-466`), `.open` (`src/actions.rs:19-22`).
2. **Internal APIs.** As github; Cloud uses basic auth with `email:token`,
   Data Center bearer (`src/api.rs:241-255,335-338`); scrubs the token and
   its base64 form from results.
3. **Capabilities.** `Network`, `Credentials`, `OpenExternal`
   (`src/lib.rs:159-163`). As a plugin: the user's site only, one slot with
   both `basic` and `bearer` schemes.
4. **Essential.** No.
5. **Sandbox today.** No: `siteUrl` has no default (`src/lib.rs:164-230`);
   `*.atlassian.net` would need a wildcard, which the broker never accepts.
6. **New capabilities.** G2 (always), G1, G4, G5; G3 for private Data
   Center.
7. **Config and DB.** `siteUrl`, `deployment` (machine-specific),
   `credential` (default `jira.token`), `timeoutSeconds`, `allowPrivateHost`.
8. **Referenced ids.** All four.
9. **Complexity and risk.** Medium and low. **Plugin**, after G2.

*Status (P11):* `plugins/first-party/jira` (`deckpilot.jira`) reproduces
the four actions for the Jira Cloud site or Data Center the user enters
(G2; no host of its own), with the credential slot `credential` (Basic or
Bearer by `deployment`); bundled with class `migration` and routed by the
`jira.*` alias block once a computer's migration is cut over (the site is
asked for). Parity and the remaining differences:
`plugins/first-party/jira/README.md`.

*Verdict (2026-11-20):* plugin only. The built-in is removed (WP-C5) after
WP-T1 (D2).

## Audit of 2026-11-16: where the migration stands

Taken on `origin/main` at `dc1a8a2e` and `knittingcode/deckpilot-plugins`
at `829a885`, before GitLab and Jira (P11). It records what exists; the
decisions stay in [module-migration.md](module-migration.md).

### What is done

| Module | Plugin | Class | Routed by | PRs |
| --- | --- | --- | --- | --- |
| `github` | `deckpilot.github` 0.1.0 | `migration` | `github.*` → `deckpilot.github.*` (6 actions) | #253, #260 |
| `notifications` | `deckpilot.notifications` | `default` | `notifications.*` → `deckpilot.notifications.*` (2 actions) | #265 |
| `gitlab` | `deckpilot.gitlab` 0.1.0 | `migration` | `gitlab.*` → `deckpilot.gitlab.*` (6 actions) | P11, after this audit |
| `jira` | `deckpilot.jira` 0.1.0 | `migration` | `jira.*` → `deckpilot.jira.*` (4 actions) | P11, after this audit |

Released builds still have plugins off (they lack the `plugins` cargo
feature, which turns plugins on, and `VITE_DECKPILOT_PLUGINS=1`), so every
built-in is what runs for 0.6.0-alpha.3 users. No built-in was removed.

### What a plugin can use now

| Area | State |
| --- | --- |
| SDK | One guest SDK, `plugins/runtime/guest/plugin-sdk` (ABI 1, `wasm32-unknown-unknown`); documented in [sdk/](sdk/README.md). No second API |
| Capabilities | `http.fetch` (exact `https` origins, credential handles: bearer, basic, header, query), `url.open` (G1), `notify` with severity, routing and delivery (G9), `run.context` (G8), `storage.*`, `settings.get`, `credentials.has`, `variables.publish/clear`, `log` |
| User-entered origins | G2 (#266): `network.userOrigins`, per-origin consent, bound credentials, private network as a separate consent. Pages of such a server: `openUrl.userOrigins` (this change) |
| Limits | 15 s per fetch, 512 KiB per response, 32 KiB per request body, at most 32 response headers from an allowlist (4 KiB each), 60 fetches per minute, 256 host calls per invocation, 16 KiB per output text, 192 KiB of output; pages: 1 per 2 s, 20 per 10 minutes, 3 per invocation |
| Not offered | G3 as a general grant (only through a consented user origin), G4 larger limits, G5 `credentialSlot` parameters, G7 button state and timers, G10 sleep, G11, G12; never: process execution (G13), file system (G14), project-context and work-session writes (G15, G16) |
| Package format | `<id>-<version>.dpplugin`, signed; verified by `crates/deckpilot-plugin-package` and the registry client ([artifact-contract.md](artifact-contract.md)). Bundled plugins are unpacked app components trusted by sha256 pins compiled into the agent, plus the macOS code seal (owner's decision of 2026-11-15) |
| Registry | `knittingcode/deckpilot-plugins`: schemas (`plugin-manifest`, `registry-index`, `registry-keyset`, `registry-plugin`, `registry-timestamp`, and the inputs `build-recipe`, `plugin-info`), one example entry (`acme.hello-weather`), no production keys. A plugin is submitted as `plugins/<path>/source.json` (a repository and a pinned commit) with `build.json`; the registry's CI builds it |
| Aliases | `ACTION_ALIASES` (`crates/deckpilot-core/src/alias.rs`): the two blocks above. Stored profiles keep the legacy ids |

### Where first-party sources live

The approved arrangement ("sources in both repositories") is one source of
truth and a pinned reference, not two copies: the source of every
`deckpilot.*` plugin is in this repository under `plugins/first-party/`,
built into the bundle by `plugins/first-party/build.sh`; the registry
receives a `source.json` that names this repository at a pinned commit, and
its CI builds the independently installable package from exactly that
commit ([`plugins/first-party/README.md`](../../plugins/first-party/README.md),
module-migration.md B5). Nothing is copied by hand, so the two cannot
drift; what must be checked is that the registry's build of the pinned
commit and the bundle's build give the same `plugin.wasm` (both are
reproducible on one platform with the pinned toolchain).

Not possible yet:

- **B6.** Before B6 the agent refused a `deckpilot.*` package from any
  registry: not in the store (`InstallSource::allows`,
  `crates/deckpilot-plugin-store/src/store.rs`, accepts a registry
  source for any id) but in `verify_package`, whose manifest check
  (`Manifest::validate`) refuses the reserved publisher; only bundled
  files used `Manifest::validate_first_party`. Now the agent supports it:
  `verify_package` applies the first-party rules when the registry client
  marks the pins (`ExpectedPackage::first_party`), which it does only when
  the registry's root-signed key set authorizes the `deckpilot` namespace
  and its index lists `deckpilot` as a verified publisher; the bundled
  ids (`deckpilot.github`, `deckpilot.gitlab`, `deckpilot.jira`,
  `deckpilot.notifications`) are never taken from a registry
  ([agent-integration.md](agent-integration.md), `PUBLISHING.md`). What
  remains is on the registry side: the first production key set with the
  authorization (B1's ceremony), the verified `deckpilot` publisher record,
  and the submission checks of `deckpilot-plugins` (#6), which still
  refuse the publisher.
- **B1.** No production registry keys exist (U-10, U-12). Packages can be
  built and verified with disposable test keys in tests and CI only.

### The order asked for, against the approved scope

*Superseded 2026-11-20.* The table below records the 2026-11-15 scope. The
owner's decision of 2026-11-20
([module-migration.md §11](module-migration.md#11-owner-decisions-of-2026-11-20))
chose option (b) for `git`, `docker` and `workspace`, and extracted the
user-facing part of `projects`. `http`, `work-session`, `general` and the
`system` launchers stay core. `system.run-command` and
`system.process-status` stay built-in.

The mission's order is GitHub, GitLab, Jira, HTTP, Git, Docker, Projects,
Workspace, Work Session, with System and General audited separately. The
approved scope (module-migration.md §1, §3, §7; owner's decision of
2026-11-15, "as proposed") agrees for the first three and disagrees for
the rest:

| Module | Approved verdict | Why it cannot simply follow | Options |
| --- | --- | --- | --- |
| `gitlab`, `jira` | plugin | nothing, since G2 and `openUrl.userOrigins` | proceed (P11) |
| `http` | actions built-in; transport already in the core (`deckpilot-net`) | the user types a URL per action, also `localhost` and plain `http`; any stored credential per action; 10 MiB and 300 s. A plugin would need "any origin the user enters, per action" and credential selection, which makes the broker a pass-through with the built-in's authority (§6) | (a) keep built-in (approved); (b) design per-action origin consent (G2's deferred variant), G4 limits and G5 slots first, each its own decision |
| `git`, `docker` | built-in | their job is running `git` and `docker`: process execution with the user's SSH agent and, for Docker, root-equivalent access (G13, refused) | (a) keep built-in (approved); (b) a design note for a narrow host-side `git.status`-style capability, which keeps the risky code in the host and isolates only formatting |
| `projects`, `work-session` | core | they own core state: `ContextEffect`s, the profile model, three core tables, the agent's session state machine (G15, G16: never grantable) | keep in the core (approved); nothing to migrate |
| `workspace` | built-in, later with G12 | launches programs on bound machine resources: G12 is high tier and not in API 1 | (a) keep built-in (approved); (b) design G12 as its own decision |
| `system`, `general` | core (launchers, `wait`, `log`); `run-command` and `process-status` built-in | process execution and the engine's own nodes | keep (approved) |

So after GitLab and Jira, nothing further can move without a new host
capability that the approved scope either refuses (process execution) or
has not designed (per-action origins, resource launch). None is invented
here. The essential core the mission lists (devices, agent lifecycle,
profiles, layouts, the workflow engine, the plugin runtime and broker, the
Marketplace client, encrypted storage, updates) is untouched by every
migration so far.
