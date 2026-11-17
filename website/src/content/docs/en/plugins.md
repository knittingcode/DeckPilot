---
title: Plugins
description: Plugins are experimental and not part of any released DeckPilot build. What exists today, how installing will work, and where to follow it.
group: run
order: 12
experimental: true
---

> **Plugins cannot be installed in any released version of DeckPilot.** The plugin system is experimental, it is switched off in every release build, and the registry has no published plugins. Everything DeckPilot does today comes from its built-in modules ([Actions](../actions/#built-in-modules)).

## What is true today

- A released DeckPilot has no Plugins page and no plugin code. There is nothing to turn on.
- The plugin system exists in DeckPilot's source code as experimental work, behind switches that are off by default and that the release workflow does not set.
- The [Plugin Marketplace](https://knittingcode.github.io/deckpilot-plugins/en/) is a separate website. It shows how the Marketplace will work, but its registry is empty: the signing keys for the production registry have not been created yet.
- There is no date for plugins in a release.

## How installing a plugin is designed to work

This describes the experimental implementation in development builds. It can change before it reaches a release.

- **Where**: a **Plugins** page in **Settings › Extensions**, with the tabs **Discover**, **Installed** and **Updates**.
- **From where**: a signed registry. A package file can be installed from disk only if its bytes are a version that the verified registry lists; there is no unverified sideloading.
- **Consent**: installing opens a review of what the plugin may do. You consent to each permission, and a plugin runs only with every permission it declares. Permissions can be revoked later.
- **Enabled separately**: a plugin is installed disabled and is turned on with its own switch.
- **Sandbox**: plugins are WebAssembly modules that run in a separate, sandboxed host process. File system, process and clipboard access can never be granted.
- **Updates and removal**: updates are checked from the same page; an update that asks for more permissions needs your consent again. Uninstalling removes the plugin's files, grants, storage, settings and credentials unless you choose to keep its settings or credentials.

## For plugin developers

Writing plugins (the SDK, the manifest, permissions, testing and publishing) is documented on the Marketplace site, not here: [plugin developer portal](https://knittingcode.github.io/deckpilot-plugins/en/developers/).
