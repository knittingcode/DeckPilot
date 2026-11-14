# Marketplace trust spikes: results

Spikes 4 (package signing and verification) and 5 (static registry trust)
for the plugin marketplace design in PR #191
(`docs/marketplace/architecture.md` on `docs/plugin-marketplace-design`,
sections 8, 9, 10, 11.5, 13.5 and 14, plus the `registry-*.schema.json`
schemas).

Standalone Cargo project, not part of the DeckPilot workspace. Nothing in
`crates/`, the root `Cargo.toml`/`Cargo.lock` or `.github/` was changed.
Every key is generated in memory by a test and dropped afterwards.

```sh
cd experiments/marketplace-spikes/trust
cargo test                               # 38 tests
cargo run --release --example measure    # numbers below
```

## Layout

| File | What it is |
| --- | --- |
| `src/sig.rs` | Exact trusted comments and role/threshold checks on top of `deckpilot_update::signature::verify`, which is reused unchanged (path dependency) |
| `src/signer.rs` | Signing on the registry side with throwaway minisign keys (`minisign` 0.10) |
| `src/registry.rs` | Key set, timestamp, index and detail documents as serde types, with `deny_unknown_fields` to match `additionalProperties: false` |
| `src/package.rs` | Spike 4: a deterministic `.dpplugin` builder, then verification in this order: size, sha256, optional publisher signature, limited unpack, manifest binding |
| `src/client.rs` | Spike 5: the client verifier (§9.3 steps 1 to 4), persisted trust state, freshness, install policy and verdicts for installed plugins |
| `tests/common/mod.rs` | An in-memory registry site with honest, tampered, replayed and offline modes |
| `tests/package.rs` | Spike 4 tests (11) |
| `tests/registry.rs` | Spike 5 tests (27) |
| `examples/measure.rs` | Measurements |
| `publish-workflow.yml.example` | Sketch of `publish.yml` in the registry repository |

## Spike 4: package signing and verification

Trust model, as #191 defines it: the registry does not sign packages
directly. The index key signs `index/<seq>.json`. The index pins the detail
file by sha256 and length, and the detail file pins the package by sha256
and size. The client accepts package bytes only when they match the values
it reached through that signed chain (`Client::resolve_install` →
`ExpectedPackage` → `package::verify`).

Publisher keys are deferred in #191 (U-2). They were tested as an optional
**extension**: a publisher key is pinned per namespace in the
reviewer-gated index, and a per-version minisign signature over the
package uses the trusted comment
`deckpilot-plugin v1 id:<id> version:<v> sha256:<hex>`.

What the tests prove:

- **Deterministic build.** The same inputs give byte-identical output,
  whatever the input order or the time. Checked properties: gzip header
  with mtime 0, no file name, XFL 2 and OS byte 255; ustar entries sorted by
  path with mtime 0, mode 0644, uid and gid 0, empty owner names; golden
  sha256 printed.
- **Build-time path rules from §8.** The builder rejects absolute paths,
  `..`, `.`, empty segments, characters outside the allowed set, paths over
  128 characters, duplicates that differ only in case, and more than 256
  entries.
- **Accepted:** a correct package through the full signed chain, both with
  and without a publisher key.
- **Rejected:**
  - Every single-bit change at every byte offset (2 flips per byte, done
    exhaustively).
  - Truncation to every length from 0 to n-1, extension, and a zeroed body
    of the same size.
  - A package for another plugin, or the previous version of the same
    plugin, served in place of the expected one.
  - A registry chain signed by a key that is not authorized. The install
    cannot even be resolved.
  - A pinned package whose own `manifest.json` names another id or version,
    or has another hash (defense in depth against a registry bug).
  - Publisher signature problems: a wrong key, a signature made for a
    different file, the right file with a comment naming another version or
    plugin, and a signature missing while a key is pinned.
- **Hostile archives that are pinned correctly** are stopped by the unpack
  limits: `../`, absolute paths, symlinks, directories, duplicates that
  differ only in case, a missing `plugin.js`, a gzip bomb (11 MiB unpacked
  against a 10 MiB limit), data that is not gzip, and pax long-name
  overrides.

Measurements (Apple M4 Pro, `--release`, median of 51 runs):

| Package | Size | size + sha256 | + publisher minisign | Full verify (sha256 + gunzip + untar + manifest) | Full + publisher signature |
| --- | --- | --- | --- | --- | --- |
| 4.9 MiB incompressible asset | 5,018,931 B | 8.8 ms | 12.2 ms | 10.4 ms | 14.2 ms |
| 2 MiB plugin.js + 3 MiB asset | 3,350,900 B | 5.9 ms | 8.2 ms | 7.7 ms | 10.3 ms |

A deterministic build of the 5 MiB package takes 66 to 73 ms. Verification
cost is negligible next to the download.

## Spike 5: static registry trust

The client follows §9.3:

