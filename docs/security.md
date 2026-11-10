# Security

DeckPilot keeps all configuration on the local computer. Since version 0.4
the configuration database is always encrypted. This document describes
what is protected, how the key is protected, what each mode guarantees, and
where the limits are.

## What is protected

The configuration database (`deckpilot.db` in the data directory) holds
profiles, projects, actions, workflows, layouts, module settings, this
computer's resource bindings (paths, URLs), runtime state and the security
settings. It is encrypted with **SQLCipher** (AES-256 in CBC mode with
per-page HMAC-SHA512, provided by a statically linked OpenSSL) using a
random or derived 256-bit key. Without the key, the file reveals nothing
but its size. Encryption is on in both protection modes; there is no
unencrypted mode.

Not encrypted: `deckpilot.vault.json` (which protection is used, with its
non-secret parameters), logs (which never contain configuration values,
keys or passwords), and the IPC token file (protected by file permissions).

## Protection modes

| | System key (default) | Password |
| --- | --- | --- |
| Key | 256 random bits from the OS random generator | Argon2id(password, random salt) |
| Where the key lives at rest | the operating system's credential store | nowhere; derived from the password when needed |
| Opening after a restart | automatic, when the user logs in | the password must be entered |
| After an update | no prompt (unless the policy asks) | the password is needed after the restart anyway |
| Protects against | a stolen or copied disk or backup *without* the user's login; other user accounts | the same, and anyone who can use the logged-in session but does not know the password |
| Does not protect against | someone or something using the logged-in session | malware in the session while unlocked |

Switch in **Settings → Security → Unlock method**. Switching is a sensitive
operation (see below) and re-encrypts the database with a new key; if any
step fails, the database and its previous protection stay as they were.

### System key

On first start the background agent creates a random 256-bit key, stores
it in the credential store, reads it back to verify it, writes the vault
file and creates the encrypted database. No password is involved.

| Platform | Credential store | Protection |
| -------- | ---------------- | ---------- |
| macOS | login Keychain, generic password item | encrypted with the login keychain; the item's access list contains the agent executable |
| Windows | Credential Manager (generic credential, *local machine* persistence, not roaming) | encrypted per user with DPAPI |
| Linux | Secret Service over D-Bus (GNOME Keyring, KDE Wallet) | encrypted in the user's login collection |

- **No fallback.** If the credential store is not available (no Secret
  Service on a headless Linux, a locked or reset keychain, access denied),
  DeckPilot does not store the key anywhere else. On first start the app
  explains the problem and offers to retry or to use a password instead —
  the user's explicit choice. An existing configuration stays locked with a
  clear message ("the key could not be read from the credential store"); it
  is not retried in a loop and nothing is re-created.
- **Never replaced.** Every key has its own random identifier (recorded in
  the vault file; entries are named `database-key-<id>` under the service
  `DeckPilot`). Storing never overwrites an existing entry, and DeckPilot
  never deletes a key that a database or a backup may still need: after
  switching to a password, the previous key stays in the store (encrypted
  backups made before the switch need it). You can remove such entries
  yourself once those backups are no longer needed.
- **One reader.** Only the background agent reads the credential store. The
  configuration app receives the key from the agent over the authenticated
  local socket. This keeps a single executable in the keychain item's access
  list. On macOS, builds that are not code-signed with a stable identity
  (development builds, and releases until Apple code signing is configured;
  see [releasing.md](releasing.md)) can make macOS ask once whether the
  updated agent may use the key ("Always Allow").
- **Not a backup.** The key in the credential store protects the copy on
  this computer only. It does not move to another computer, and if the user
  account, the keychain or the computer is reset or lost, the configuration
  cannot be opened. **Keep an encrypted export** as the backup (see below).

### Password

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
- **One derivation layer:** the key is passed to SQLCipher as a *raw key*
  (`PRAGMA key = "x'…'"`), which skips SQLCipher's own PBKDF2 passphrase
  derivation. The same applies to system keys.
- **Verification:** a wrong password yields a key whose HMAC check fails on
  the first page; SQLCipher refuses to read and nothing is written. There is
  no separate password hash that could be attacked offline more cheaply than
  the database itself.
- **Storage:** the password is never stored. There is no password recovery.

### Common to both modes

- Keys exist in memory of the configuration app and the agent while
  unlocked, in buffers that are zeroized when dropped (`zeroize`). Secret
  types are redacted from `Debug` output, so they cannot end up in logs by
  accident. Where third-party code (JSON serialization, IPC framing,
  SQLCipher internals, the credential store libraries) briefly copies a key,
  those copies are not under DeckPilot's control.
- Keys and passwords never appear in command-line arguments, environment
  variables, log files, the vault file or persistent IPC tokens.
