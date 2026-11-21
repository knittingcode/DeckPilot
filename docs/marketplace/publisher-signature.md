# First-party publisher signature

Status: **implemented, not provisioned** (Wave A of moving the bundled
plugins to `knittingcode/deckpilot-plugins`). No publisher key exists
yet, so nothing verifies and nothing changes in any build: every release
bundle is still built from `plugins/first-party/` and trusted by the pins
compiled into the agent ([module-migration.md §4.4](module-migration.md#44-who-signs-bundled-plugins)).
Security review request: [publisher-review-request.md](publisher-review-request.md).

## Why

The nine bundled plugins (`plugins/first-party/bundled.json`) are to be
built, packaged and released in `knittingcode/deckpilot-plugins`, one
GitHub Release per plugin version, and DeckPilot is to bundle those
release artifacts instead of a second copy of their sources. An
application build then needs to know, offline and without the registry,
that an artifact it downloaded was released by DeckPilot. The registry's
signatures do not answer that: no registry key exists (U-10, U-12), the
registry never serves the bundled ids (X12), and its index key is an
online key. The publisher signature is the narrow answer: one role, one
namespace (`deckpilot`), one kind of document (a release artifact).

It extends what exists and adds no format: the package is an ordinary
`.dpplugin` ([artifact-contract.md](artifact-contract.md)); the signature
is a minisign signature in the registry's format, checked with
`deckpilot_plugin_registry::sig` (exact trusted comments, distinct-key
thresholds); revocations reuse the registry's `RevokedVersions`; the
rollback floor is `released-pins.json`, whose guard already exists.

## Formats

| What | Where | Format |
| --- | --- | --- |
| Pinned public keys | `crates/deckpilot-plugin-registry/publisher/first-party.json`, compiled in (`publisher::PINNED_KEYS`) | `{"publisherKeysVersion": 1, "publisher": "deckpilot", "keys": [...], "threshold": n, "revokedKeys": [...]}`; keys are base64-wrapped minisign public keys; `keys` empty = not provisioned |
| Release artifact | GitHub Release `<id>--v<version>` of `knittingcode/deckpilot-plugins` | `<id>-<version>.dpplugin`, unchanged format |
| Signature | next to it | `<id>-<version>.dpplugin.minisig`: base64 minisign over the package bytes, trusted comment exactly `deckpilot-publisher v1 role:release id:<id> version:<version> sha256:<hex> size:<bytes>` |
| Artifact lock | `plugins/first-party/artifacts.json` | `{"artifactsVersion": 1, "plugins": [{"id", "version", "sha256", "size"}], "revoked": [{"id", "versions", "reason"}]}` (`publisher::ArtifactLock`); empty in Wave A |
| Audit log | the file `release verify --audit` names | JSON Lines, one `publisher::AuditRecord` per verification, accepted or refused |

The comment is domain-separated from the registry's
(`deckpilot-registry v1 role:`) and the updater's (`timestamp:…`), and
binds id, version, sha256 and size: a valid signature of another plugin,
another version or other bytes never passes.

## What a build checks (`publisher::verify_release`)

In this order; every failure is a refusal, there is no warning-only path:

1. The pinned keys are valid and provisioned. Never a key from the
   artifact, its manifest, its id or the repository it came from.
2. Size and sha256 of the downloaded bytes equal the reviewed claim (the
   lock entry), before anything parses them.
3. A threshold of distinct current keys signed exactly these bytes with
   exactly the release comment; a signature by a `revokedKeys` key is
   refused even if another key also signed.
4. The release is not revoked by the lock.
5. **The privileged-kinds gate** (below), then the package with the
   first-party reader rules (`verify_package`, `first_party`): archive
   limits, manifest rules, id and version, the static module check.
6. Compatibility with the host: `engines.deckpilot`, `engines.pluginApi`,
   `platforms` (`Manifest::check_compatibility`).
7. Rollback: never below the highest version `released-pins.json`
   records for the platform, and the same version only with the same
   files (the rule `check-released-pins.sh` enforces for source builds).

`deckpilot-registry release verify` runs it from the command line, with
the pinned keys unless `--keys` names others (tests; the output says so),
appends the audit record with `--audit`, and with `--unpack DIR` writes
the verified files.

## Authenticity is not authorization

A verified release says who released the bytes, nothing about what they
may do. Nothing in `VerifiedRelease` is a grant or a tier. Once staged as
a bundled plugin, the copy is trusted by the pins compiled into the agent
like every bundled plugin; seeding derives only low-tier grants;
credentials, private network, process execution and every other medium-
or high-tier permission wait for the user's consent as before. The
registry path is unchanged: the registry never serves the bundled ids, a
`deckpilot.*` registry package is checked with the first-party rules and
never receives the 1.9 privileged kinds.

**The privileged-kinds gate is closed.** A signed artifact that declares
any plugin API 1.9 privileged kind (`tools`, i.e. process execution;
`resources`; `projectContext`; `hostViews`; `openUrl.fromParams`;
`resource` parameters; `node.changesContext`) is refused with
`PublisherError::PrivilegedGateClosed`, whatever the signature. That
covers Docker, Git, Projects, Workflows and Workspace; GitHub, GitLab,
Jira and Notifications pass it (unit test
`the_privileged_kinds_gate_is_closed`). There is no flag, key or
configuration that opens it. Opening it needs the independent review of
[publisher-review-request.md](publisher-review-request.md) and a code
change (the reader would apply `Manifest::validate_bundled` to such an
artifact, and the X12 tests would grow a "signed artifact" case).

## Keys

Not created here. **Creating or rotating the production key needs the
owner** (as for the registry keys, U-10). The intended handling:

1. Offline, `deckpilot-registry keys generate --role publisher --out
   publisher.key` with a password. Keep an offline backup of the
   encrypted secret key.
2. Store the encrypted secret key and its password as secrets of a
   dedicated environment of `knittingcode/deckpilot-plugins` (proposed:
   `official-release`, release tags only, required reviewer), never in a
   repository, never in `knittingcode/deckpilot` (the release separation,
   [distribution.md](distribution.md), forbids plugin signing here and
   `check-release-separation.py` enforces it). The release workflow passes
   them to `release sign --key-env … --password-env …`, so the secret never
   touches the disk.
3. Pin the public key in a reviewed pull request to
   `publisher/first-party.json` (and the plugin repository's copy). It
   reaches users with an application release; nothing at run time can
   change it.
4. Rotation or compromise: move the key to `revokedKeys`, pin the new
   one, and revoke the releases signed after the compromise in
   `artifacts.json`. Until an application release carries the new keys,
   builds simply cannot verify; no user installation trusts the key at
   run time.

## Wave B: consuming an artifact

Not done in Wave A. The first plugin that moves (proposed:
`deckpilot.notifications`) needs, in one pull request each side:

- deckpilot-plugins: its source under `official/`, a release of
  `<id>--v<new patch version>` signed in `official-release`;
- here: its `artifacts.json` entry, `bundled.json` marking it
  `"source": "artifact"`, and `plugins/first-party/build.sh` downloading,
  verifying (`release verify` with `--released-pins` and `--audit`) and
  unpacking it into `bundled/<id>/` before the pins are written, then
  deleting `plugins/first-party/<name>/`.

The bytes of a plugin built in deckpilot-plugins differ from those
`build.sh` builds here for the same source (Cargo hashes the SDK's git
source into the module), so the first artifact of each plugin is a **new
patch version**; `check-released-pins.sh` and the rollback floor refuse
anything else.