1. **Key set chain.** `keys/<n+1>.json` must carry a threshold of
   signatures from distinct keys of the previous root role, and also from
   the new root role when the root role changes.
2. **Timestamp.** The signature over the raw bytes is checked first, then
   the JSON is parsed. Requirements:
   - the exact comment `role:timestamp seq:<s> keys:<kv>`;
   - JSON values equal to the values in the comment;
   - `keys_version` equal to the current key set;
   - `now < expires + 5 min`;
   - `seq` at least the stored seq;
   - at an equal seq, the same index reference.
3. **Index.** Fetched only when the reference changes. Checks, in order:
   - path `index/<seq>.json`;
   - length and sha256 before parsing;
   - index role and exact comment;
   - seq and `keys_version` equal to the timestamp's;
   - unexpired;
   - `download_prefix` as fixed by the schema;
   - revocations cumulative: every stored revocation is still present.
4. **Detail.** Fetched on demand, with a content-addressed path, length and
   sha256 checked against the index.

Size caps run before any parsing: key set 16 KiB, timestamp 4 KiB, index
4 MiB, detail 256 KiB.

The persisted state is one small JSON file, about 1.4 KB whatever the size
of the index: key set, seq floor, timestamp and revocations. The verified
index is cached in a separate file, and that cache is used only when its
sha256 equals the reference in the stored timestamp. The cache is written
before the state, so a crash between the two writes only causes a refetch.

Tests:

| Requirement | Result |
| --- | --- |
| Valid index | accepted; state persisted and reloaded identically; daily timestamp re-signing followed for 60 days |
| Expired timestamp | rejected (`TimestampExpired`); the 5 min skew is honored (+4 min ok, +6 min rejected) |
| Expired index (90 d) with a fresh timestamp | rejected (`IndexExpired`) |
| Rollback to a lower seq, after a restart | rejected (`Rollback`); in-memory and on-disk state unchanged |
| Clock set back to make an old timestamp valid | still a rollback (the seq floor is monotonic) |
| Two indexes signed for the same seq | rejected (`Equivocation`) |
| Index signed by a stranger, the timestamp key or a root key | rejected; timestamp signed by the index key rejected |
| Wrong comment (other seq, key set or role, Tauri updater format, non-canonical number) | rejected |
| One byte tampered in key set, timestamp or index | rejected |
| Index key rotation through a root-signed `keys/2.json` | accepted; the retired key is refused afterwards; a timestamp still naming key set 1 is refused |
| Key set signed by a key that is not a root | rejected |
| Root rotation | needs the old **and** the new root role; old roots are powerless afterwards |
| Threshold 2-of-2 (U-10 alternative) | one signature rejected; the same key twice rejected; two distinct keys accepted |
| Index-key compromise recovery | an inflated seq (1,000,000) is recovered with `seq_floor_reset`; without the reset it is a rollback; `seq_floor` acts as a floor |
| Foreign `download_prefix` in a validly signed index | rejected |
| Revocations | `disable` and `yank` (per version and `"*"`) and `yanked` in the detail are refused for install; installed versions get `Disable` / `RunWithNotice` / `Run`, also after a restart while offline and stale |
| Revocation dropped by a later index | rejected (`RevocationDropped`) |
| Tampered detail file | install refused |
| Tampered index cache on disk | not trusted; refetched online; revocations are still enforced offline from the state |
| Freeze attack (validly signed old timestamp replayed) | **not detectable until expiry** (`Unchanged`); after expiry + skew it is rejected, the state becomes `Stale` and installs stop; once the freeze is lifted the new seq and its revocation arrive |
| Oversized timestamp | rejected before parsing |

Offline behavior, as defined and tested (matching §14):

| Situation | `refresh` result | Installs and updates | Installed plugins |
| --- | --- | --- | --- |
| Never verified, offline | `Offline(NeverVerified)` | refused (`NeverVerified`) | n/a (sideloading only) |
| Offline, cached timestamp and index unexpired | `Offline(Fresh)` | allowed from the verified cache (the download still needs the network or a staged package, and is checked against the cached pins) | run |
| Offline, cached timestamp expired (> 7 d) or index expired | `Offline(Stale(..))` | refused (`Stale`), "registry stale" banner | run; stored revocations enforced |
| Back online | `Unchanged` or `Updated` | per the new freshness | per the new revocations |

A security error (as opposed to a network error) never replaces the stored
state. The client keeps the last good state, and its freshness decays
normally.

Refresh cost (release build): index of 100 / 1,000 / 3,000 plugins = 46 KB /
407 KB / 1.2 MB; first refresh 0.7 / 2.0 / 5.0 ms; unchanged refresh 0.2 to
0.5 ms. At about 400 bytes per plugin, the 4 MiB cap allows about 10,000
plugins.

## Publish workflow sketch

`publish-workflow.yml.example` follows the job table of §11.5:

- **Triggers:** only `push` to `main` and `workflow_dispatch`; no
  `pull_request` or `pull_request_target`.
