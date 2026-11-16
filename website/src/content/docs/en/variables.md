---
title: Variables
description: Reference shared values, the active project, system values and the outputs of earlier steps with one syntax, and keep secrets redacted.
group: automate
order: 9
---

Variables are how a value gets from where it is known to where it is needed: a path used by many actions, the key of the active project, the output of an earlier step.

## Syntax

A reference is a name between double braces:

```text
{{ projects.current.key }}
cd {{ shared.paths.root }}
```

It is plain substitution. There are no expressions, a value is never expanded a second time, and an unknown name such as `{{ env.home }}` is an error. A reference that is the whole value keeps its type; inside text it is rendered as text.

## What you can reference

| Name | Value |
| --- | --- |
| `shared.<namespace>.<name>` | A shared variable you created |
| `projects.current.id`, `.key`, `.name` | The active project |
| `system.profile.current.id`, `.name` | The active profile |
| `system.device.current.id` | The current device |
| `system.platform` | The operating system |
| `workflow.id`, `workflow.execution.id` | The running workflow and this run |
| `workSession.state`, `.startedAt`, `.elapsedSeconds`, `.canFinish` and others | The current Work Session |
| `steps.<node>.output.<key>` | An output of an earlier node of the workflow |
| `steps.vars.output.<key>` | A variable set earlier in this run |

## Where references work

In action parameters, in the mappings of workflow nodes, and in key bindings and their parameter overrides. Type `{{` in a field to open the list of available variables, or use **Insert a variable** to open the picker. The picker shows each variable's value, source, type and scope, grouped as Shared, System, Profile, Project, Modules, Workflow and Step outputs.

Current values are shown only while the agent is running. Values that depend on a run, such as the run's project, are not known until the workflow runs.

## Shared variables

Open **Settings › General › Variables** and choose **New variable**.

- **Key**: for example `paths.root`, two to four segments separated by dots. It cannot be changed after saving.
- **Label** and **Description**.
- **Type**: Text, Integer, Yes/no, Project, Profile, Resource, JSON or Stored credential.
- **Scopes**: Global, Profile, Project.
- **Sensitive**, and a **Default value**.

A variable can have a different value per scope, set under **Values**. A project's value wins over the profile's, the profile's over the global one, and the default is used last.

The table lists every variable with its owner, type, scope, current value and whether it is editable, and can copy a reference. A variable that is still used cannot be deleted until its references are removed.

## Secrets

- A sensitive variable is shown as redacted. Its value is never shown again, in logs or in history.
- A **Stored credential** variable holds only the name of a credential, not the secret ([Actions](../actions/#credentials)).
- A sensitive value that would flow into a non-sensitive output, variable or input fails validation, so it cannot leak by accident.
- A stored credential or a resource inside text is a type error.
