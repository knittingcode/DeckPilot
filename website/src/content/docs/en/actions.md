---
title: Actions
description: Create actions from the built-in modules, test them, manage credentials, and allow the capabilities that are off by default.
group: automate
order: 7
---

An action is one thing DeckPilot can do: open an application, read a repository's status, send an HTTP request. You create the actions you need from the action types that the built-in modules provide, then use them on keys and in workflows. The library starts empty; nothing is generated for you.

## Creating an action

1. Open **Configure › Action Library** and choose **New action**.
2. **Step 1 of 2: choose what the action does.** Action types are grouped by module; **Search action types** filters them.
3. **Step 2 of 2: name it and set its parameters.** Fill in **Name**, **Icon**, **Key label** (up to 32 characters; `{project.key}` and `{project.name}` are replaced with the active project) and **Description**, then the parameters of the type.
4. **Create action**.

An existing action has **Save**, **Duplicate**, **Run** and **Delete**. **Run** tests the action and shows the result, the message and the output values. It needs the agent to be running and the profile to be the active one. The **Used in** list shows where an action is referenced; deleting a referenced action asks, for each reference, whether to replace or unbind it.

Action types that modify something show the notice **This action changes things** before you save.

## Built-in modules

| Module | Action types |
| --- | --- |
| General | Log message, Wait |
| Projects | Cycle projects, Select project, Current project, Project repository, Project documentation, Resolve project resource |
| System | Open application, Open URL, Open file, Open folder, Run command, Program status |
| Workspace | Open in editor, Open pages, Open project workspace |
| Git | Repository status, Current branch, Changed files, Last commit, Remote status, Open repository |
| Docker | Docker status, Containers, Compose status, Compose up, Compose down, Compose restart, Container logs |
| HTTP | HTTP request, Health check |
| GitHub | Current GitHub user, GitHub repository, Open pull requests, Review requests, Checks, Open on GitHub |
| GitLab | Current GitLab user, GitLab project, Open merge requests, Review requests, Pipeline status, Open on GitLab |
| Jira | Current Jira user, Jira issue, Jira issues, Open in Jira |
| Notifications | Show notification, Workflow summary |
| Work Session | No action types; it has its own page and key ([Gestures](../gestures/#the-work-session-key)) |

What the modules deliberately do not do:

- **Git** only reads. It never commits, resets, cleans, checks out, pushes or fetches.
- **GitHub**, **GitLab** and **Jira** only read, with GET requests. GitHub Enterprise, self-managed GitLab and Jira Data Center are supported through the module's settings.
- **Docker** uses the Docker CLI with Compose v2. Compose down never removes volumes or images.
- **Open URL** accepts `http`, `https` and `mailto` addresses only.

## Turning modules on and off

**Settings › Extensions › Modules** has a switch for each module. Disabling a module hides its pages and action types; the actions, workflows and layouts that use it are kept and shown as **Unavailable** until you enable it again. A module cannot be disabled while an enabled module depends on it.

All modules are built into DeckPilot. Installing modules from other sources is not supported; see [Plugins](../plugins/) for the experimental plugin system.

## Things that are off until you allow them

Anything that can change your system or reach inside your network is off on every computer until you turn it on there. These switches are in each module's settings under **Settings › Extensions**. They are never exported, and importing a configuration cannot turn them on.

| Module | Switch | What it allows |
| --- | --- | --- |
| System | Allow running commands | **Run command**. It runs only programs you bound as resources, without a shell. |
| Docker | Allow starting and stopping services | **Compose up**, **Compose down** and **Compose restart** |
| HTTP | Allow this computer and private networks | Requests to this computer and to the private hosts you list |
| GitHub, GitLab | API host may be on a private network | A self-hosted API host on a private network |
| Jira | Site may be on a private network | A Jira site on a private network |

## Credentials

Tokens and other secrets are stored as credentials, never typed into an action.

Open **Settings › Extensions › HTTP**, card **Credentials**, and choose **Add a credential**: a **Name** (lowercase letters, digits, `.`, `_` and `-`, for example `github.token`), a **Description** and the **Secret**, then **Store credential**.

- A credential is stored encrypted on this computer only. It is never part of a workflow or an export, and it is not shown again after saving; use **Replace secret** to change it.
- An action or a module refers to a credential by its name.
- Credentials are sent only over HTTPS, or to this computer.
- For Jira Cloud the secret is `email:api-token`; Jira Data Center uses a personal access token. GitLab needs a token with the `read_api` scope.
