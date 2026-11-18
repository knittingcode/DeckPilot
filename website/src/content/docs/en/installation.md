---
title: Installation
description: Install DeckPilot on macOS, Windows or Linux, and what to expect from each installer, including signing and the Linux udev rule.
group: start
order: 1
---

DeckPilot is published as installers on GitHub Releases. The [download page](../../download/) lists the files of the latest published release with their sizes and SHA-256 checksums.

> The only supported device is the **Stream Deck MK.2**, and DeckPilot has been used with real hardware on macOS only. On Windows and Linux it is covered by automated tests.

## What you need

- A Stream Deck MK.2. Other models can only be used to design layouts (see [Devices](../devices/)).
- One of the systems DeckPilot is built for:

| System | Architecture | Installer |
| --- | --- | --- |
| macOS | Apple Silicon | `DeckPilot_<version>_aarch64.dmg` |
| macOS | Intel | `DeckPilot_<version>_x64.dmg` |
| Windows | x64 | `DeckPilot_<version>_x64-setup.exe` or `DeckPilot_<version>_x64_en-US.msi` |
| Linux | x64 | `DeckPilot_<version>_amd64.AppImage` or `DeckPilot_<version>_amd64.deb` |

There is no ARM build for Windows or Linux. Stable releases will also have an `.rpm` package; pre-releases do not.

## macOS

1. Download the disk image for your Mac: `aarch64` for Apple Silicon, `x64` for Intel.
2. Open it and copy DeckPilot to your Applications folder.
3. Start DeckPilot.

The macOS builds are signed with an Apple Developer ID and notarized by Apple, so Gatekeeper accepts them. If macOS asks for the Input Monitoring permission, grant it to the agent: it is what lets DeckPilot read the device.

## Windows

1. Download the setup program (`-setup.exe`) or the MSI package.
2. Run it.
3. Start DeckPilot.

The Windows installers are **not code-signed yet**. Windows SmartScreen therefore shows a warning the first time you start one. Compare the file's SHA-256 checksum with the one on the [download page](../../download/#verify-title) before you continue. The device needs no driver installation: it uses the HID driver built into Windows.

## Linux

Choose the Debian package or the AppImage.

### Debian package

```sh
sudo apt install ./DeckPilot_<version>_amd64.deb
```

The package installs a udev rule that gives the logged-in user access to the Stream Deck MK.2 without root. Replug the device after installing. `apt` also installs the packages DeckPilot depends on, among them `libayatana-appindicator3-1` for the agent's tray icon (on Fedora, the `.rpm` installs `libayatana-appindicator-gtk3`).

### AppImage

```sh
chmod +x DeckPilot_<version>_amd64.AppImage
./DeckPilot_<version>_amd64.AppImage
```

The AppImage cannot install the udev rule, so do it yourself once. Download [`70-deckpilot-streamdeck.rules`](https://github.com/knittingcode/deckpilot/blob/main/packaging/linux/70-deckpilot-streamdeck.rules) from the repository, then:

```sh
sudo cp 70-deckpilot-streamdeck.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Unplug the device and plug it in again.

### The tray icon

The agent's tray icon needs AppIndicator (StatusNotifier) support in your desktop. On desktops without it, for example GNOME without the extension, no icon is shown, but the agent keeps working and the app can still be opened normally.

## After installing

Start DeckPilot. The background agent starts with it and appears in the system tray. Continue with [First launch](../first-launch/).

## Where DeckPilot keeps its data

| System | Folder |
| --- | --- |
| macOS | `~/Library/Application Support/io.github.knittingcode.deckpilot` |
| Windows | `%APPDATA%\io.github.knittingcode.deckpilot` |
| Linux | `~/.local/share/io.github.knittingcode.deckpilot` |

The exact location of your configuration is shown in the app under **Settings › Storage**.
