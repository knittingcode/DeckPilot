# Documentation

An index of DeckPilot's documentation. Documents marked *design record*
were written before the implementation and keep the audit and design that
led to it; where the code differs, the code and the tests win.

## Using DeckPilot

- [Devices](devices.md): supported Stream Deck models, drivers and how far
  each model has been verified on each platform.
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
- [Action architecture](action-architecture.md): action types, instances
  and shared variables. *Design record, implemented in 0.6.0.*
- [Integration modules](integration-modules.md): the System, Workspace,
  Git, Docker, HTTP, GitHub, GitLab, Jira and Notifications modules.
- [Compatibility](compatibility.md): protocol negotiation and the tested
  matrix of app and agent versions.

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

## Roadmap

- [Roadmap](roadmap.md): milestones, what is done, what comes later and
  what is out of scope.
