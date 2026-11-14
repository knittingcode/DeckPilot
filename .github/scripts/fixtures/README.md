# Throwaway updater keys (test fixtures)

Minisign key pairs made with `tauri signer generate --ci` for
`.github/scripts/updater-key-pair-fixtures.sh`, the required check
*Updater signing keys match*. They sign nothing and are trusted by nothing:
DeckPilot builds trust only the release public key pinned in
`.github/updater.pub`.

| Files | Password |
|---|---|
| `throwaway-updater.secret`, `throwaway-updater.pub` | `throwaway fixture password` |
| `other-updater.secret`, `other-updater.pub` | empty |

The secret keys are named `.secret` because `*.key` is ignored by git, so
that real keys are not committed by accident. Never put a real key here. The release signing key exists only in the
`production-release` environment and on the owner's machine
([docs/releasing.md](../../../docs/releasing.md)).
