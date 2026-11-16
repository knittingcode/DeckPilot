# Update test fixtures

`test-key.pub` is the public half of a key pair generated only for these
tests with `tauri signer generate`. Its private key was discarded after
signing `artifact.bin` (producing `artifact.bin.sig`). This key is unrelated
to the release signing key and must never be used to sign releases.

`v0.6.0-alpha.3-prerelease.json` is the update manifest published with
DeckPilot 0.6.0-alpha.3, byte for byte as downloaded from that release
(SHA-256 `6bba36ae5fb27e48cceb606b6e38a962d9c850d045668a6eb1cd208285bc7084`).
The artifact checks must keep accepting it.
