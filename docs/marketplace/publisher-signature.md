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
| Signature | next to it | `<id>-<version>.dpplugin.minisig` (one file per signing key; a threshold of distinct keys must verify, 1 at first): base64 minisign over the package bytes, trusted comment exactly `deckpilot-publisher v1 role:release id:<id> version:<version> sha256:<hex> size:<bytes>` |
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
or high-tier permission wait for the user's consent as before. On the
registry path ([below](#the-agent-installing-and-updating-official-plugins))
a verified release is installed through the store's ordinary install or
update with exactly the consent the user gave: a `deckpilot.*` registry
package is checked with the first-party rules and never receives the 1.9
privileged kinds.

**The privileged-kinds gate is closed.** A signed artifact that declares
any plugin API 1.9 privileged kind (`tools`, i.e. process execution;
`resources`; `projectContext`; `hostViews`; `openUrl.fromParams`;
`resource` parameters; `node.changesContext`) is refused with
`PublisherError::PrivilegedGateClosed`, whatever the signature. That
covers Docker, Git, Projects, Workflows and Workspace; GitHub, GitLab,
Jira and Notifications pass it (unit test
`the_privileged_kinds_gate_is_closed`). There is no flag, key or
configuration that opens it; the agent's copy of the gate
(`apps/agent/src/plugins/official.rs`, `PRIVILEGED_GATE`) is a compiled
constant, `Closed`. Opening it needs the independent review of
[publisher-review-request.md](publisher-review-request.md) and a code
change (the reader would apply `Manifest::validate_bundled` to such an
artifact, and the X12 tests would grow a "signed artifact" case).

## The agent: installing and updating official plugins

Status: **implemented, inert until a key is provisioned.** Every agent
build refuses every registry-delivered `deckpilot.*` package before any
download today, because `publisher/first-party.json` pins no key
(`this_build_cannot_verify_any_official_release`). Nothing changes for the
bundled copies: they are seeded offline from the application bundle and
trusted by the compiled pins exactly as before.

`apps/agent/src/plugins/official.rs` (`Official`), called from
`PluginService::fetch` for every `deckpilot.*` id (install, update and
dependency installs alike):

1. **Before any download** (`PluginService::pins`): the keys must be valid
   and provisioned, the compiled artifact lock valid and the floors file
   readable (`Official::ready`); otherwise the request is refused
   (`invalidPackage`). The registry chain then resolves the target as for
   any plugin: root-signed key set authorizing `deckpilot`, verified
   `deckpilot` publisher record, pinned size, sha256, manifest hash and
   permissions (`expected.first_party`).
2. The package (pinned digest) and `<package>.minisig` (at most
   `MAX_SIGNATURE_BYTES`) are downloaded from the registry's release
   location (`Registry::signature`). Where they were hosted never counts.
3. `Official::verify`: `publisher::verify_release` with the claim taken
   from the registry's pins (id, version, sha256, size), the pinned keys,
   the compiled lock's `revoked`, the running host
   (`HostEnvironment`: app version, plugin API 1.9, platform; a missing
   host refuses) and the **effective floor**: the highest of the floor this
   agent recorded for the plugin, the bundled pin it compiles in, and the
   highest version any DeckPilot release shipped on this platform
   (`plugins/first-party/released-pins.json`, compiled in), so a fresh
   install of a plugin that is not bundled has a floor too. Equal versions
   must have equal files, or the release is refused. The compiled artifact
   lock adds a minimum of its own: never below a locked release, and at its
   version only its package (`Compiled::check_locked`). A
   registry that pins other bytes than the publisher signed, a signature
   of another id or version, a revoked key, a revoked release, a lower
   version or the same version with other files are all refusals.
4. The package is verified once more against the registry pins with the
   first-party rules (`verify_package`), which yields the only type the
   store installs.
5. The release is recorded (its version and the keys that signed it, for
   every verified version, not only the highest) and the floor is raised,
   both saved (`<data dir>/plugin-official/floors.json`, atomically)
   **before** the store installs anything. A record that cannot be saved
   refuses the release. An interrupted install is retried with the same
   bytes and nothing older.
6. Every outcome is appended to `<data dir>/plugin-official/audit.jsonl`:
   `{"prev": <sha256 of the previous line>, "record": AuditRecord}`,
   hash-chained from `genesis` and across the rotations (above 1 MiB the
   log becomes `audit.jsonl.1`, older ones shift up to `audit.jsonl.5`,
   the oldest is dropped); the agent's log carries each line's hash
   (`official audit line`). Refusals before a verification are audited
   too: `pins` refusing because nothing can be verified (no key, an invalid
   lock, unreadable floors), and a signature that is missing or cannot be
   downloaded.

   **Limits of the audit log.** It is tamper-evident, not tamper-proof: an
   edited, removed or reordered line breaks the chain, but whoever can
   write the data directory (the user's own account, or malware running
   as the user) can rewrite or truncate the whole chain or delete the
   files; the last line is anchored only by its hash in the agent's log,
   which the same account can edit; the first line of the oldest kept
   generation chains to a file that was dropped; a write failure is
   logged and never refuses a release. It records what this agent
   decided, for diagnosis and review, and is not evidence against a local
   attacker. A signed or remote log would be needed for that.

Then the store's install or update runs exactly as for any registry
plugin: the user's consent only, a permission escalation staged until the
user approves it, trial load before activation, the crash-safe journal.

**Run-time revocation.** The installed **registry** copies of official
plugins are checked (`Official::revoked_installed`) at every start (with
or without a registry, offline included), after every registry refresh,
at every enable and every invocation (`check_verdict`), and for the
target of every rollback (`rollback` passes `official::Verdicts`, so a
rollback to a revoked version is refused). A version is not run if the
compiled lock revokes it, if the release this agent recorded for that
version was signed by a key the pinned keys now list in `revokedKeys`, if
this agent has no record of verifying that version at all (a deleted or
reset floors file: **fail closed**), or if the floors cannot be read. It
is quarantined like a registry `disable` verdict (`official::Verdicts` on
top of the registry's verdicts; without a registry, quarantine only,
nothing is lifted). Bundled copies are trusted by the compiled pins and
are not affected. A revocation therefore
reaches users with an application release (the lock and the keys) or
with the registry's own signed revocations, which apply as before.

**Bundled → Marketplace-managed.** The user's `updatePlugin` of a plugin
installed from the bundle fetches the official release as above and hands
it to the store's update with source `registry`:

- the plugin id is the same, so settings, credentials, storage, grants and
  every binding in profiles, layouts, projects and workflows stay; the
  update is refused if the new version would drop an action id, a setting
  or a credential slot the bundled version declares
  (`official::check_transition`);
- grants: kept where the new version declares them alike, narrowed where
  it narrows them, and anything new or broadened waits for the user's
  consent (the store's permission diff); nothing is derived or
  auto-granted, not even low tiers;
- the bundled copy stays active until the store commits the new one;
  every refusal (signature, floor, revocation, host, transition, consent)
  leaves it running. After a crash, recovery either completes the verified
  update or undoes it; the floor stays raised, so the retry needs the same
  bytes;
- idempotent: the same release again is "not newer" and changes nothing;
- offline: the managed copy keeps working from the store; seeding never
  replaces a copy from another source, so it is left alone, and a fresh or
  offline start without a registry still seeds the bundled copies;
- nothing uninstalls a bundled plugin: a user who removes the managed copy
  can restore the bundled one (`restoreBundledPlugin`), which the compiled
  pins vouch for whatever the floor.

**What turns it on (owner actions, none done).** Registry delivery of an
official plugin needs all of these. The first two are independent
authorities, and each is required:

1. A production publisher key pinned in `publisher/first-party.json`
   ([Keys](#keys)), shipped in an application release.
2. A production registry key set (root-signed, U-10/U-12) whose
   `first_party` field authorizes the `deckpilot` namespace, an index
   that lists `deckpilot` as a verified publisher, and the registry root
   keys embedded in the application (`DECKPILOT_REGISTRY_ROOT_PUBKEYS`,
   [agent-integration.md](agent-integration.md#trusted-root-keys)).
3. Each release published with its `<package>.minisig` next to the
   package in the plugin repository's release.

The exception to "the registry never serves a bundled id" (X12) is
exactly this path. No unsigned, third-party, file or development path
accepts a `deckpilot.*` id: the store refuses sideloaded and dev
installs of them (`InstallSource::allows`), the third-party reader rules
refuse the reserved publisher, and `PluginService::fetch` runs the
publisher verification for every registry download of a `deckpilot.*` id.

The plugins that declare privileged kinds (Docker, Git, Projects,
Workflows, Workspace) cannot make this transition while the gate is
closed, and the store refuses privileged kinds from any non-bundled source
(`check_privileged_source`) even if it were open.

**The gate's opening path** (`official::admit_privileged`): reachable only
through `PRIVILEGED_GATE = Open(&GateReview)`, a reviewed code change. It
applies only to an artifact whose size, sha256, signatures and revocations
already passed, and needs the review's `min_threshold` (at least 2) of
distinct keys that actually signed (counted again, and recorded as the
signers), the plugin named by the review, **every** privileged construct
of the manifest named by the review
(`deckpilot_plugin_store::privileged_constructs`: the privileged grant
keys and the `resource` parameters, `node.changesContext`, host variable
sources, host views, `statusItems` and `dashboardCards`), the registry's
manifest hash and permissions, then the bundled reader rules
(`verify_bundled_files`, `validate_bundled`), compatibility and the floor.
The store's backstop (`check_privileged_source`) refuses the same
constructs from every source but the bundle, whatever the manifest rules
said. The publisher verifier's gate itself refuses a manifest that does
not parse (`check_gate`).

**Derived grants are for bundled copies only.** The module migration
derives low and medium grants (§5.5 of module-migration.md) only onto a
copy installed from the bundle; a newer registry copy of the same
`deckpilot.*` id keeps exactly what the user granted, and the store
refuses `derive_grants` for any other source.
Tested behind a test-only gate (`the_opening_path_needs_its_review_conditions_and_meets_the_store_lock`),
which also shows the store's second lock.

Tests: `apps/agent/src/plugins/official/tests.rs` (tampered package,
swapped and replayed signatures, foreign key, registry permission and
namespace tampering, downgrade and same-version rebuilds, the bundled pin
as a floor, revoked keys and releases before and after install,
incompatible host, the closed gate and its opening path, the transition
keeping settings and grants, new privileges staged, a dropped action
refused, crash recovery at every update failpoint, offline start,
fail-closed floors, the hash chain), and
`apps/agent/tests/plugins_registry_https.rs`
(`first_party_plugins_need_the_root_authorization_and_a_verified_publisher`:
an unsigned bundled id refused over HTTPS, the signed one installed; test
keys through `DECKPILOT_TEST_PUBLISHER_KEYS`, feature `test-hooks` only).

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
