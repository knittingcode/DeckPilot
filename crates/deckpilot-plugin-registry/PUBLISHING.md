# Registry publishing tooling

> **Not live yet.** The registry repository
> [`knittingcode/deckpilot-plugins`](https://github.com/knittingcode/deckpilot-plugins)
> exists, with its workflows and its Pages site
> (<https://knittingcode.github.io/deckpilot-plugins/>). Its environments
> `registry-publish` and `registry-timestamp`, their secrets and every
> production key are still pending the owner's approval of the trust
> configuration, so no signed registry is published. This tool has been
> used only with throwaway keys in tests.

Registry metadata and plugin packages are published and signed only from
the registry repository, never with or alongside a DeckPilot application
release, and with keys and environments of their own: see the distribution
contract, [`docs/marketplace/distribution.md`](../../docs/marketplace/distribution.md).

`deckpilot-registry` is the offline half of sections 9 to 11 of the plugin
marketplace design (`docs/marketplace/architecture.md`, pull request #191):
it writes the static registry site that `RegistryClient` in this crate
verifies, and it checks the result with that same client before anything
is deployed. It has no network code. It does not upload releases, call the
GitHub API or deploy Pages; the workflows below do that around it.

```sh
cargo build -p deckpilot-plugin-registry --features publish --locked --release
target/release/deckpilot-registry help
```

The `publish` feature only adds the `publish` module and the binary
(dependencies `minisign` and `zeroize`). Without it the crate is the
verifier library and nothing else.

## Working directory

Every command works on plain files in `--dir` (default `.`). `site/v1/` is
exactly what Pages serves under `https://knittingcode.github.io/deckpilot-plugins/v1/`.

```text
roots.json                             embedded root role (public): the app's
                                       DECKPILOT_REGISTRY_ROOT_PUBKEYS
registry/publishers/<login>.json       publisher entries of the index
registry/plugins/<id>/plugin.json      listing and detail metadata (--plugin-detail)
registry/plugins/<id>/versions/<v>.json  version history, append-only
registry/revocations.json              cumulative revocations
releases/<id>--v<v>/<id>-<v>.dpplugin  release assets (GitHub Releases, not Pages)
site/v1/keys/<n>.json(.minisig)        root-signed key sets (signature array)
site/v1/index/<seq>.json(.minisig)     signed index snapshots (newest plus 10)
site/v1/plugins/<id>/<sha256>.json     content-addressed details
site/v1/timestamp.json(.minisig)       the only mutable document
```

Secret keys never belong in this directory.

## Commands

| Command | Does | Needs |
| --- | --- | --- |
| `keys generate --role root\|index\|timestamp --out FILE` | minisign key pair: `FILE` (mode 0600, scrypt-encrypted unless `--no-password`) and `FILE.pub` | password |
| `keyset init --root PUB… [--root-threshold N] --index PUB --timestamp PUB [--seq-floor N]` | `roots.json` and an unsigned `keys/1.json` | public keys only |
| `keyset rotate [--root PUB…] [--root-threshold N] [--index PUB] [--timestamp PUB] [--seq-floor N] [--seq-floor-reset]` | unsigned `keys/<n+1>.json`; unchanged roles are carried over; `seq_floor` defaults to the live seq | public keys only |
| `keyset sign --key FILE [--version N]` | adds one root signature; prints how many of the previous and new root thresholds are met | a root key |
| `package --plugin-dir DIR --module FILE.wasm --out FILE.dpplugin` | builds a package: `manifest.json`, `LICENSE` and the files the manifest names from `DIR`, the module as `plugin.wasm` (`docs/marketplace/artifact-contract.md`), deterministic and checked by every reader rule | — |
| `publisher --namespace NS --github-id N --login L [--display-name] [--verified]` | `registry/publishers/<login>.json` | — |
| `add-version --package F.dpplugin [--plugin-detail plugin.json] --source-repository O/N --tag T --commit SHA --builder V [--source-path P] [--attestation URL]` | verifies the package with `deckpilot-plugin-package` (`read_archive`, then `verify_package` against its own digests), takes sha256, size, manifest digest, permissions, engines and platforms from it, appends the version, stores the release asset | — |
| `yank --id ID --version V --reason R [--advisory URL]` | marks the version `yanked` in its detail and adds a `yank` revocation (the Settings notice) | — |
| `revoke --id ID --versions V,…\|* --action disable\|yank --reason R [--advisory URL]` | adds a revocation, or updates reason, advisory and date of an identical one; never removes or narrows | — |
| `lift-revocation --id ID [--versions …]` | removes revocations; refused unless a root-signed key set with `seq_floor_reset` has no index yet | — |
| `build-index --source-commit SHA [--min-app-version V] [--keep 10]` | details and an unsigned `index/<seq>.json`, `seq` = live seq + 1 (at least `seq_floor`); prunes to the newest 11 snapshots and the details they pin | — |
| `sign-index --key FILE [--seq N]` | `index/<seq>.json.minisig` with `deckpilot-registry v1 role:index seq:<seq> keys:<n>` | the index key |
| `timestamp --key FILE [--expires-in 7d]` | re-signs `timestamp.json` for the newest signed index | the timestamp key |
| `verify-site [--roots F] [--state DIR] [--min-remaining 3d] [--require-packages]` | runs the client end to end (see below) | — |

`--now <RFC 3339>` replaces the system clock in every command that records
a time.

### What each command refuses

- **Key sets**: roles sharing a key; an index or timestamp threshold other
  than 1 (those documents carry a single signature, so a higher threshold
  could never be met); a rotation that changes nothing; a `seq_floor`
  lower than the current one without `--seq-floor-reset`; a signature from
  a key that is not a root of the previous or the new key set. Every later
  command first walks the whole key set chain with the client's own
  `apply_keyset`, so nothing is built on a key set clients would refuse.
- **Versions**: an invalid package; a version not strictly greater than
  the latest recorded (append-only, section 11.3); another package for a
  recorded version (re-adding the identical package is a no-op); an
  unknown publisher namespace; a detail over 256 KiB.
- **Index**: an earlier index still unsigned; revocations of the live
  index that `registry/revocations.json` no longer covers at least as
  strongly (the client's `RevocationDropped` rule), except right after a
  `seq_floor_reset` key set; a document over the schema's limits.
- **Signatures**: a key that is not in the current key set's role; an
  index naming an older key set (a rotation needs a new index).
- **Timestamp**: validity over 14 days (the client's cap) or not positive;
  an index whose signature does not verify for the current key set; a seq
  below the live timestamp's (unless the key set resets the floor) or
  below `seq_floor`; a signing time earlier than the live timestamp's.

### `verify-site`

The CI gate before every deploy. It opens a `RegistryClient` with
`roots.json` (or `--roots`) and runs `refresh` over `site/v1` exactly as
the agent will: key set chain, timestamp, index. Then it requires `Fresh`
and at least `--min-remaining` before expiry, verifies every listed
detail with `verify_detail`, runs `resolve_install` for every offered
version (and checks withdrawn ones are refused), verifies each release
asset found under `releases/` with `verify_package` against the pins
(`--require-packages`: every one must be there), and checks that every
listed revocation is enforced by `installed_verdict`.

Without `--state` it is a fresh client (trust on first use). With `--state
DIR` it uses a `FileTrustStore`: run it once against the live site to get
the state of an existing client, then against the new site with that
state, and it proves that existing clients accept the update: no rollback,
no equivocation, no dropped revocation, a continuous key set chain. A
refused site never changes the stored state.

## Secrets

- Passwords come from `--password-stdin` (one line), `--password-env NAME`,
  or `DECKPILOT_REGISTRY_KEY_PASSWORD`. They are never printed, never put
  in error messages, and wiped from the tool's own memory after use.
- Secret keys are read from files only. Output names public key ids at
  most. `SigningKey` and `Password` redact themselves in `Debug`.
- Documents are deterministic (pretty JSON in struct order, final newline;
  same inputs, same bytes). Minisign signatures are randomized, so only
  the signature files differ between two runs.

## Production keys: key ceremony (U-10)

`keys generate` exists for local registries and tests. Production keys
are created in a key ceremony that the owner decides on (U-10; v1 plans
two root keys with threshold 1). At minimum:

1. On an offline machine, create the two root keys and the first index
   and timestamp keys, each with its own strong password.
2. `keyset init` with the public keys; `keyset sign` with one or both
   roots on that machine. Commit `roots.json` and `keys/1.json(.minisig)`.
3. Root secret keys stay offline (maintainer machine plus an offline
   backup) and never reach GitHub.
4. In `knittingcode/deckpilot-plugins`, the index key and its password
   become the `REGISTRY_INDEX_KEY(_PASSWORD)` secrets of the
   `registry-publish` environment (tags `*--v*`), and the timestamp key the
   `REGISTRY_TIMESTAMP_KEY(_PASSWORD)` secrets of both `registry-publish`
   and `registry-timestamp` (`main` only, the daily refresh). They are
   never secrets of `knittingcode/DeckPilot`.
5. `roots.json` becomes the DeckPilot repository variable
   `DECKPILOT_REGISTRY_ROOT_PUBKEYS`.

| Event (section 9.4) | Commands |
| --- | --- |
| Rotate the index or timestamp key | `keys generate` offline; `keyset rotate --index NEW.pub` (or `--timestamp`); `keyset sign` with a root; replace the environment secret; `build-index`, `sign-index` with the new key, `timestamp`; deploy key set, index and timestamp together |
| Index-key compromise | as above with `--seq-floor-reset` (`--seq-floor` the last legitimate seq); `lift-revocation` for revocations the attacker added; re-publish |
| Timestamp-key compromise | as above for `--timestamp`; impact was limited to freezing |
| Root rotation | `keyset rotate --root …`; `keyset sign` with the current roots and with the new roots |
| Root loss or compromise | app release with new embedded roots (not this tool) |

## Intended GitHub Actions flow (separate registry repository)

**Not set up; requires owner authorization to create the repository and
keys.** The design is section 11; the workflow sketch is
`experiments/marketplace-spikes/trust/publish-workflow.yml.example`. The
tool is built in each job from a pinned DeckPilot commit with `cargo
build -p deckpilot-plugin-registry --features publish --locked`.

### `validate.yml`: untrusted pull requests, no secrets

`pull_request` only (never `pull_request_target`), `permissions: {}`, no
environment, no secrets. Tooling from the base SHA, PR data read as data.
In a scratch copy of `main`'s `registry/` it replays the PR: the
reproducible build, then `add-version` with the built package (which
verifies it), then `build-index`. To run the client end to end without
any secret, it signs that scratch registry with throwaway keys made in
the job (`keys generate --no-password`, a fresh `keyset init`), runs
`sign-index`, `timestamp` and `verify-site --require-packages`, and
discards the keys and the output. The job summary shows the permission
diff and the computed sha256 and size. Its result is advisory (a PR can
edit the workflow); the `gate` job on `main` repeats it.

### `publish.yml`: release tags, protected environments

Runs for a release tag the owner creates (`<plugin-id>--v<version>`, or
`registry--v…` to re-sign the index only); tags are admin-only by ruleset.

| Job | Environment | Secrets | Tool |
| --- | --- | --- | --- |
| `resolve` | — | none | the tag names a reviewed commit on `main` and a recorded version |
| `build` (untrusted, `contents: read`) | — | none | rebuilds every recorded package from its pinned source; plugin build scripts run only here |
| `sign` | `registry-publish` | index and timestamp keys | builds nothing; `tools/publish-site.sh index` re-checks each package's sha256 and size against the reviewed pins, signs the index and the timestamp, and runs `verify-site` as an existing client |
| `release` (`contents: write`) | — | none | uploads the verified package to the GitHub Release of the tag (never creates a tag) |
| `deploy` | `github-pages` | none | the landing page plus the signed `site/v1` |
| `verify-published` | — | none | fetch the live site anonymously, `verify-site` on it |

Signing happens only in the protected environments; their secrets are
released only to jobs on `main` after review, never to pull requests or
forks. No job that runs author code has a secret, an environment or
`id-token: write`.

### `timestamp.yml`: daily

`schedule` plus `workflow_dispatch`, same concurrency group as
`publish.yml`: mirror the live site, `verify-site` (state of an existing
client), `timestamp --expires-in 7d` in `registry-timestamp`, `verify-site
--state … --min-remaining 6d`, deploy, `verify-published`. It never needs
the index key: the timestamp alone keeps clients fresh (this crate's
decision 1), and `index/<seq>.json` stays byte-for-byte the same.

## Not covered

- Fetching or deploying anything (mirroring the live site, GitHub
  Releases, Pages), `check-live`, `registry-keys.yml`.
- Validation of a PR's `registry/plugins/<id>/versions/<v>.json`
  submission against the built package: here the version record is
  derived from the package, so a submission file would be compared with
  `add-version`'s result by the workflow.
- Source archives (`source.archive`) and attestations are recorded only if
  given; nothing creates them.
- Publisher verification (`verified` is set by the maintainer).
