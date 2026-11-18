---
title: Plugins
description: DeckPilot installs plugins from a signed Marketplace. Where to find them, what they may do, and how updates and removal work.
group: run
order: 12
---

> **DeckPilot installs plugins from its signed Marketplace.** They are installed from DeckPilot's signed plugin registry, run in a separate sandbox and get only the permissions you agree to. The GitHub, GitLab and Jira integrations and notifications are plugins built into DeckPilot.

## What you get

- **Where**: the **Marketplace** button in the toolbar, with the tabs **Discover**, **Installed** and **Updates**. **Settings › Extensions › Plugins** shows the same, starting with the installed plugins. Search the registry, read each plugin's description, version and the permissions it asks for.
- **From where**: DeckPilot's plugin registry, the one the [Plugin Marketplace](https://knittingcode.github.io/deckpilot-plugins/en/) shows. A package file can be installed from disk only if its bytes are a version that the verified registry lists; there is no unverified sideloading.
- **The first plugins**, by DeckPilot: Air Quality (Polish air quality stations, data from GIOŚ), Markets (ECB euro reference rates, via Frankfurter), Pomodoro, Break & Hydration, and Memory (a game on your keys).
- **Built in**: the GitHub, GitLab and Jira integrations and notifications come with DeckPilot as plugins and cannot be replaced from a registry. When you update from a release without plugins, each configured integration moves to its plugin; a stored token or a server on a private network is used by the plugin only after you agree. Configurations a plugin cannot serve yet, such as GitHub Enterprise, stay on the built-in integration.

## How installing works

- **Consent**: installing opens a review of what the plugin may do. You consent to each permission, and a plugin runs only with every permission it declares. Permissions can be revoked later.
- **Enabled separately**: a plugin is installed disabled and is turned on with its own switch. An installed plugin can be configured and its actions bound to keys like any other action.
- **Sandbox**: plugins are WebAssembly modules that run in a separate, sandboxed host process, part of every installer. A plugin reaches only the network hosts it names, notifications, its keys and its own storage. File system, process and clipboard access can never be granted, and a plugin never sees your stored credentials directly.
- **Verified registry**: on every refresh DeckPilot checks the registry against root keys built into the application: its signed key set, index and timestamp, and every package by its size and SHA-256. A stale, rolled-back or tampered registry is refused, and plugins that are already installed keep working.
- **Updates and removal**: updates are checked from the same page; an update that asks for more permissions needs your consent again. A plugin can be rolled back to an earlier version. Uninstalling removes the plugin's files, grants, storage, settings and credentials unless you choose to keep its settings or credentials.
- **Turning plugins off**: plugins are on by default. Start the agent with `--disable-plugins`, or set `DECKPILOT_ENABLE_PLUGINS=0`, and the built-in integrations run as before.

## For plugin developers

Writing plugins (the SDK, the manifest, permissions, testing and publishing) is documented on the Marketplace site, not here: [plugin developer portal](https://knittingcode.github.io/deckpilot-plugins/en/developers/). The registry publishes DeckPilot's own plugins; it is not yet open to plugins from other publishers.
