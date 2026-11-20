# Documentation

An index of DeckPilot's documentation. Documents marked *design record*
were written before the implementation and keep the audit and design that
led to it; where the code differs, the code and the tests win.

## Using DeckPilot

- [Devices](devices.md): supported Stream Deck models, drivers and how far
  each model has been verified on each platform.
- [Device controls](device-controls.md): which controls and inputs an
  action can be bound to, and what happens to a binding that cannot run.
  *Design record, sections 1–3 implemented.*
- [Security](security.md): what is protected, the encrypted configuration,
  key handling, locking, exports and the threat model.
- [Updates](updates.md): update channels, signed updates, the automatic
  check and how app and agent are updated together.
- [Feature modules](modules.md): how DeckPilot is built from modules and
  what enabling or disabling one does.
- [Variables](variables.md): the unified variable registry, references and
  the Variables page. *Design record, implemented.*
- [Workflow graphs](workflow-graphs.md): the workflow model, nodes and how
  the agent runs a graph.
- [Key gestures, stateful keys and work sessions](gestures-and-work-session.md):
  click, double click and long press, stateful keys and the Work Session.
  *Design record, implemented in 0.6.0.*

## Architecture

- [Architecture](architecture.md): the agent and the configuration app,
  crates and how they fit together.
- [Background agent](background-agent.md): the agent process, its IPC
  protocol and requests, and hardware support.
- [Data model](data-model.md): entities, identifiers, the database schema
  and its migrations.
- [Key text](key-text.md): how key titles are wrapped, fitted and scrolled
  (text modes), the layout report, the agent's marquee scheduler and its
  cost.
- [Action architecture](action-architecture.md): action types, instances
  and shared variables. *Design record, implemented in 0.6.0.*
- [Integration modules](integration-modules.md): the System, Workspace,
  Git, Docker, HTTP, GitHub, GitLab, Jira and Notifications modules.
- [Compatibility](compatibility.md): protocol negotiation and the tested
  matrix of app and agent versions.
- [Built-in actions and feature parity](feature-parity.md): what exists
  for folders, pages, profile switching, key gestures, multi actions,
  system and device actions, what is missing, and the designs for it.
  *Audit and design record.*

## Plugin marketplace (experimental)

- [Plugins in the agent](marketplace/agent-integration.md): the experimental
  plugin backend, its IPC and how it is tested.
- [Plugin dependencies](marketplace/dependencies.md): required and optional
  dependencies of plugins (plugin API 1.4), how they are resolved, the
  install plan with each plugin's consent, dependency-aware disable,
  uninstall, update and rollback, and `plugins.db` schema 5.
- [Plugin artifact contract](marketplace/artifact-contract.md): ABI 1 wasm
  modules and the package.
- [Testing the marketplace locally](marketplace/local-testing.md): a local
  registry, the app and the agent.
- [Moving built-in modules to plugins](marketplace/module-migration.md): the
  core boundary, bundled default plugins, action id compatibility and the
  migration. *Design record, proposed.* Its evidence is the
  [module classification matrix](marketplace/module-matrix.md).

## Contributing and development

- [Development](development.md): toolchain, the live development session,
  builds, checks and conventions.
- [Contributing](../CONTRIBUTING.md): how to report issues and contribute
  pull requests.
- [Security policy](../SECURITY.md): supported versions and how to report a
  vulnerability privately.

## Releases

- [Releasing](releasing.md): release tags, the release workflow, signing
  and publication.
- [Release validation](release-validation.md): how a release candidate is
  validated, with the evidence to collect.
- [Release readiness](release-readiness.md): whether a version is ready to
  be published, and what is known about the current one.
- [MVP acceptance](mvp-acceptance.md): the repeatable macOS acceptance run
  and its results.

## Distribution

- [Distribution architecture](distribution/README.md): distribution
  metadata baked into each build, update providers per channel,
  cross-channel rules, downstream publication and the agent update
  lifecycle. *Design record; not yet implemented.*
- [Distribution audit](distribution/audit.md): what packaging, signing,
  updates and data handling do today, with code references.
- [Capability matrix](distribution/capability-matrix.md): GitHub, Homebrew,
  WinGet, APT, Flathub, the Mac App Store and the Microsoft Store compared
  on sandbox, agent, HID, plugin host, JIT, credentials and updates, with
  sources. The same matrix in machine-readable form:
  [`targets.toml`](distribution/targets.toml).

## Roadmap

- [Roadmap](roadmap.md): milestones, what is done, what comes later and
  what is out of scope.
