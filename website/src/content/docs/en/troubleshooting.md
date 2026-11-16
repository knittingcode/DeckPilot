---
title: Troubleshooting
description: What to check when the Stream Deck is not detected, the tray icon is missing, the configuration will not open or an update fails.
group: help
order: 13
---

## The Stream Deck is not detected

**Settings › Devices** says **No Stream Deck detected**.

- Check the model. Only the Stream Deck MK.2 has a driver ([Devices](../devices/)).
- Choose **Refresh devices** in the title bar, or **Refresh Devices** in the tray menu. DeckPilot looks for devices every two seconds on its own.
- Make sure the agent is running: **Settings › Background Agent**, **Start agent**.
- Quit other software that controls the Stream Deck, such as the manufacturer's application, so that it does not conflict with DeckPilot over the device.
- **Linux**: the udev rule must be installed, and the device replugged afterwards. The `.deb` package installs it; with the AppImage you install it yourself ([Installation](../installation/#linux)).
- **macOS**: if macOS asks for the Input Monitoring permission, grant it to the agent.
- **Windows**: no driver is needed; the device uses the built-in HID driver.

DeckPilot has been used with a real device on macOS only. If it does not work for you on Windows or Linux, please [open an issue](https://github.com/knittingcode/deckpilot/issues/new/choose) with the diagnostics report (below).

## Windows warns about the installer

The Windows installers are not code-signed yet, so SmartScreen warns on first start. Check the file's SHA-256 checksum against the [download page](../../download/#verify-title) before continuing.

## There is no tray icon on Linux

The tray icon needs AppIndicator (StatusNotifier) support, which some desktops, such as GNOME without the extension, do not have. The agent still runs and the keys work; open the app from your application menu.

## DeckPilot cannot open its configuration

With the system key, the configuration's key is in your credential store (Keychain, Credential Manager, or Secret Service).

- If the store is locked, unlock it or sign in again, then try again. On Linux the keyring may show its own unlock prompt.
- If the system asks whether DeckPilot may use the credential store, allow it.
- If the key was deleted from the credential store, the configuration cannot be recovered. Start a new configuration and restore your encrypted export with **Settings › Backup & Restore › Import…**.

With a password, a forgotten password cannot be recovered either. An encrypted export is the only backup.

## The keys show "Locked"

DeckPilot is locked and runs no actions. Open the app and unlock it: **Unlock…**, or **Unlock with Touch ID** on macOS. Change when it locks in **Settings › Security**.

## The agent is not running, or does not match the app

- **Settings › Background Agent** shows the agent's state, with **Start agent**.
- If the app says the running agent does not match it or is too old, choose **Restart agent**. This usually follows an update.
- Read the agent's log under **Settings › Logs & Diagnostics**, **Show log…**.

## An action does nothing or is refused

- **Run command**, **Compose up**, **Compose down** and **Compose restart**, and requests to private networks are off until you allow them on this computer ([Actions](../actions/#things-that-are-off-until-you-allow-them)).
- Actions of a disabled module are shown as **Unavailable**. Enable the module in **Settings › Extensions › Modules**.
- A resource that is **Missing** or **Not set on this machine** needs a mapping in **Configure › Projects**, tab **This machine**.
- **Run** in the Action Library works only while the agent is running and the profile is the active one.

## A trigger did not fire

Triggers fire only for the active profile and only while the configuration is unlocked. A firing that is more than two minutes late is skipped, and so is one that arrives while the previous run is still going ([Workflows](../workflows/#triggers)).

## An update fails

DeckPilot refuses an update whose signature does not verify. Download the installer from the [download page](../../download/) and install it by hand. The update log is `updates.log` in the `logs` folder.

## Reporting a problem

1. Open **Settings › Logs & Diagnostics** and use **Copy** in the **Diagnostics** card. The report contains versions, your operating system and paths on your computer, but no configuration content.
2. [Open an issue](https://github.com/knittingcode/deckpilot/issues/new/choose) and paste the report.

Report security vulnerabilities privately, as described in the [security policy](https://github.com/knittingcode/deckpilot/blob/main/SECURITY.md), not in a public issue.
