# APT repository: the separate `.deb` variant (notes)

**Status: notes and a dry run, nothing built or published.** No APT
repository, repository key or host exists. Creating them, and adding the
variant to the release build, are separate, explicitly authorized steps
(docs/distribution/README.md, section 10).

## Why a separate variant

The GitHub `.deb` updates itself: it carries the updater key and installs
newer GitHub `.deb` files with `pkexec dpkg -i`. In an APT repository that
would put files under dpkg's control that the repository never published
and break `apt`'s view of the installed version (capability matrix,
2.10). So:

- **R7:** the GitHub `.deb` is never republished to the repository. The
  repository takes only `DeckPilot_<version>_amd64_apt.deb`, whose
  embedded distribution record says `apt`, verified from
  `release-meta.json`'s `artifacts` (`distribution_channel: apt`,
  `matrix_id: apt-deb`). The `distribute-apt` dry run refuses a release
  without it.
- **R1, R2:** the variant's provider is `external_package_manager`. It
  never constructs the Tauri updater, and `prepare`/`install` return
  `Unsupported`; the UI shows `sudo apt update && sudo apt upgrade`.
- **R9:** the channel is compiled in; nothing at run time (environment,
  settings, `/var/lib/dpkg`) can turn the variant into a self-updater.

## Build (stage A of the release, not implemented)

The variant must be byte-identical to what the release verified, so it is
built in the release workflow's Linux job and attached to the draft
(README.md, section 6.1), beside the GitHub packages:

| Setting | Value |
| --- | --- |
| `DECKPILOT_DISTRIBUTION_CHANNEL` | `apt` |
| `DECKPILOT_PACKAGE_FORMAT` | `deb` |
| `DECKPILOT_UPDATER_PUBKEY` | **unset** (`targets.toml`: `updater_key = false`) |
| `self-update` Cargo feature | compiled (`updater_compiled = true`), unreachable without the key and the GitHub provider |
| Bundler | `--bundles deb` only, without `tauri.release.conf.json`'s `createUpdaterArtifacts` (no `.sig`) |
| File name | `DeckPilot_<version>_amd64_apt.deb` (`targets.toml` `apt-deb`; `check-assets.sh` keeps it out of the update manifest) |
| Package `Version` | **the Debian form**, below |
| udev rule | installed, as in the GitHub `.deb` (`/usr/lib/udev/rules.d/70-deckpilot-streamdeck.rules`) |
| Agent, autostart | `/usr/bin/deckpilot-agent`, XDG autostart, as the GitHub `.deb` |

The agent and the application are rebuilt with the variables above; the
GitHub binaries cannot be reused, because the record is compiled in.

### Version: `X.Y.Z~kind.N`, not `X.Y.Z-kind.N`

The Tauri bundler writes the application version verbatim into the
control file (`tauri-bundler` 2.10.4, `src/bundle/linux/debian.rs`:
`Version: {settings.version_string()}`). For a pre-release that is
`0.6.0-alpha.3`, which dpkg reads as upstream `0.6.0` with Debian revision
`alpha.3`. Checked with `dpkg --compare-versions` (dpkg 1.22.22,
`debian:stable-slim`, no network):

| Comparison | dpkg |
| --- | --- |
| `0.6.0` vs `0.6.0-alpha.3` | `0.6.0 < 0.6.0-alpha.3` (wrong: the release sorts before its alpha) |
| `0.6.0` vs `0.6.0~alpha.3` | `0.6.0 > 0.6.0~alpha.3` |
| `0.6.0~alpha.3` vs `0.6.0~beta.1` | `<` |
| `0.6.0~rc.1` vs `0.6.0` | `<` |
| `0.6.0~alpha.10` vs `0.6.0~alpha.9` | `>` |

With the verbatim version, `apt upgrade` would never move a user from
`0.6.0-alpha.3` to `0.6.0`. The variant must carry `0.6.0~alpha.3`
(`tools/distribution/distlib.py`, `debian_version`), set for example by
rewriting the control file's `Version` after bundling or through a
bundler option if one exists; which is not decided. `generate.py apt`
refuses a package whose `Version` is not the Debian form.

## Repository (the `distribute-apt` job)

The dry run (`tools/distribution/generate.py apt`) writes the tree a
repository host would serve, unsigned:

```
pool/main/d/deck-pilot/DeckPilot_<version>_amd64_apt.deb
dists/<suite>/main/binary-amd64/Packages
dists/<suite>/Release
```

- Package name `deck-pilot` (the bundler's kebab-case of `DeckPilot`).
- Suites: `stable` for releases, `prerelease` for alpha, beta and rc.
  Users who want pre-releases add the `prerelease` suite; the app never
  chooses (README.md, section 1).
- `InRelease` and `Release.gpg` need the repository's GPG signing key and
  are not written. A real job would sign with a key held only in a
  `distribute-apt` environment, never in `production-release`.

What users would add (once a host exists; placeholders):

```sh
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://<apt host>/deckpilot.asc | sudo tee /etc/apt/keyrings/deckpilot.asc > /dev/null
echo "deb [signed-by=/etc/apt/keyrings/deckpilot.asc] https://<apt host> stable main" \
  | sudo tee /etc/apt/sources.list.d/deckpilot.list
sudo apt update && sudo apt install deck-pilot
```

## Still needed

- The release build of the variant (release workflow, `check-assets.sh`,
  `targets.toml` status `shipping`), with the Debian version.
- A repository host and its deploy credential; the repository GPG key
  (offline primary, signing subkey) and its published public key.
- An arm64 variant, later (`targets.toml` says x86_64 now).
- R8 documentation: a user who installs the GitHub `.deb` over the APT one
  has a self-updating install until `apt upgrade` replaces it.
