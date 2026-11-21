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
| Signatures | next to it, one file per signing key | `<id>-<version>.dpplugin.<KEYID>.minisig`, `KEYID` the signing key's minisign key id as 16 uppercase hex characters in the byte order minisign itself prints it (`publisher::signature_file_name`). Each file: base64 minisign over exactly the package bytes, trusted comment exactly `deckpilot-publisher v1 role:release id:<id> version:<version> sha256:<hex> size:<bytes>`. A threshold of distinct pinned keys must verify ([below](#threshold-and-signature-files)). The single name `<package>.minisig` is not read or written (nothing was released with it) |
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
   exactly the release comment, each in its own file
   (`publisher::check_signatures`, [below](#threshold-and-signature-files));
   a valid signature by a `revokedKeys` key is refused even if other keys
   also signed.
4. The release is not revoked by the lock.
5. **The privileged-kinds gate** (below), then the package with the
   first-party reader rules (`verify_package`, `first_party`): archive
   limits, manifest rules, id and version, the static module check.
6. Compatibility with the host: `engines.deckpilot`, `engines.pluginApi`,
   `platforms` (`Manifest::check_compatibility`).
7. Rollback: never below the highest version `released-pins.json`
   records for the platform, and the same version only with the same
   files (the rule `check-released-pins.sh` enforces for source builds).

`deckpilot-registry release verify` runs it from the command line: it
reads every `<package>.*.minisig` next to the package (or in
`--signatures-dir DIR`), prints one line per file (`counted` with the key
id and fingerprint, or `ignored` with the reason), and enforces the
threshold of the pinned keys unless `--keys` names others (tests only; the
output says `not the pinned keys`). It appends the audit record with
`--audit`, and with `--unpack DIR` writes the verified files.

## Threshold and signature files

`first-party.json` pins up to four current keys and a `threshold` from 1
to their number; a document whose threshold is 0 or above the number of
current keys is invalid and nothing verifies (revoked keys never count
towards it). One key id must name exactly one pinned key, current or
revoked. The threshold is never lowered by anything at run time; the
owner raises it to 2 when the second production key is pinned.

`publisher::check_signatures` takes the signature files (key id from the
file name, bytes) and counts **each distinct, pinned, current key at most
once**. A file counts only if:

- its name's `KEYID` is a well-formed key id (16 characters `0-9A-F`);
- the key id inside the signature equals the file name's `KEYID` and is a
  pinned current key (it is checked against that one key only);
- no earlier file already counted for that key;
- it is at most `MAX_SIGNATURE_BYTES`, exactly the four minisign lines,
  and verifies over exactly the package bytes with exactly the release
  comment (id, version, sha256, size).

Everything else is ignored, never counted, and reported with its reason:
the same signature copied under a second key's name, two files of one key,
an unknown key, a name that does not match the signature's key id, a
wrong comment (other id, version or bytes), a malformed or oversized
file. A file named for a **revoked** key whose signature verifies refuses
the release outright (`RevokedKey`). Fewer than `threshold` counted keys
refuses it (`BelowThreshold`, fail closed, with every ignored file's
reason); more than eight files (`MAX_SIGNATURE_FILES`) refuses it too.

`release sign` signs with one key per invocation and writes that key's
file; the release workflow runs it once per key, in separate jobs, and
publishes all files next to the package. `release verify` is the combine
and check step.

The agent fetches exactly one file per pinned **current** key id
(`<package URL>.<KEYID>.minisig`, size-capped, from the same base as the
package) and never a revoked key's. A missing or unreadable file is
tolerated as long as the files found can still meet the threshold;
otherwise the install is refused before verification and audited.

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
2. The package (pinned digest) and one `<package>.<KEYID>.minisig` per
   pinned current key id (each at most `MAX_SIGNATURE_BYTES`) are
   downloaded from the registry's release location
   (`Registry::signature`, `PluginService::official_signatures`). Missing
   files are tolerated only while the threshold can still be met. Where
   they were hosted never counts.
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
   lock, unreadable floors), and too few signature files found or
   downloadable to meet the threshold.

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
are not affected. A retained version whose installed files are exactly the bundled copy
this agent pins (the bundled version kept for rollback after the update
to a registry release) is trusted like any bundled copy, so the rollback
to it goes through; a retained copy whose files differ is not run. The
reasons say which case it is: `revoked` (the lock, a revoked key) or
`not verified` (`invalidPackage`: no record, unreadable records, files
that differ from the pins).

**`floors.json` across versions.** Unknown fields are tolerated for
reading: a downgraded agent uses every floor and signer record it
understands, so its revocation checks and the copies it can vouch for keep
working. It never rewrites such a file, since that would drop what the
newer agent recorded, so official installs are refused until a newer
DeckPilot runs again. A different `floorsVersion` is refused entirely
(fail closed: the known fields may mean something else in a new format),
which quarantines registry copies of official plugins until it is fixed. A revocation therefore
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
3. Each release published with a threshold of
   `<package>.<KEYID>.minisig` files (one per signing key) next to the
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
keys through `DECKPILOT_TEST_PUBLISHER_KEYS`, feature `test-hooks` only;
three keys pinned with threshold 2: one file per key id fetched, a
missing third tolerated, one signature or a copy under another key's name
refused). The multi-signature rules themselves:
`crates/deckpilot-plugin-registry/tests/publisher.rs` (two valid keys, one
valid key, a signature duplicated under two names, two files of one key, a
wrong publisher key, wrong id, version or bytes, a revoked key with and
without a second valid key, a downgrade, missing files, a name and key id
mismatch, a threshold above the pinned keys, too many files, the CLI) and
`threshold_two_needs_two_distinct_pinned_keys` and
`a_revoked_key_never_counts_with_threshold_two` in the agent's tests
(registry metadata rewriting permissions or pinning other bytes still
refused with two valid signatures).

