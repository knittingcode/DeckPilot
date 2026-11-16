# Downstream distribution tools (dry runs)

Used by [`.github/workflows/distribute.yml`](../../.github/workflows/distribute.yml)
(docs/distribution/README.md, sections 6.3 and 6.4). Nothing here
publishes: the tools read a published release and write files to review.
Standard library Python only.

| File | What |
| --- | --- |
| `verify-release-input.sh <tag> <channel> <out>` | The common first step: the release is published, immutable, pre-release flag as the tag says; the tag points at `release-meta.json`'s `commit`; the channel's assets match `SHA256SUMS` and `release-meta.json`'s `artifacts` (hash, size, `distribution_channel`). Writes `<out>/input.json`. Only API reads and downloads |
| `generate.py <channel> --input <input.json> --out <dir>` | The channel's files: Homebrew cask, WinGet manifests, APT repository tree (unsigned), Flatpak manifest, Mac App Store build plan, MSIX manifest and MSI listing, each with a `summary.json` of what blocks a real publication |
| `distlib.py` | Which artifacts each channel takes, version mappings (`release_channel`, the Debian `~` form) |
| `templates/` | Draft Mac App Store entitlements and the MSIX `AppxManifest.xml` |
| `test.sh <work dir>` | Runs both against a stubbed `gh` serving a release shaped like `v0.6.0-alpha.3`, whose `artifacts` are computed by `tools/release-pipeline/release-artifacts.py` from `docs/distribution/targets.toml`, and compares the outputs with `tests/expected/` (`UPDATE_EXPECTED=1` rewrites them). Also checks every refusal: draft, mutable, wrong pre-release flag, moved tag, tampered asset, missing `artifacts`, a GitHub build offered as another channel's, the GitHub `.deb` for APT (R7), and a non-Debian version |

The real `v0.6.0-alpha.3` release predates `artifacts` in
`release-meta.json`, so `verify-release-input.sh` refuses it with that
reason; the first release built with PR D's metadata is the first one the
dry runs can take.
