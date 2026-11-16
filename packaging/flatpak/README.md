# Flatpak (Flathub) draft

**Status: draft, never built, not submitted.** `flatpak` and
`flatpak-builder` were not available where this was written, so nothing
here has been run; every Flatpak cell of the
[capability matrix](../../docs/distribution/capability-matrix.md) that
depends on it stays **Unverified**. Submitting to Flathub is a separate,
explicitly authorized step (docs/distribution/README.md, section 10).

| File | What |
| --- | --- |
| [`io.github.knittingcode.deckpilot.yml`](io.github.knittingcode.deckpilot.yml) | The manifest. `@TAG@` and `@COMMIT@` are filled from a verified release by `tools/distribution/generate.py flathub` (the `distribute-flathub` dry run) |
| [`io.github.knittingcode.deckpilot.desktop`](io.github.knittingcode.deckpilot.desktop) | Desktop entry, named after the app ID as Flathub requires |
| [`io.github.knittingcode.deckpilot.metainfo.xml`](io.github.knittingcode.deckpilot.metainfo.xml) | AppStream metadata (screenshots, releases and the OARS rating still missing) |
| [`udev.md`](udev.md) | The host udev rule users must install themselves |

## Application ID

`io.github.knittingcode.deckpilot`, **identical to the bundle identifier**
(`apps/desktop/src-tauri/tauri.conf.json` `identifier`,
`crates/deckpilot-ipc/src/paths.rs` `APP_IDENTIFIER`), which must not
change.

The mixed-case `io.github.knittingcode.DeckPilot` was considered and not
used:

- A Flatpak app ID is case-sensitive and names the desktop file, the
  AppStream ID, the icon, the D-Bus names the app may own and
  `~/.var/app/<app ID>`. A second spelling would give the Flatpak an
  identity that differs from every other channel only in case.
- Flathub verifies `io.github.<owner>.<repo>` IDs against the GitHub
  repository [S20]; the repository is `knittingcode/deckpilot`, lower case.
- The data directory inside the sandbox is named by `APP_IDENTIFIER`
  either way: `~/.var/app/io.github.knittingcode.deckpilot/data/io.github.knittingcode.deckpilot`.

Whether Flathub accepts the ID is decided at submission (Unverified).
Sources cited as [S..] are those of the capability matrix, section 8.

## Permissions (`finish-args`)

Each one is a reviewer decision. They match `targets.toml` (`flathub`).

| Permission | Why | Alternative |
| --- | --- | --- |
| `--device=all` | Stream Decks are `/dev/hidraw*`; `--device=usb` covers `/dev/bus/usb` only and Flatpak has no hidraw option. StreamController ships with it [S21] | None today |
| `--share=network` | Modules and plugins make HTTP requests | None |
| `--share=ipc`, `--socket=wayland`, `--socket=fallback-x11`, `--device=dri` | The WebKitGTK window | None |
| `--talk-name=org.kde.StatusNotifierWatcher` | The agent's tray icon [S22] | No tray in the Flatpak |
| `--talk-name=org.freedesktop.secrets` | The database key in the Secret Service (`crates/deckpilot-keystore`) | The Secret portal: a per-app secret, a new `KeyStore` backend [S23] |
| not granted: `--filesystem=xdg-data/io.github.knittingcode.deckpilot` | Would make data of a `.deb`/AppImage install visible | Export and import |

Autostart uses the Background portal (`RequestBackground` with
`autostart: true`), which needs no `finish-args`; the agent's autostart
strategy for Flathub is a stub that reports "unsupported" today (PR E).

## Build variant

Set in `build-options.env`, read by `crates/deckpilot-distribution` at
compile time: `DECKPILOT_DISTRIBUTION_CHANNEL=flathub`,
`DECKPILOT_PACKAGE_FORMAT=flatpak`. The application is built with
`--no-default-features`, so the `self-update` feature (the Tauri updater
plugin) is not compiled in, and no `DECKPILOT_UPDATER_PUBKEY` is set
(rule R1). Flathub updates through `flatpak update`; the provider is
`store_managed`.

## Offline sources

Flathub builds without network access [S16]. Before a submission, from the
release's commit:

```sh
# https://github.com/flatpak/flatpak-builder-tools
python3 flatpak-builder-tools/cargo/flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json
flatpak-node-generator pnpm pnpm-lock.yaml -o node-sources.json
```

## Open questions in the draft

- **Bundled first-party plugins.** The other packages carry them as a Tauri
  resource (`tauri.plugins.conf.json`, `bundled/`). Without the Tauri
  bundler, the Flatpak must install them where the agent looks for its
  resource directory; that path is not decided.
- **The plugin host** is not bundled in any package yet (audit section 9).
  Whether its Landlock and seccomp sandbox stacks inside Flatpak's is
  Unverified.
- **Runtime version** (`org.gnome.Platform` 49) and the `node24` extension
  must be checked against what Flathub offers at submission time.
- **The manifest has never been built.** It may need more modules (for
  example a newer `libayatana-appindicator` for the tray).

## How to verify (exact steps, not yet run)

On a Linux host with Flatpak (Fedora, Ubuntu 24.04 or later):

```sh
sudo apt install flatpak flatpak-builder        # or: sudo dnf install flatpak flatpak-builder
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user flathub org.gnome.Platform//49 org.gnome.Sdk//49 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08 org.freedesktop.Sdk.Extension.node24//25.08 \
  org.flatpak.Builder

# The manifest for a release (fills @TAG@ and @COMMIT@), next to the sources:
python3 tools/distribution/generate.py flathub --input input.json --out build/flathub
cp packaging/flatpak/*.desktop packaging/flatpak/*.xml build/flathub/   # referenced from the checkout, kept for the linter
# cargo-sources.json and node-sources.json as above, into build/flathub/

flatpak run org.flatpak.Builder --user --install --force-clean build/flatpak build/flathub/io.github.knittingcode.deckpilot.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest build/flathub/io.github.knittingcode.deckpilot.yml
flatpak run io.github.knittingcode.deckpilot
```

Then, with a Stream Deck attached and the host rule installed
([udev.md](udev.md)):

1. **hidraw with `--device=all`:** the device list shows the Stream Deck;
   `flatpak run --command=sh io.github.knittingcode.deckpilot -c 'ls -l /dev/hidraw*'`
   lists the nodes with the user's ACL. Without the rule, opening fails
   with `EACCES`.
2. **Tray:** the tray icon appears (GNOME needs the AppIndicator
   extension).
3. **Secret Service:** the agent starts and unlocks its database; a
   second start does not ask again.
4. **Background portal:** not implemented in the agent yet (PR E stub).
5. **Nested Landlock and seccomp:** once the plugin host is bundled, its
   sandbox report inside the Flatpak must be `full`.
