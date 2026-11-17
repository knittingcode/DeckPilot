---
title: Updates
description: How DeckPilot checks GitHub Releases for new versions, the stable and pre-release channels, signature checks, and updating by hand.
group: run
order: 11
---

DeckPilot updates itself from GitHub Releases, the same place the installers are published. No other server is involved.

## How a check works

While the configuration app is open it asks GitHub for a newer version, once a day by default. Checks do not happen while only the agent is running.

When a newer version exists, DeckPilot tells you. What happens next is your choice.

## Settings

Open **Settings › Updates**.

| Setting | Options |
| --- | --- |
| Check for updates | Checks now |
| When an update is available | **Notify me** (default), **Download, then ask**, or **Install when idle** |
| Update channel | **Stable** (default) or **Pre-releases** |
| Check every | 6 hours, 12 hours, 1 day (default), 3 days or 7 days |
| Check automatically while DeckPilot is open | On by default; turn it off to check only by hand |

**Install when idle** installs after ten minutes without device input. It is marked as not recommended until updates have been tested on your system; installing automatically is off unless you choose it.

An update is applied with **Install and restart**. DeckPilot waits for running workflows before it updates.

## Channels

- **Stable** offers stable releases only. A pre-release is never offered on this channel.
- **Pre-releases** offers the newest published release of either kind, so you move on to a stable release as soon as one is newer than your pre-release.

> The default channel is Stable. An installed pre-release is offered a stable release as soon as one is newer than it, on either channel. To keep being offered pre-releases as well, select **Pre-releases**.

DeckPilot never installs an older version than the one you have. Switching back to Stable keeps your installed pre-release until a newer stable release exists.

## What is verified

Every update package is signed (minisign). Before installing, the app checks the signature against the public key that is compiled into it, and checks that the signed version is the one that was announced. A package whose signature does not verify is refused, and packages are accepted only from this project's GitHub release downloads.

## On each system

- **macOS**: the app installs the signed update package itself.
- **Windows**: the system installer takes over. This path is not covered by automated tests yet.
- **Linux**: DeckPilot updates the same kind of package that is installed (AppImage, `.deb` or `.rpm`). A `.deb` or `.rpm` is installed with the system's package tools, which ask for administrator authorization.

## Updating by hand

You can always download the newer installer from the [download page](../../download/) and install it over the existing version. Make an encrypted export first ([First launch](../first-launch/#back-up-with-an-encrypted-export)).

## What has not been tested

An update between two published releases has not yet been installed end to end on real installations; the mechanism is covered by automated tests. That is why automatic installation stays off by default.
