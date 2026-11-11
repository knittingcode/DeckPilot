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
