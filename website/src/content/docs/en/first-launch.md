---
title: First launch
description: What happens the first time DeckPilot starts, how the encrypted configuration is protected, and how to lock it and back it up.
group: start
order: 2
---

## What happens on first start

DeckPilot creates its configuration the first time it starts, without asking for a password:

1. The agent generates a random 256-bit key.
2. It stores the key in your system's credential store: the login Keychain on macOS, the Credential Manager on Windows, or the Secret Service on Linux (GNOME Keyring, KDE Wallet).
3. It creates the encrypted configuration database.

If the credential store asks whether DeckPilot may use it, allow it. If the store is not available, the **Welcome to DeckPilot** window offers **Try again** or **Use a password instead**; with a password you finish with **Create encrypted configuration**.

The configuration is always encrypted with SQLCipher (AES-256). There is no unencrypted mode.

## System key or password

You choose how the configuration is unlocked under **Settings › Security**, in **Unlock method**:

- **System key** (the default): the key is in the credential store, so DeckPilot opens without a password.
- **Password**: the key is derived from your password with Argon2id. The password cannot be recovered; keep an encrypted export as a backup.

Switch with **Use a password instead…** or **Use the system key instead…**.

## Locking

**Settings › Security › Locking and authentication** controls when DeckPilot locks and when it asks you to authenticate.

- **Lock automatically**: Never, when the computer sleeps, when the screen locks or the computer sleeps, after 15 minutes of inactivity, or after 1 hour of inactivity.
- **Ask for authentication**: only for sensitive operations, after updates and for sensitive operations, or every time DeckPilot starts.
- **Lock now** locks immediately.

While DeckPilot is locked, the Stream Deck shows "Locked" and runs no actions.

Things to know:

- Touch ID on macOS is the only system authentication. Windows Hello and polkit are not integrated.
- With the system key on Windows and Linux, automatic locking and the stricter authentication policies are not available, and sensitive operations ask for a confirmation only.
- Locking together with the screen lock is detected on macOS and Linux, not on Windows.
- With the system key, the lock is a barrier in the user interface, not a cryptographic one. Use a password if you need the configuration to be unreadable without it.

## Back up with an encrypted export

An export is your backup, and the way to move a configuration to another computer.

1. Open **Settings › Backup & Restore**.
2. In **Export and import**, choose **Export…**, give the export its own password, then **Choose file and export**. The result is a `.deckpilot` file.

An export does not contain paths and other settings of this computer, keys or passwords. To restore, choose **Import…**, pick the file, **Show contents**, review the import (**Add to my configuration** or **Replace my profiles**) and confirm with **Import**.

## First steps

1. Connect a Stream Deck MK.2. It appears under **Settings › Devices** ([Devices](../devices/)).
2. Create a profile and add your projects ([Profiles and projects](../profiles/)).
3. Create actions from the modules you need ([Actions](../actions/)).
4. Place them on keys in the Layout Designer ([Layouts](../layouts/)).
5. Combine actions into workflows when one action is not enough ([Workflows](../workflows/)).
6. Make an encrypted export.
