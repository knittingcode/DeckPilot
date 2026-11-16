---
title: Profiles and projects
description: Profiles keep independent setups apart, projects let one key act on whatever you are working on, and resources bind them to a computer.
group: design
order: 4
---

## Profiles

A profile is an independent set of actions, workflows, projects and layouts, with its own icons. Use profiles to keep unrelated setups apart.

Open **Configure › Profiles**.

- **New profile** creates one: enter a **Name** and a **Description**, then **Save**.
- **Make active** puts a profile on the device. The active profile carries the badge **On device**.
- The pencil opens **Edit profile** (name and description), the copy icon duplicates the profile, and the bin deletes it after a confirmation.

If you delete the active profile, the device has no layout until you make another profile active.

### Editing one profile while another is active

The profile you edit is chosen in the drop-down at the top of the sidebar; it shows **Editing** and the profile's name. It is independent of the profile that runs on the device, so you can prepare a profile without disturbing the one in use.

## Projects

Projects live inside a profile. Each has a short **Key** (for example `SHOP`), a **Name** and **Resources**. Project-aware actions act on the active project, so one key can open the editor, the repository or the pages of whichever project you are working on.

Open **Configure › Projects**.

- **New project** creates a project.
- Each row can be edited, copied to another profile, or deleted.
- **Default project** is either the first project in the order or a specific one.
- **Cycle order**: drag to reorder. Next and previous project follow this order.

The project switcher card (previous, next, select) works for the profile that is active on the device. Switching only changes which project actions refer to: nothing is checked out, restarted or launched.

To switch projects from the Stream Deck, create a **Cycle projects** or **Select project** action ([Actions](../actions/)) and place it on a key.

## Resources on this computer

A project names its resources by role, for example a repository or a documentation folder. Where those really are differs between computers, so the mapping is kept per computer.

Open **Configure › Projects**, tab **This machine**, card **Resources on this machine**, and choose **Add mapping**:

- **Resource key**: the name actions refer to.
- **Kind**: Directory, Application, Terminal, Docker environment, Local service, URL or Other.
- **Value**: the path, program or address on this computer.

Mappings are stored for this computer only and are never part of a profile, which is why a profile can be exported and used elsewhere. Each mapping shows whether it was **Found**, is **Missing**, is **Not set on this machine**, or was **Not checked**.
