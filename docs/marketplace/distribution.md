# Distribution contract: application and plugins

Status: **decided** for the separation; the registry's keys and
environment are **not created yet** (they wait on the owner's approval of
the trust configuration, U-10 and U-12). Applies to the experimental plugin
marketplace; nothing here changes what an application release contains
today.

DeckPilot ships through two channels that never share a release, a key, a
secret or an environment.

| | Application | Plugins |
| --- | --- | --- |
| Repository | `knittingcode/DeckPilot` | `knittingcode/deckpilot-plugins` |
| What it holds | application source and application releases | the static plugin registry and the independently distributed plugin packages |
| Served from | GitHub Releases of `knittingcode/DeckPilot` | registry metadata: GitHub Pages, <https://knittingcode.github.io/deckpilot-plugins/v1/>; packages: GitHub Releases of `knittingcode/deckpilot-plugins` |
| Signed with | the application updater key (minisign, `TAURI_SIGNING_PRIVATE_KEY`) and the Apple Developer ID | the registry roles: root keys (offline), index key and timestamp key (`crates/deckpilot-plugin-registry/PUBLISHING.md`) |
| Signing environment | `production-release` | `registry-publish`, in `knittingcode/deckpilot-plugins` only |
| Trusted because | the updater signature, against the public key built into the app | the registry signatures, against the root keys built into the app (`DECKPILOT_REGISTRY_ROOT_PUBKEYS`) |
| Lifecycle | application versions, tags `v*` of this repository | plugin versions and registry snapshots, on their own schedule |

## `knittingcode/DeckPilot`: the application only

An application release (`.github/workflows/release.yml`,
[docs/releasing.md](../releasing.md)) contains exactly:

- the installers: `.dmg`, `.AppImage`, `.deb`, `.rpm` (stable releases),
  `.msi`, NSIS `-setup.exe`;
- the updater archives (`DeckPilot_<arch>.app.tar.gz` and the packages
  above) with their updater signatures (`.sig`);
- the update manifest (`latest.json`, or `prerelease.json` for a
  pre-release);
- `release-meta.json` (commit, channel, signing record), the release notes
  and `SHA256SUMS`.

Nothing else. In particular never a plugin package (`*.dpplugin`), never
registry metadata (`roots.json`, key sets, index snapshots,
`timestamp.json`, details, `*.minisig`) and never a plugin build output.
The update manifest names, for each platform, that platform's application
updater package only.

The plugin host (`deckpilot-plugin-host`) is a component of the
application: when it is bundled, it ships *inside* the installers (like the
agent), signed and updated with the application. It is never a release
asset of its own and never an independently installable package.

Registry and plugin signing never happen in this repository's workflows,
in any environment: no `deckpilot-registry` signing or publishing command,
no registry secret, no `registry-*` environment, and never with the
application's signing secrets or the `production-release` environment.
This repository's CI only *tests* the registry tooling, with throwaway keys
made inside the tests.

## `knittingcode/deckpilot-plugins`: the registry and the packages

- **GitHub Pages** serves the signed registry metadata under
  <https://knittingcode.github.io/deckpilot-plugins/v1/> (`site/v1/` of
  `PUBLISHING.md`). The agent trusts it **only through its signatures**,
  verified against the root keys built into the application, never because
  of where it is hosted: a compromised or impersonated Pages site can at
  most withhold updates (which the timestamp's expiry turns into a visible
  `Stale` state), not change what clients accept.
- **GitHub Releases** of that repository hold the signed plugin packages
  (`<id>-<version>.dpplugin`), pinned by sha256 and size in the signed
  index; one release per plugin version.
- Its workflows sign in its own environment, `registry-publish`, with its
  own keys. That environment and the production registry keys do not exist
  yet: creating them waits on the owner's approval of the trust
  configuration (U-10 key ceremony, U-12). Until then the tooling has been
  used only with throwaway keys in tests.

## Independent lifecycles, separate keys

- A plugin is published, yanked or revoked without an application release,
  and an application release never waits for, contains or re-signs a
  plugin.
- The application updater key never signs registry metadata or a plugin
  package, and no registry key ever signs an application update. Losing or
  rotating one never affects the other.
- No secret and no environment is shared between the two repositories'
  release workflows.
- The application embeds only *public* registry keys (the root role); a
  root rotation that clients must learn of is the one case that needs an
  application release (`PUBLISHING.md`, "Root loss or compromise").

## Enforcement in this repository

Run by *Release pipeline tests* (`.github/workflows/release-tests.yml`,
job `gate`) on every pull request that touches a workflow or
`tools/release-pipeline/` (not a required check):

- `tools/release-pipeline/check-assets.sh` (stage A's last check and stage
  B's first): the release's assets must be exactly the application's
  allowlist above; a plugin package or registry file is refused by name,
  anything else as "not an application asset".
- `tools/release-pipeline/publish.sh` and `verify-published.sh`: each
  platform's update in the manifest must be that platform's application
  updater package of this tag; a URL to a plugin package, registry
  metadata, a signature, a checksum file or another platform's package is
  refused. Tests: `tools/release-pipeline/test-publication.sh`.
- `tools/release-pipeline/check-release-separation.py`: no workflow
  publishes plugin packages or registry metadata, runs a registry signing
  or publishing command, signs a plugin package, or uses a registry secret
  or environment, and none mixes them with the application's signing
  secrets or `production-release`. Tests:
  `tools/release-pipeline/test-release-separation.sh` with the fixtures in
  `tools/release-pipeline/fixtures/release-separation/`.
