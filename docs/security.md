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
- **Never replaced.** A missing database is never re-created behind the
  user's back: when the key file shows that a configuration existed but
  `deckpilot.db` is gone (moved, deleted, a drive not connected), nothing is
  created and the app explains the situation; only an explicit "Start a new
  configuration" moves the old key file aside (it is kept, as is its key)
  and creates an empty configuration. A first setup is staged so that an
  interruption never leaves a key file without its database. Changes to the
  files are serialized between the agent and the app with a lock file
  (`deckpilot.vault.lock`).
- **Never overwritten.** Every key has its own random identifier (recorded in
  the vault file; entries are named `database-key-<id>` under the service
  `DeckPilot`). Storing never overwrites an existing entry, and DeckPilot
  never deletes a key that a database or a backup may still need: after
  switching to a password, the previous key stays in the store (encrypted
  backups made before the switch need it). You can remove such entries
  yourself once those backups are no longer needed.
- **One reader.** Only the background agent reads the credential store. The
  configuration app receives the key from the agent over the authenticated
  local socket. This keeps a single executable in the keychain item's access
  list, so restarting the agent or the app never prompts (verified on macOS).
- **macOS and code signatures.** The Keychain grants access to the item to
  the agent's *code identity*. When that identity changes — every update of
  a build that is not signed with a Developer ID, and every rebuild of the
  agent during development — macOS asks for the login keychain password
  before the agent may read the key ("Always Allow" covers that build
  only). DeckPilot does not hang meanwhile: the agent keeps running, the app
  shows "Waiting for the macOS Keychain", and if access is denied the
  configuration stays locked with a "Try again" button. Releases signed with
  a Developer ID keep a stable identity across updates (the agent also has a
  fixed signing identifier from an embedded `Info.plist`) and are not asked;
  until signing is configured (see [releasing.md](releasing.md)), the
  "no prompt after an update" promise of the system key does **not** hold on
  macOS. Windows Credential Manager and the Secret Service have no
  per-application access list, so updates do not prompt there.
- **Startup.** The key is read on a background thread, so a credential store
  prompt never blocks the agent. If the store cannot be *reached* (no D-Bus
  session yet, the keyring daemon starting after the agent at login), the
  agent tries again after 2, 4, 8, 16, 32 and 64 seconds. A *refusal* (a
  locked or reset keychain, a cancelled or denied prompt) is never retried
  by itself, so the user is not asked again and again; "Try again" in the
  app asks once more.
- **Linux.** Items are created in the default (login) collection, labelled
  "DeckPilot database key". If that collection is locked, the keyring shows
  its own unlock prompt. Windows stores the credential with *local machine*
  persistence; DPAPI ties it to the user's Windows password.
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
  *when the screen locks or the computer sleeps*, *after 15 minutes* or
  *after 1 hour of inactivity*. Inactivity means no device input and no
  configuration change. Sleep is detected by the agent on every platform (a
  wall-clock jump between its checks every 5 seconds). The screen lock is
  detected where it can be read reliably, and the option is disabled with
  an explanation elsewhere:

  | Platform | Screen lock source |
  | -------- | ------------------ |
  | macOS | the window server session (`CGSSessionScreenIsLocked`) |
  | Linux | systemd-logind's `LockedHint` of the session (set by GNOME, KDE Plasma and other lockers; desktops that do not set it are not detected) |
  | Windows | not implemented; the option is not available |

  DeckPilot locks when the screen *becomes* locked, so unlocking DeckPilot
  while the screen is still locked does not lock it again at once. While
  locked the devices show "Locked" and run no actions.
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
workflows, layouts and their bindings), the portable settings of feature
modules, portable application settings (how updates are checked and
applied) and the logical resource keys the profiles use with their kinds.
It never contains machine paths or application locations, the database key
or anything from the credential store, passwords, IPC tokens, the security
policy (importing it could weaken another computer's protection), runtime
state (such as the time of the last update check) or caches. Stored
credentials (the secrets modules use by name) are not exported, secrets or
descriptions: a module setting such as GitHub's `credential` carries only the
credential's *name*, and each computer stores its own secret. Module settings
a module declares machine-specific or sensitive are never exported, and are
dropped when a (hand-made) export carries them anyway. Exporting works the
same in both protection modes and asks for authentication first.

`crates/deckpilot-portability/tests/no_secrets_in_exports.rs` checks this
with marked values. It fills a store with credentials, machine-specific
module settings, machine paths, security settings and runtime state, then
asserts that none of them appear in the payload or the sealed archive (in
which not even profile names are readable). It also checks that importing
never adds or replaces a credential, even from a crafted export, that
exporting one profile contains nothing of the others, and that imported
copies get their own identifiers, so projects and active-project selections
never cross profiles.

A secret the user types into an ordinary text parameter of an action is
configuration like any other text and is exported with its profile. The
HTTP module refuses secret headers and query parameters written into a
workflow for this reason; credentials are the place for secrets.

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