## Keys

Not created here. **Creating or rotating the production keys needs the
owner** (as for the registry keys, U-10). Two keys, threshold 2: a single
stolen key or secret cannot sign a release. The intended handling:

1. Offline, on two separate occasions (ideally two machines),
   `deckpilot-registry keys generate --role publisher --out
   publisher-a.key` and `... --out publisher-b.key`, each with its own
   password. The output prints the public key; its key id is the one
   minisign prints (`minisign public key <KEYID>` in the `.pub` file's
   comment line). Keep an offline backup of each encrypted secret key,
   apart from the other.
2. Store them as secrets of the dedicated environment `official-release`
   of `knittingcode/deckpilot-plugins` (release tags only, required
   reviewer), never in a repository, never in `knittingcode/deckpilot`
   (the release separation, [distribution.md](distribution.md), forbids
   plugin signing here and `check-release-separation.py` enforces it):

   | Secret | Content |
   | --- | --- |
   | `DECKPILOT_PUBLISHER_KEY_A` | the text of `publisher-a.key` (encrypted secret key) |
   | `DECKPILOT_PUBLISHER_KEY_A_PASSWORD` | its password |
   | `DECKPILOT_PUBLISHER_KEY_B` | the text of `publisher-b.key` |
   | `DECKPILOT_PUBLISHER_KEY_B_PASSWORD` | its password |

   The release workflow signs in two separate jobs, each with one key:
   `deckpilot-registry release sign --package <id>-<version>.dpplugin
   --id <id> --version <version> --key-env DECKPILOT_PUBLISHER_KEY_A
   --password-env DECKPILOT_PUBLISHER_KEY_A_PASSWORD` (and `_B`), so the
   secret never touches the disk and no job holds both keys. Each writes
   `<id>-<version>.dpplugin.<KEYID>.minisig`; a final job collects both,
   runs `release verify` against the pinned keys (threshold 2) and
   publishes the package with both files.
3. Pin both public keys and `"threshold": 2` in one reviewed pull request
   to `publisher/first-party.json` (and the plugin repository's copy). It
   reaches users with an application release; nothing at run time can
   change it. (Agent tests and `release verify --keys` use throwaway keys
   only.)
4. Rotation or compromise: move the key to `revokedKeys`, pin the new
   one, keep the threshold, and revoke the releases signed after the
   compromise in `artifacts.json`. Until an application release carries
   the new keys, builds simply cannot verify; no user installation trusts
   the key at run time.

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
