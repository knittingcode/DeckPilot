# Security

DeckPilot keeps all configuration on the local computer. Since version 0.4
the configuration database is encrypted by default. This document describes
what is protected, how, and where the limits are.

## What is protected

The configuration database (`deckpilot.db` in the data directory) holds
profiles, projects, actions, workflows, layouts, module settings, this
computer's resource bindings (paths, URLs) and runtime state. It is
encrypted with **SQLCipher** (AES-256 in CBC mode with per-page HMAC-SHA512,
provided by a statically linked OpenSSL). Without the password, the file
reveals nothing but its size.

Not encrypted: the key derivation parameters (`deckpilot.vault.json`, no
secrets), logs (which never contain configuration values, keys or
passwords), and the IPC token file (protected by file permissions).

## Keys and passwords

```
password ──Argon2id──▶ 256-bit database key ──raw key──▶ SQLCipher
                ▲
   deckpilot.vault.json: algorithm, version, memory, iterations,
   parallelism, random 16-byte salt
```

- **Key derivation:** Argon2id (`RustCrypto` `argon2`), 64 MiB memory,
  3 iterations, 1 lane, a random 16-byte salt per database. These defaults
  are above the OWASP minimum (19 MiB, 2 iterations) and take well under a
  second on current hardware.
- **One derivation layer:** the derived key is passed to SQLCipher as a
  *raw key* (`PRAGMA key = "x'…'"`), which skips SQLCipher's own PBKDF2
  passphrase derivation. There are no stacked or redundant KDFs.
- **Verification:** a wrong password yields a key whose HMAC check fails on
  the first page; SQLCipher refuses to read and nothing is written. There is
  no separate password hash that could be attacked offline more cheaply than
  the database itself.
- **Storage:** the password is never stored. The key exists only in memory
  of the configuration app and the agent while unlocked, in buffers that are
  zeroized when dropped (`zeroize`). Secret types are redacted from `Debug`
  output, so they cannot end up in logs by accident. Where third-party code
  (JSON serialization, IPC framing, SQLCipher internals) briefly copies a
  key, those copies are not under DeckPilot's control.
- **No custom cryptography:** DeckPilot only composes maintained libraries
  (SQLCipher/OpenSSL, `argon2`, `chacha20poly1305`, the OS random generator).

## Password lifecycle

| Situation | What happens |
| --------- | ------------ |
| First start | The app shows the setup screen; the password (at least 8 characters) is entered twice; an empty encrypted database is created. |
| App restart while the agent runs | The app fetches the key from the unlocked agent; no prompt. |
| Computer restart | The agent starts locked (autostart). Devices show "Locked", the tray says so, and the app asks for the password. |
| First start after an update | The password is asked for once (the app stores the version it was last confirmed for). The database is not re-encrypted. |
| Password change (Settings) | Requires the current password. The agent is locked, a copy encrypted with the new key is written and verified, files are swapped, and the agent is unlocked again. |
| Lock now (Settings) | The app and the agent forget the key. |

There is no password recovery. Keep an encrypted export as a backup.

## Trust boundary between the app and the agent

The configuration app is the only component that sees the password. It
derives the key and sends **the key, not the password**, to the agent over
the local IPC socket (`unlock`). The agent can hand the key back to the app
(`getDatabaseKey`) so that reopening the window does not ask again.

The IPC socket is authenticated with a random token stored in a file only
the user can read (and the socket itself is only accessible to the user).
Consequently, **any process running as the same user while DeckPilot is
unlocked can obtain the key** — the same as with other unlocked password
managers and keychains. Encryption protects the configuration at rest:
on a stolen disk, in backups, or from other user accounts. It does not
protect against malware running in your session.

Passwords never appear in command-line arguments, environment variables,
log files or persistent IPC tokens.

## Encrypting a configuration from an earlier version

DeckPilot 0.3 stored the database unencrypted. On the first start of 0.4 the
app detects it (by its header) and explains the migration. Nothing is
changed until the user sets a password.

1. The integrity of the old database is checked and its write-ahead log is
   folded into the main file.
2. An encrypted copy is written directly with `sqlcipher_export` (no
   intermediate unencrypted file) and verified: integrity, schema version and
   the row count of every table.
3. The files are swapped with renames. The original is kept as
   `deckpilot.unencrypted-backup.db` until the user deletes it in Settings.
   Interrupted migrations are repeated or completed on the next start.

### Removing the unencrypted copy

Settings offers to delete it. **Deleting a file does not guarantee that its
contents are unrecoverable**, especially on SSDs (wear levelling), on
copy-on-write file systems (APFS, Btrfs) and in backups or snapshots
(Time Machine, Windows File History, cloud sync). For a thorough cleanup:

- Use full-disk encryption (FileVault, BitLocker, LUKS); then the deleted
  blocks were encrypted all along.
- Delete the copy, then remove it from backups and snapshots that contain it,
  or let them expire.
- On SSDs, rely on full-disk encryption (or the drive's secure-erase
  function when retiring the drive) rather than overwriting tools.

## Backups and schema upgrades

Before a schema upgrade, DeckPilot checks the database's integrity, writes an
**encrypted** copy with the same key to `backups/` next to the database,
verifies it, and only then upgrades in a single transaction. The result is
recorded in `migration_history`. The newest five backups are kept. See
[data-model.md](data-model.md#compatibility-between-versions) for the
version policy.

## Encrypted exports

Exports are separate files protected by an **export password** chosen per
export. Format (`.deckpilot`, JSON):

```json
{ "header": "{…}", "ciphertext": "<base64>" }
```

The header contains `format: "deckpilot-configuration"`, `version: 1`,
`cipher: "xchacha20poly1305"`, a random 24-byte nonce, the Argon2id
parameters with their own random salt, the creation time and the app
version. The exact header text is authenticated as associated data of
XChaCha20-Poly1305 (`RustCrypto` `chacha20poly1305`), so any change to it
or to the ciphertext is detected. A wrong password and a modified file are
reported the same way.

The payload (version 1) contains profiles (with projects, actions,
workflows, layouts), the portable settings of feature modules and the
logical resource keys the profiles use with their kinds. It never contains
machine paths or application locations, the database key, passwords, IPC
tokens, runtime state or caches.

### Importing

Imports are treated as untrusted input: the file size is limited (32 MiB),
the archive is authenticated, then the payload's structure, identifiers,
references and size limits are validated before anything is shown. The user
then sees a preview (profiles and conflicts, modules and their status,
unknown action types, resources to set up on this computer) and chooses to
merge (per-profile: import as a copy, replace or skip) or replace all local
profiles. The import is written in one transaction. Imported workflows do
not run, imported profiles are not activated, and this computer's resource
bindings are never overwritten. No module code runs during an import:
module settings are checked against the module's declared contract.

## Reporting a vulnerability

Please report security problems privately through GitHub's
[security advisories](https://github.com/knittingcode/DeckPilot/security/advisories/new)
rather than in public issues.
