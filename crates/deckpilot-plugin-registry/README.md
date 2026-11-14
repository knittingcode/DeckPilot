# deckpilot-plugin-registry

**Status: experimental.** Not used by DeckPilot yet. No app, agent or module
crate depends on it, and plugins stay disabled; nothing a user sees changes.

Client-side verification of the signed static plugin registry: sections 9,
10, 12.3 steps 1 and 2, 13.5 and 14 of the plugin marketplace design
(`docs/marketplace/architecture.md`, proposed in pull request #191). It
promotes the registry half of `experiments/marketplace-spikes/trust`
(spike 5) and reuses `deckpilot_update::signature::verify` unchanged.

There is no network code. Documents go in as bytes, either through the
step-by-step API or through a `RegistrySource` the agent's fetcher will
implement; state goes to a `TrustStore`.

## API

- `RegistryClient::open(embedded_roots, store, Policy)`.
- `refresh(&source, now)`: key set chain, timestamp, index; an unavailable
  source gives `Refresh::Offline(freshness)`.
- Step by step: `apply_keyset(bytes, signatures)`,
  `verify_timestamp(bytes, signature, now) -> VerifiedTimestamp`,
  `needs_index`, `commit(timestamp, Some((index, signature)), now)`,
  `note_clock(now)`.
- `freshness(now)`, `can_install(now)`, `index()` (browsing from the cache,
  whatever the freshness), `detail_ref(id)`, `verify_detail(id, bytes)`.
- `resolve_install(id, version, detail_bytes, host, now) -> InstallTarget`
  with the URL, the consent permissions and the
  `deckpilot_plugin_package::ExpectedPackage` the download must match.
- `installed_verdict(id, version)`: `Run`, `RunWithNotice` (yanked) or
  `Disable` (revoked), from the stored revocations, also offline.
- `TrustStore` with `FileTrustStore` (atomic replace: temporary file in the
  same directory, fsync, rename, directory fsync) and `MemoryTrustStore`.
- `sig`: exact trusted comments and threshold role verification.
- `schemas`: the four #191 registry schemas.

## Verification

- **Key sets.** `keys/<n+1>.json` needs a threshold of signatures from
  distinct keys of the current root role (the embedded roots for `n = 0`)
  with the comment `deckpilot-registry v1 role:keys version:<n+1>`, and
  also from the new root role when it changes. Keys are compared by their
  Ed25519 key, so one key written twice counts once (the spike compared
  strings). A key set whose roles share a key is refused, so the
  unattended timestamp key can never act as the index key or a root.
- **Timestamp.** At most 4 KiB. The signature is verified over the raw
  bytes first, with the exact comment `role:timestamp seq:<s> keys:<kv>` in
  canonical form; then the content must equal the comment, name the current
  key set, have `expires - signed_at` at most 14 days, be unexpired
  (`now < expires + 5 min`), have `seq` at least the stored floor and, at an
  equal seq, the same index reference. A replayed older signature of the
  same seq never shortens the stored expiry.
- **Index.** Only fetched when the reference changes. Path
  `index/<seq>.json`; length and sha256 against the timestamp before
  parsing; the index role and exact comment; seq and key set equal to the
  timestamp's; the fixed `download_prefix`; every detail path
  content-addressed (`plugins/<id>/<sha256>.json`); revocations cumulative.
  Index, seq, revocations and timestamp are stored together (index cache
  first, then the state).
- **Detail.** Length and sha256 against the index before parsing; id and
  publisher equal to the entry; versions strictly increasing; package URLs
  under `<prefix><id>--v<version>/`; `latest` naming published, non-yanked
  versions; permissions that parse as manifest permissions.
- **Signatures.** Stricter than minisign: exactly the four minisign lines,
  so nothing unsigned can follow a signature. Domain separation: every
  registry comment starts with `deckpilot-registry v1 role:` and must match
  exactly, so a Tauri updater signature or a signature of another role,
  seq or key set never passes; registry signatures in turn fail the
  updater's `verify_release_artifact` (no `version:` field).
- **Everything else in the schemas** (lengths, patterns, ranges,
  uniqueness) is enforced by each document's `validate`, and the tests run
  the same documents through the bundled JSON Schemas.

## Decisions on the spike's findings

1. **Index expiry gap.** `index.expires` is 90 days in #191 and nothing
   re-signs the index between publishes. This crate lets the timestamp
   alone govern freshness: by default `index.expires` is only checked to be
   after `generated_at`. `Policy::enforce_index_expiry` restores the strict
   reading for a registry that re-indexes on a schedule. Rationale: the
   timestamp is re-signed daily and pins the index by hash; an old index
   cannot come back because seq is monotonic; enforcing the expiry would
   need a reviewer-approved re-index every 60 to 90 days or every client
   goes stale. #191 should state this (or drop `expires` from the index).
2. **Freeze window.** Still timestamp expiry plus skew (7 days and 5
   minutes with #191's schedule). The client now also refuses any
   timestamp valid for more than 14 days, so a stolen timestamp key cannot
   sign a long-lived timestamp to stretch the freeze.
3. **Lifting revocations.** Revocations are cumulative: a new index must
   keep revoking every stored (plugin, version) at least as strongly
   (`disable` covers `yank`, `"*"` is only covered by `"*"`). Reason,
   advisory and date may change; scope and strength may not shrink. The
   only way to lift one is a root-signed key set with `seq_floor_reset:
   true`: the next index verified after it may drop revocations, once.
   That is the index-key-compromise recovery of section 9.4 (an attacker
   with the index key could also have added bogus revocations), and the
   same offline, deliberate root ceremony serves to correct a mistaken
   revocation. Installed versions are re-enabled once such an index is
   verified, as section 13.5 says. No schema change is needed.
4. **Key set deployment order.** Unchanged: after `keys/<n+1>.json` is
   accepted, timestamps naming key set `n` are refused, so the key set,
   the re-signed index and the timestamp must deploy together.
5. **`seq_floor`.** Without a reset, `stored = max(stored, seq_floor)`.
   With `seq_floor_reset`, `stored = seq_floor`, the stored timestamp is
   dropped (it belongs to the compromised era; the client is
   `NeverVerified` until the next timestamp) and the next index may lift
   revocations.
6. **Root replacement by app release.** The state records a fingerprint of
   the embedded roots. If a new app build embeds other roots, the stored
   key set chain and timestamp are dropped and the chain is walked again
   from `keys/1.json` under the new roots; the seq floor and the
   revocations are kept.
7. **Clock resets.** The state keeps a clock floor: the newest verified
   `signed_at`, raised by the local clock but never above the stored
   timestamp's expiry plus skew. Freshness and expiry use `max(now,
   floor)`, so setting the clock back cannot make an expired timestamp
   fresh again, and a clock far in the future only makes the client stale
   until its next successful refresh. Rollback stays prevented by the seq
   floor.
8. **Unauthenticated local state** (spike limitation) remains: a local
   attacker who can write the store can reset it. A missing state starts
   from the embedded roots (trust on first use); a corrupt or unknown-format
   state is an error, never a silent reset. The product should keep the
   state in the encrypted database (section 12.4).

Publisher keys (U-2) were an optional extension in the spike and are not
part of #191's schemas; they are not implemented here.

## Offline

| Situation | `refresh` | Installs and updates | Installed plugins |
| --- | --- | --- | --- |
| Never verified | `Offline(NeverVerified)` | refused | sideloading only |
| Stored timestamp unexpired | `Offline(Fresh)` | allowed from the verified cache | run |
| Stored timestamp expired | `Offline(Stale(TimestampExpired))` | refused | run; revocations enforced |
| Index cache lost or tampered | `Offline(Stale(IndexUnavailable))` | refused | run; revocations enforced |

A security error never replaces the stored state.

## Schemas

`schema/registry-*.schema.json` are byte-identical to
`docs/marketplace/schemas/` on `docs/plugin-marketplace-design` at
`a55c021`. No defect was found, so none was changed. They refer to the
manifest schema by `$id`; the tests resolve it to
`deckpilot_plugin_manifest::SCHEMA_JSON`.

## Tests

`cargo test -p deckpilot-plugin-registry`: 58 tests (1 unit, 50 in
`tests/registry.rs`, 7 in `tests/schemas.rs`), all keys generated per test
and dropped. Every spike 5 test is ported; added: every single-bit change of
the key set, timestamp, index and detail (two bits per byte) and of the
signature files (only untrusted-comment changes may pass), truncations,
comment variants (other seq, key set or role, trailing space, `v2`, Tauri
updater format, empty), registry signatures against the updater verifier,
replayed key sets, distinct-key thresholds, shared role keys, expiry policy,
overlong validity, clock set back and far ahead, replayed older timestamps,
weakened, narrowed and moved revocations, revocation lifting through a
reset, root replacement by app release, corrupt state, failed saves,
inconsistent details, host compatibility, an install end to end through
`deckpilot-plugin-package`, and schema agreement for every published
document plus about 70 invalid variants refused by both the schema and the
model.