- **No custom cryptography:** DeckPilot only composes maintained libraries
  (SQLCipher/OpenSSL, `argon2`, `chacha20poly1305`, `keyring-core` with the
  platform stores, the OS random generator, the OS authentication dialogs).

## Lifecycle

| Situation | System key (default) | Password |
| --------- | -------------------- | -------- |
| First start | The agent creates the key and the encrypted database; the app opens without a prompt. | Only when chosen (e.g. no credential store): the password is entered twice. |
| Login / computer restart | The agent starts with the login, reads the key and restores the active profile, project and layouts. | The agent starts locked; devices show "Locked"; the app asks for the password. |
| App restart while the agent runs | The app gets the key from the agent. | The same. |
| Agent restart | Unlocks itself again. | The app (if open) hands it the key. |
| Update | No prompt; the database is not re-encrypted. | The password is needed after the restart. |
| Credential store unavailable | Locked, with the reason; "Try again" once it is available. | Not affected. |
| Lock now / auto-lock | Unlock with Touch ID or the login password (macOS). | Unlock with the password. |

## Locking and authentication

**Settings → Security** has two policies, stored in the encrypted database:

- **Lock automatically:** *Never* (default), *when the computer sleeps*,
  *after 15 minutes* or *after 1 hour of inactivity*. Inactivity means no
  device input and no configuration change. Sleep is detected by the agent
  on every platform (a wall-clock jump between its periodic checks). Locking
  when the *screen* locks is not implemented and therefore not offered.
  While locked the devices show "Locked" and run no actions.
- **Ask for authentication:** *only for sensitive operations* (default),
  *also after updates*, or *also every time DeckPilot starts*.

**Sensitive operations always require a fresh authentication**, valid for
two minutes and used up by one operation: exporting the configuration,
switching the unlock method, changing these settings, and deleting the
unencrypted copy of a legacy database. Changing the password requires the
current password. Switching profiles or projects, device operations and
running workflows never ask. (DeckPilot does not store credentials for
modules yet; when it does, viewing or removing them will be sensitive too.)

How the user is verified:

| | macOS | Windows, Linux |
| --- | --- | --- |
| Password mode | the DeckPilot password | the DeckPilot password |
| System key | Touch ID or the login password, through `LocalAuthentication` | an explicit confirmation only |

Touch ID is used only through the operating system's dialog; DeckPilot never
sees fingerprints or the login password. Without a system authentication
(Windows Hello and polkit are not integrated yet), a system-key setup offers
neither automatic locking nor the stricter authentication policies: they
would not be backed by anything. Sensitive operations then ask for an
explicit confirmation, which prevents accidents but not access by someone at
the unlocked computer — choose a password if that matters.

The lock and these checks are enforced by the configuration app and the
agent. In system-key mode they are a barrier in the user interface, not a
cryptographic one: a process running as the same user can read the key from
the credential store or ask the agent (next section).

## Trust boundary between the app and the agent

The agent reads the system key, or receives the key that the configuration
app derived from the password (the key, **not the password**), over the
local IPC socket. The app gets the key back from the agent
(`getDatabaseKey`) so that reopening the window does not ask again.

The IPC socket is authenticated with a random token stored in a file only
the user can read (and the socket itself is only accessible to the user).
Consequently, **any process running as the same user while DeckPilot is
unlocked can obtain the key** — the same as with other unlocked password
managers and keychains. Encryption protects the configuration at rest: on a
stolen disk, in backups, or from other user accounts. It does not protect
against malware running in your session.

## Development builds

Development and test builds never touch an installed DeckPilot's keys:
debug builds and isolated data directories (`DECKPILOT_DATA_DIR`) use the
credential store service `DeckPilot Development`, and automated tests use an
in-memory store and a scripted authentication — no real credentials are
read, written or deleted by tests. To remove development keys on macOS:
`security delete-generic-password -s "DeckPilot Development"` (repeat until
none is left).

## Encrypting a configuration from an earlier version

DeckPilot 0.3 stored the database unencrypted. On the first start of 0.4 the
agent detects it (by its header) and encrypts it with a new system key. If
the credential store is not available, nothing is changed and the app
explains the migration and offers a password instead.

1. The integrity of the old database is checked and its write-ahead log is
   folded into the main file.
2. An encrypted copy is written directly with `sqlcipher_export` (no
   intermediate unencrypted file) and verified: integrity, schema version and
   the row count of every table.
3. The files are swapped with renames. The original is kept as
   `deckpilot.unencrypted-backup.db` until the user deletes it in Settings.
   Interrupted migrations are repeated or completed on the next start.

### Removing the unencrypted copy

Settings offers to delete it (a sensitive operation). **Deleting a file does not guarantee that its
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

Exports are the backup and the way to move a configuration to another
computer, in both protection modes. They are separate files protected by an
**export password** chosen per export, independent of the database key and
of the credential store. Format (`.deckpilot`, JSON):

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
