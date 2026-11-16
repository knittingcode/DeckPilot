# MSIX (full trust) and HID (hidapi)

Question ([capability matrix](../../../docs/distribution/capability-matrix.md),
section 2.4): does hidapi, which uses SetupDi and `CreateFile` rather than
the WinRT HID API that the `humaninterfacedevice` capability gates,
enumerate and open HID devices from inside a full-trust MSIX package as it
does unpackaged?

**Status: prepared, not run.** It runs as the manual job
`experiment-msix-hid` of [`distribute.yml`](../../../.github/workflows/distribute.yml)
(`gh workflow run distribute.yml -f experiment-msix-hid=true`), which can
only be dispatched once the workflow is on the default branch. The
capability stays **Unverified** until it has run, and even then a hosted
runner has no Stream Deck (and possibly no HID device at all), so a run
can show that enumeration behaves the same packaged and unpackaged, not
that a Stream Deck works.

What [`run.ps1`](run.ps1) does on a disposable Windows runner:

1. builds `src/main.rs` (hidapi 2.6.7, the agent's version) and runs it
   unpackaged (control, `unpackaged.txt`);
2. packs it with `makeappx` into a full-trust MSIX
   ([`AppxManifest.xml`](AppxManifest.xml): `runFullTrust`, an execution
   alias, an experiment identity, never DeckPilot's);
3. signs it with a throwaway self-signed certificate created on the
   runner, trusts that certificate in `LocalMachine\TrustedPeople`, and
   installs the package for the current user;
4. runs it through its execution alias, so it has package identity
   (`packaged.txt` reports `GetCurrentPackageFullName`);
5. removes the package and the certificate.

The reports are uploaded as the workflow artifact `experiment-msix-hid`.
Compare `hid_devices` and the `open` lines of the two reports.

Not covered: a Stream Deck, the Microsoft Store's own signing, the
AppContainer child from a packaged full-trust process (plugin host), and
the startup task.
