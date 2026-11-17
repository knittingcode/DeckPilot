---
title: Background agent
description: The agent owns the device and runs your actions with the window closed. How to use its tray menu, start it at login and stop it.
group: run
order: 10
---

DeckPilot has two programs. The **agent** talks to the Stream Deck, draws the keys, runs actions and workflows, and provides the tray icon. The **configuration app** is the window where you design things. The app starts the agent if it is not running; the agent keeps running when the window is closed.

## The tray menu

| Item | Does |
| --- | --- |
| Open DeckPilot | Opens the configuration app. While locked it reads **Open DeckPilot to Unlock…** |
| Active Profile | A submenu to switch the profile on the device |
| Project | Shows the active project |
| Connected Devices | Lists the devices the agent sees |
| Refresh Devices | Looks for devices now |
| About DeckPilot | Information about the application |
| Quit DeckPilot | Stops the agent |

While the configuration is locked, the menu says **Locked – actions are paused**.

On Linux the tray icon needs AppIndicator (StatusNotifier) support. Without it there is no icon, but the agent works.

## Closing and quitting

- Closing the window leaves the agent running. On macOS, so does ⌘Q, which quits the app's interface only.
- **Quit DeckPilot** in the tray menu stops the agent and blanks the keys. On macOS it also closes the app; on Windows and Linux the app stays open.
- In the app, **Settings › Background Agent** has **Stop agent** and **Start agent**.

## Starting at login

Turn on **Start the agent when I log in** under **Settings › Background Agent**, in **Startup**. DeckPilot registers the agent for your user account only: a LaunchAgent on macOS, an XDG autostart entry on Linux, a Run key for the current user on Windows. No system service is installed.

The same can be done from a terminal:

```sh
deckpilot-agent autostart enable
deckpilot-agent autostart disable
deckpilot-agent autostart status
```

## When app and agent differ

The app and the agent negotiate what they both support, so a newer app works with an older agent for what both understand. When they do not fit, the app says so, for example **The running agent does not match this app** or that the agent is too old, and offers **Restart agent**. If workflows are running, the button reads **Interrupt workflows and restart**: nothing ends a workflow silently.

## Logs and diagnostics

The agent writes a log file per day into the `logs` folder of DeckPilot's data folder and keeps seven of them. Open **Settings › Logs & Diagnostics** and choose **Show log…** to read it.

The **Diagnostics** card has a **Copy** button for a report to attach to a bug report: versions of the app and the agent, the operating system, data and log folders, the number of devices and compatibility. It contains paths on your computer but no configuration content.
