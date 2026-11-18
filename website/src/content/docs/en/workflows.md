---
title: Workflows
description: Combine actions into graphs with conditions, parallel branches, loops, retries and timeouts, and start them from keys, schedules or events.
group: automate
order: 8
---

A workflow runs several actions as one unit. It runs in the background agent, so it works with the app's window closed.

## Creating a workflow

Open **Workflows** in the navigation rail at the left of the window.

- **New workflow** opens the graph editor.
- **New step workflow** creates a simple sequential list of steps. **Open in graph editor** converts it to a graph later; the steps are kept.

The toolbar has **Save**, **Run workflow** (or **Cancel run**), **Undo**, **Redo** and **Arrange nodes**. A workflow with problems cannot be saved, and it has to be saved before it can run.

## Nodes

The catalog on the side lists the nodes you can add; **Search nodes** filters it. It includes your profile's actions by module, and can create a new action of any type.

| Node | Does |
| --- | --- |
| Start | Where the run begins. Every workflow has exactly one. |
| Action | Runs one of your actions. |
| Condition | Branches to **Yes** or **No**: a node succeeded, an output equals a value, an output is set, or the active project is a given one. |
| Parallel and Join | Run branches at the same time, then wait for them. |
| Loop | Runs another workflow 1 to 100 times. |
| Sub-workflow | Runs another workflow as one step. |
| Set variable | Stores a value for later nodes of this run. |
| Outputs | Declares what the workflow returns. |
| Delay | Waits, up to 10 minutes. |
| Stop | Ends the run. |

Loop and Sub-workflow appear in the catalog once another workflow exists.

### When a step fails

Each node has settings for failure:

- **On failure**: **Stop the workflow** or **Continue with Next**. If the node's **On error** output is connected, that path always handles the failure.
- **Retries**: up to 5, offered only for actions that are safe to repeat. **Wait between (ms)** doubles after each attempt. Sub-workflows are never retried.
- **Timeout (seconds)**: 30 seconds per action by default, at most 10 minutes per node. Some actions cannot be interrupted.

## Passing values between steps

Each parameter of a node takes the **Action's value**, a **Value for this node**, or a value **From earlier nodes or variables**: one output, one variable, or text composed of several, with a **Preview**.

A reference is written `{{ steps.<node>.output.<key> }}`. It is plain substitution; there are no expressions. See [Variables](../variables/) for everything a reference can name.

A workflow can declare **Inputs** and **Outputs** in its panel. A workflow with required inputs cannot be run directly: it is meant to be called from a Sub-workflow node.

Sensitive outputs, such as a command's output or a response body, reach later nodes but are never kept in the history.

## Starting a workflow

- From the app, with **Run workflow**.
- From the Stream Deck: place the workflow on a key ([Layouts](../layouts/)).
- With a trigger.

### Triggers

Select nothing in the editor to see the workflow's panel, and add a trigger under **Triggers** with **Schedule** or **Event**. A workflow can have up to 16; each has a **Paused** switch.

- **Schedules**: at an interval (1 minute to 7 days), every day, or on weekdays, at a local time.
- **Events**: DeckPilot started, Profile activated, Active project changed, Stream Deck connected, Stream Deck disconnected.

Triggers fire only for the active profile, and only while the configuration is unlocked. Nothing is caught up afterwards: a firing more than 2 minutes late is skipped, and so is one that comes while the previous run is still going.

## Watching a run

Running nodes are highlighted on the canvas, with the attempt number when a node is retried. The status bar lists running workflows and can cancel them. Cancelling stops the run before its next node; locking the configuration cancels every run.

In the inspector, **Runs** lists the last 20 runs, and **Run details** shows **Metrics**, **Failures** and a **Timeline**. DeckPilot keeps the newest 50 runs per workflow and 500 per profile. History is not exported.

## Limits

- A workflow has at most 200 nodes. Graphs have no cycles, and a workflow cannot call itself; nesting goes at most 4 levels deep.
- At most 4 workflows run at once, one run per workflow at a time, with up to 4 parallel nodes per run.
- **May change the active project** is a permission of each workflow, off for new workflows.
