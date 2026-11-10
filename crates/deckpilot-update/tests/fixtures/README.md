# Update test fixtures

`test-key.pub` is the public half of a key pair generated only for these
tests with `tauri signer generate`. Its private key was discarded after
signing `artifact.bin` (producing `artifact.bin.sig`). This key is unrelated
to the release signing key and must never be used to sign releases.