- **Permissions:** `permissions: {}` at the top level.
- **Untrusted build:** the `build` job is the only one that runs author
  code, and it has no environment and no secrets.
- **Index key:** only in the reviewer-gated environment `registry-release`,
  which deploys from `main` only.
- **Timestamp key:** only in `registry-timestamp`, which deploys from
  `main` only and has no reviewer.
- **Key handling:** keys are written to a 0600 file in `$RUNNER_TEMP` and
  deleted in the same step.
- **Self-check:** the job verifies its own output with the client's own
  verifier (`deckpilot-registry`, built from a pinned DeckPilot commit)
  before deploying, and again against the live site afterwards.

Fork PRs cannot reach the keys, because environment secrets are released
only to `main` deployments after approval.

## Limitations

- **No real network.** `Source` is an in-memory map. Not covered: HTTP,
  redirects (https only), streaming abort at the declared size,
  `If-None-Match`, GitHub Pages caching, and release-asset redirect hosts.
- **Unauthenticated local state.** The state file is not authenticated. A
  local attacker who can write the data directory can reset the seq floor
  (rollback) or delete revocations. The product should keep it in the
  encrypted DB, as §12.4 plans. Losing the state falls back to the embedded
  roots (trust on first use), after which any validly signed, unexpired
  timestamp from the last 7 days is accepted.
- **Rust packer, not the Node builder.** Determinism is shown for the Rust
  packer (`tar` + `flate2`), not for the Node `@deckpilot/plugin-builder`
  of §8. Cross-tool and cross-OS reproducibility is spike S6. The gzip OS
  header byte must be pinned explicitly: it is 255 here, and Node and zlib
  may write 3 or another value by platform.
- **Schema not enforced.** Field-level JSON Schema patterns (id grammar,
  semver, URL regexes) are not re-validated by the client; only the
  trust-relevant fields are.
- **Rough measurements.** One machine (M4 Pro), wall-clock medians; no
  Windows or Linux numbers.
- **The workflow is a sketch.** Several `tools/*` scripts and the
  `deckpilot-registry` subcommands it calls do not exist, and some action
  SHAs are `<pin>` placeholders.

## Findings and open questions for the owner

1. **The index expires with no automatic re-sign (gap in #191).**
   `index.expires` is 90 days, but `timestamp.yml` re-signs only the
   timestamp. If nothing is published for 90 days, every client goes stale
   (tested: `IndexExpired`). Options:
   - (a) a scheduled re-index under `registry-release`, which needs a
     reviewer approval about every 60 days;
   - (b) a longer index expiry, since freshness comes from the timestamp
     anyway;
   - (c) no index expiry check while the timestamp is valid.

   Recommendation: (b) or (c), with alerts.
2. **Freeze window = timestamp expiry + skew (7 d + 5 min).** Within that
   window, a replayed timestamp cannot be told apart from "nothing new", so
   a revocation can be delayed by up to 7 days. Shorter expiry means faster
   revocation but more exposure to the 60-day schedule-disable rule and to
   outages. Is 7 days right, or should it be 2 to 3 days?
3. **Revocations cumulative, enforced by the client.** The spike refuses an
   index that drops a stored revocation. This protects against a
   compromised index key un-revoking a version, but it means a mistaken
   revocation can never be lifted (only superseded by a new version). §13.5
   implies that lifting is possible in principle. Which do you want?
4. **Key set deployment order.** After `keys/<n+1>.json` is accepted,
   timestamps naming key set n are refused. `keys/<n+1>`, the re-signed
   index and the timestamp must land in the same Pages deploy, or clients
   go stale until the next publish. `publish.yml` should enforce this.
5. **Root threshold (U-10).** 1-of-2 and 2-of-2 both work. With a single
   maintainer, 2-of-2 only adds protection if the two keys live on
   different media or in different places. Where are the root keys kept
   (offline laptop, encrypted USB, paper backup)? minisign keys cannot live
   on a hardware token.
6. **Index and timestamp thresholds.** #191 implies threshold 1. The code
   supports more, but a threshold above 1 on the unattended timestamp role
   has no benefit.
7. **Publisher keys (U-2).** They work and cost about 3.4 ms per 5 MiB.
   Adopting them needs schema changes (`publishers[].signing_key`,
   `versions[].publisher_signature`; both schemas currently forbid extra
   properties) and a key-loss or rotation story per publisher. They protect
   only against a registry compromise that leaves the publisher's own key
   intact.
8. **Signing tool in CI.** Feeding a password to the `minisign` CLI
   non-interactively is unverified. The sketch therefore signs with the
   registry's own Rust CLI (the same code as `src/signer.rs`). Decide
   whether CI signs with minisign, rsign2 or the in-house tool.
9. **seq_floor semantics.** The spike reads it as `stored = max(stored,
   seq_floor)` without a reset and `stored = seq_floor` with one. #191
   should state this explicitly.
