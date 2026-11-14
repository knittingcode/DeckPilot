# Releasing

Releases are built by `.github/workflows/release.yml` when a release tag
is pushed (or when the workflow is started manually for an existing tag),
in two stages: stage A builds, signs and checks everything into a **draft**
release; stage B publishes it only after a maintainer approves it in the
`production-release` environment and every check has passed again
([release steps](#release-steps)).

## Release tags and the tag gate

Only these tags build a release; the trigger's `v*` is just a preliminary
filter:

| Tag | Example | Kind | GitHub release |
| --- | ------- | ---- | -------------- |
| `vX.Y.Z` | `v0.6.0` | stable | release |
| `vX.Y.Z-alpha.N` | `v0.6.0-alpha.1` | pre-release | pre-release |
| `vX.Y.Z-beta.N` | `v0.6.0-beta.2` | pre-release | pre-release |
| `vX.Y.Z-rc.N` | `v0.6.0-rc.1` | pre-release | pre-release |

Exactly the expression
`^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-(alpha|beta|rc)\.[1-9][0-9]*)?$`:
no leading zeros, `N` from 1, no build metadata, nothing normalized.
`testv0.6.0`, `v0.6.0-bettta.1`, `v0.6`, `v01.0.0`, `v1.0.0-beta.0`,
`v1.0.0-preview.1` and `v1.0.0-extra` are all refused.

The first job of the workflow, **Validate the release tag** (`validate-tag`,
`tools/release-pipeline/validate-tag.sh`), is the authoritative gate. With
read-only permissions (`contents: read`, `checks: read`) it checks, without
changing anything:

1. the tag against the expression above;
2. the commit the tag points to (annotated tags are dereferenced with
   `git rev-parse "$TAG^{commit}"`) is the checked-out, unmodified commit
   and is reachable from the protected `main` branch: a tag on a side
   branch or on an unmerged commit is refused;
3. every version declaration matches the tag:
   `deckpilot-dev check-versions --tag <tag>` (the workspace version in
   `Cargo.toml`, which the agent and every crate inherit, the workspace
   packages in `Cargo.lock`, `apps/desktop/package.json`,
   `apps/desktop/src-tauri/tauri.conf.json` — the version the updater
   artifacts are signed for — and a `CHANGELOG.md` section);
4. the required checks passed on **exactly** that SHA, using the checks API:
   for every required check name, the latest check run of that commit
   (highest ID) must have concluded `success`; a missing, running,
   cancelled, skipped or failed check refuses the tag, and runs of other
   commits never count. The required checks are the pinned list in
   `tools/release-pipeline/required-checks.txt` — `Frontend`, `Storage with
   SQLCipher`, `Rust (ubuntu-latest)`, `Rust (macos-latest)`,
   `Rust (windows-latest)`, `Updater signing keys match`, the required
   status checks of the *Protect main* ruleset — plus whatever the branch
   rules of `main` require when the release runs (read with
   `GET /repos/{owner}/{repo}/rules/branches/main`; if that cannot be read,
   the pinned list alone applies, with a warning). The *Signing check*
   workflow also runs on every push to `main` so that `Updater signing keys
   match` exists for the squash-merge commit that is tagged;
5. classification: `alpha`, `beta` and `rc` are pre-releases, a plain
   version is stable (`deckpilot-dev release-tag <tag>` must agree).

Its outputs `version`, `channel` (`stable` or `prerelease`), `sha`,
`prerelease` (`true`/`false`) and `msi_version` drive the rest of the
workflow. `msi_version` exists because Windows Installer accepts only
numeric versions (`0.6.0-alpha.1` fails the MSI build): alpha.N becomes
`X.Y.Z.N`, beta.N `X.Y.Z.(20000+N)`, rc.N `X.Y.Z.(40000+N)`, a stable
release `X.Y.Z`. Windows Installer compares only the first three fields
and DeckPilot's installer allows same-version upgrades, so the stable
release installs over its pre-releases. A version an MSI cannot carry
(major or minor above 255, patch above 65535, N above 19999) is refused by
the gate. Every later job builds `sha`, not whatever the tag points to by
then. Every other
job depends on `validate-tag` through `needs` (directly or transitively);
`tools/release-pipeline/check-workflow-gate.py` fails the *Release pipeline
tests* workflow if a job is added without it, if the gate gets write
permissions or if the workflow-level permissions grant write access. Only
the jobs that write to the draft release (draft, build, manifest, verify)
and the publishing job (publish, in the `production-release` environment,
behind the publication gate) have `contents: write`.

The tests (`tools/release-pipeline/test.sh`, run by
`.github/workflows/release-tests.yml` on pull requests that touch the
pipeline; not a required check) use a synthetic repository and mocked API
answers (`tools/release-pipeline/gh-stub.sh`); no real tag is created.

### The former `vX.Y.Z-N` test tags

`v0.5.0-1` and `v0.5.0-2` were signed draft test builds. Tags of that form
**no longer start a release**: the gate refuses them. The two existing
drafts stay as they are (the compatibility workflow still uses the
`v0.5.0-2` binaries). To test the pipeline end to end, push an `alpha` tag
of a commit on `main`: it builds the same signed, notarized draft, which is
never published unless a reviewer approves stage B. To build installers
without a release, use the packaging workflow instead.

## One-time provisioning

The repository does not contain any signing key. A maintainer creates the
updater key pair locally and stores it only in GitHub's encrypted settings:

```sh
mkdir -p ~/.config/deckpilot/signing && chmod 700 ~/.config/deckpilot/signing
pnpm --filter @deckpilot/desktop tauri signer generate -w ~/.config/deckpilot/signing/updater.key
```

1. Store the key and its password as repository secrets without echoing
   them (`gh` prompts for the password and stores it without a trailing
   newline):

   ```sh
   gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.config/deckpilot/signing/updater.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
   ```
2. Store the public key as the repository **variable**
   `DECKPILOT_UPDATER_PUBKEY`; it is compiled into release builds:

   ```sh
   gh variable set DECKPILOT_UPDATER_PUBKEY < ~/.config/deckpilot/signing/updater.key.pub
   ```
3. Keep an offline backup of the private key. Losing it means existing
   installations can no longer verify updates; publishing it lets anyone sign
   updates.

### macOS: Developer ID signing and notarization

Required for a macOS release. The workflow **refuses to build macOS
releases** without it, unless the repository variable
`DECKPILOT_ALLOW_UNSIGNED_MACOS` is set to `true` as a deliberate decision.

Why it matters beyond Gatekeeper: the background agent keeps the database
key in the Keychain, and the Keychain grants access to the agent's code
identity. An unsigned (or ad-hoc signed) agent gets a new identity with
every update, so macOS asks for the login keychain password after each
update (verified; see [security.md](security.md#system-key)). A Developer ID
signature keeps the identity stable across releases — together with the
agent's fixed signing identifier, `io.github.knittingcode.deckpilot.agent`,
which comes from an `Info.plist` embedded in the agent executable
(`apps/agent/Info.plist`; never change it). Without it, `codesign` would
derive the identifier from the build's UUID and change it in every release.

Credentials (Apple Developer Program; notarization with an App Store Connect
API key):

| Name | Kind | Content |
| ---- | ---- | ------- |
| `APPLE_CERTIFICATE` | secret | the *Developer ID Application* certificate with its private key, exported as `.p12`, base64-encoded |
| `APPLE_CERTIFICATE_PASSWORD` | secret | the `.p12` password |
| `APPLE_API_KEY` | secret | the App Store Connect API **private key** (`AuthKey_….p8` content, or its base64 encoding) |
| `APPLE_SIGNING_IDENTITY` | variable | `Developer ID Application: Name (TEAMID)` |
| `APPLE_TEAM_ID` | variable | the team identifier |
| `APPLE_API_KEY_ID` | variable | the API key's ID |
| `APPLE_API_ISSUER` | variable | the API key's issuer ID |

(The Tauri bundler's own `APPLE_API_KEY` environment variable means the key
*ID*; the workflow writes the private key to a file and passes it as
`APPLE_API_KEY_PATH`, with `APPLE_API_KEY_ID` under the bundler's name.)

What the release workflow does on macOS:

1. The Tauri bundler imports the certificate into a temporary keychain and
   signs inside out: the bundled agent, the app executable and the app
   bundle, each with the hardened runtime (`bundle.macOS.hardenedRuntime`)
   and **no entitlements** — DeckPilot needs none (USB HID access through
   IOKit, the login Keychain, Touch ID through LocalAuthentication and the
   system WebKit all work under the hardened runtime without them). The app
   uses the system WebKit, so there are no embedded frameworks.
2. The bundler submits the app to Apple's notary service, **waits for the
   result**, fails the build unless it is *Accepted* (printing Apple's log),
   and staples the ticket to the app. The updater archive (`.app.tar.gz`) is
   made from the stapled app and signed with the updater key as before.
3. The workflow notarizes the disk image the same way
   (`.github/scripts/notarize.sh`: `notarytool submit --wait`, anything but
   *Accepted* fails), staples it and replaces the uploaded image in the
   draft release.
4. `.github/scripts/verify-macos-app.sh` checks the app on disk, the app in
   the updater archive and the app in the disk image: a strict signature over
   the bundle, every executable signed by `APPLE_TEAM_ID` with the hardened
   runtime and without entitlements, the agent's fixed identifier,
   Gatekeeper's assessment (`spctl`) and the stapled ticket
   (`stapler validate`). If the bundler skipped notarization (for example
   because of incomplete credentials), this step fails the release.

The *Apple signing check* workflow (`apple-signing-check.yml`) checks the
credentials without a release: the certificate provides exactly the
configured identity of the configured team, a throwaway executable signs
with it (hardened runtime, secure timestamp), and Apple's notary service
accepts the API key. It runs in the `production-release` environment, so
only on a release tag and after a reviewer approves the deployment:
`gh workflow run "Apple signing check" --ref v<version>`. With
`-f notarize=true` it also submits that throwaway executable for
notarization; this records a submission in the developer account. On any
other ref the environment refuses the job before a secret is read.

### Windows: Authenticode (not yet provisioned)

Installers are not Authenticode-signed; SmartScreen warns on first start.
Signing needs a code-signing certificate (or a cloud signing service) and
`bundle.windows.signCommand` or `certificateThumbprint` in the Tauri
configuration. The updater signature does not depend on it. Windows has no
per-application access list on the Credential Manager, so unsigned updates
do not affect the system-managed database key.

The updater signature is independent of all platform signatures.

## Checking the signing configuration

Without building or publishing anything:

- **In CI, without secrets:** *Updater signing keys match*, the required
  check of the *Signing check* workflow (`.github/workflows/signing-check.yml`,
  every pull request, every push to `main`), uses no secret
  (`.github/scripts/updater-key-pair-fixtures.sh`). It checks that the
  release public key pinned in `.github/updater.pub` is a well-formed
  Ed25519 minisign key and that the repository variable
  `DECKPILOT_UPDATER_PUBKEY` (compiled into release builds) is that key
  (required on `main`; pull requests from forks may not see the variable),
  and that `.github/scripts/updater-key-pair.py` accepts and refuses the
  committed throwaway keys in `.github/scripts/fixtures`. It runs the same
  way for pull requests from this repository, from forks and from
  Dependabot. `tools/release-pipeline/test-updater-key-pair.sh` also tests
  the script with keys freshly made by the Tauri CLI.
- **In the release, with the real key:** the release workflow's `prepare`
  job, in the `production-release` environment, first runs
  `updater-key-pair.py --pinned .github/updater.pub --require-variable`:
  the secret key decrypts with its password, and its key ID and public key
  equal the pinned key and the variable. *wrong password or key* means the
  two secrets do not belong together. It runs before any toolchain,
  dependency or build step of the job, and the release stops there if it
  fails. A new release key means a new `.github/updater.pub` (reviewed like
  any change) together with the variable.
- **Secret isolation:** `tools/release-pipeline/check-secret-isolation.py`
  (run by the *Release pipeline tests*) refuses any workflow that pull
  requests or merge queue groups trigger and that references a signing
  secret, any job with a signing secret outside `production-release`, and
  the triggers `pull_request_target` and `workflow_run`.
- **Locally:** sign a throwaway file with your key (the password is read
  without echo and never appears in the command line) and verify it against
  the public key:

  ```sh
  printf 'test\n' > /tmp/deckpilot-signing-test.bin
  read -rs "TAURI_SIGNING_PRIVATE_KEY_PASSWORD?Password: " && export TAURI_SIGNING_PRIVATE_KEY_PASSWORD; echo
  pnpm --filter @deckpilot/desktop tauri signer sign -f ~/.config/deckpilot/signing/updater.key /tmp/deckpilot-signing-test.bin
  unset TAURI_SIGNING_PRIVATE_KEY_PASSWORD
  DECKPILOT_UPDATER_PUBKEY="$(cat ~/.config/deckpilot/signing/updater.key.pub)" \
    cargo run -p deckpilot-dev -- verify-signature /tmp/deckpilot-signing-test.bin /tmp/deckpilot-signing-test.bin.sig
  rm /tmp/deckpilot-signing-test.bin /tmp/deckpilot-signing-test.bin.sig
  ```

  (`read -rs "VAR?prompt"` is zsh; in bash use `read -rsp "Password: " VAR`.)

## Version management

The version in `[workspace.package]` of the root `Cargo.toml` is the one
authoritative source: every crate, including the agent, inherits it.
`apps/desktop/package.json`, `apps/desktop/src-tauri/tauri.conf.json` (the
version the updater artifacts are signed for) and the workspace's entries
in `Cargo.lock` repeat it; `deckpilot-dev set-version <version>` writes all
of them and `deckpilot-dev check-versions [--tag <tag>]` refuses any
difference (CI runs it; the tag gate runs it with the tag).

Notes go into `CHANGELOG.md`, either under `## [Unreleased]` or directly
into the section of the version being developed (`## [X.Y.Z] - Unreleased`).
A pre-release uses the notes of its `X.Y.Z` section.

`deckpilot-dev prepare-release <version>` prepares a release as a pull
request and **never creates a tag**:

1. refuses a malformed version (the tag grammar above, and the MSI limits),
   an existing tag, a version not newer than every existing `v*` tag (local
   or on `origin`), an existing branch, and uncommitted changes;
2. creates `release/prepare-v<version>` from `origin/main`;
3. moves the `## [Unreleased]` notes into the `X.Y.Z` section (a stable
   release dates it, `## [X.Y.Z] - YYYY-MM-DD`; a pre-release keeps
   `- Unreleased`) and refuses a release without notes;
4. `set-version <version>`, then `check-versions --tag v<version>`, as the
   tag gate will check it;
5. commits only the five version files, pushes the branch and opens the
   pull request (`gh pr create`). `--local` stops after the commit.

After the pull request is merged and the required checks have passed on
the merge commit, a maintainer tags **that commit** by hand and pushes the
tag; that push starts the release workflow.

## Release steps

1. `cargo run -p deckpilot-dev -- prepare-release <version>` (see
   [version management](#version-management)), review and merge the pull
   request.
2. Wait until the required checks have passed on the merge commit, then tag
   that commit:
   `git tag -s v0.6.0-alpha.1 <sha> -m "DeckPilot 0.6.0-alpha.1" && git push origin v0.6.0-alpha.1`.
3. **Stage A** (automatic, no user can see anything yet):
   - validates the tag (see [the tag gate](#release-tags-and-the-tag-gate));
   - checks that updater signing is configured, that `CHANGELOG.md` has
     notes for the version (they become the release notes and the notes
     shown by the in-app updater), and that macOS signing is configured (or
     explicitly waived);
   - runs the full installer checks of the *Packaging* workflow on the
     tagged commit, unsigned and without any secret (`packaging` job, which
     calls `.github/workflows/packaging.yml`): Linux `.deb` and `.rpm`,
     Windows NSIS and MSI, built with the release configuration and a
     throwaway updater key, checked by `tools/ci/check-packages.sh` (names,
     versions, application, bundled agent, udev rule, updater signatures).
     The draft, and with it every signed build and the publication, waits
     for them. The called workflow builds the run's commit, which the gate
     checked is the tag's (`RUN_SHA`): a tag moved after the push is
     refused. The job passes the gate's `prerelease` and `msi_version`
     (inputs `prerelease` and `msi-version`), so a pre-release builds no
     `.rpm` (RPM refuses `-` in versions) and its MSI gets the same numeric
     version as the signed build (WiX refuses `0.6.0-alpha.1`);
   - runs the real-binary compatibility matrix against the previous release
     (`tools/compat/matrix.sh`, baseline in
     `tools/release-pipeline/compat-baseline`): database migration and IPC
     in both directions, on the tagged commit;
   - creates the **one** draft release of the tag for exactly the gate's
     commit (`tools/release-pipeline/create-draft.sh`). It refuses a tag
     that already has a published release (a published release is never
     rebuilt, replaced or given new assets) and a draft built from another
     commit (a moved or reused tag); a draft of the same commit is reused
     by a re-run;
   - builds the agent and the app for Linux (x86_64), macOS (Apple silicon
     and Intel) and Windows (x86_64), bundling the agent as an external
     binary, and uploads to that draft (the builds never create a release);
   - signs updater artifacts; each build also hands its update-manifest
     entries (target keys, asset names, signatures) to a single job that
     assembles the manifest once, after all builds, and attaches it —
     `latest.json` for a stable release, `prerelease.json` for a
     pre-release, never both (the update channels, see
     [updates.md](updates.md#channels-and-discovery)): concurrent merges by
     the builds could lose entries. It fails if a platform or package type
     is missing, if two builds claim the same entry or if an asset is not in
     the draft (`tools/ci/assemble-latest-json.sh`, tested on every pull
     request with fake signed packages by `tools/ci/test-latest-json.sh`);
   - checks that the built app embeds the configured updater public key;
   - on macOS, verifies code signature, agent identity and notarization when
     signing is configured. The Developer ID certificate is put into the
     environment only right before the bundler runs and removed after it,
     so dependency installation scripts and the agent build never see it;
   - records what each build's installers actually carry, measured on the
     files (`codesign`/`stapler` on macOS, `Get-AuthenticodeSignature` on
     Windows, nothing on Linux), and fails if signing was configured but a
     macOS build is not signed or not notarized
     (`tools/release-pipeline/release-meta.sh`). The record becomes
     `release-meta.json` (tag, version, channel, commit, manifest, code
     signature and notarization per build) and a *Code signing* table in the
     release notes; a build without a platform signature is listed as
     **unsigned** — never claimed otherwise. Windows installers stay
     unsigned until Authenticode credentials are provisioned;
   - attests build provenance for every artifact
     (`gh attestation verify <file> --repo knittingcode/DeckPilot`);
   - downloads the draft, verifies the manifest and every signature with
     `deckpilot-dev verify-release` — including that each signature names
     the announced version, which installations require
     (`requireSignedVersion`, the protection against downgrade attacks) —
     refuses the other channel's manifest, checks that the manifest has an
     entry for every platform and package type (macOS app on both
     architectures, Linux AppImage, `.deb` and, for stable releases,
     `.rpm`, Windows MSI and NSIS), attaches `SHA256SUMS` and checks that
     every required asset is in the draft
     (`tools/release-pipeline/check-assets.sh --final`).
4. **Stage B** (after approval): the *Publication gate* job checks that the
   `production-release` environment exists and requires a reviewer (see
   below); the
   *Publish and verify* job then waits for a reviewer's approval in the
   workflow run. After approval it checks the draft again
   (`tools/release-pipeline/publish.sh`) — exactly one release for the tag,
   the tag still points to the built commit, every required asset and
   nothing of the other channel, `SHA256SUMS` covers exactly the assets and
   matches each of them, `release-meta.json` names this tag, version,
   channel, manifest and commit, every updater signature verifies, the
   manifest announces this version with an update for every platform and
   package type and points only to assets of this tag, the notes are not
   empty — and only then publishes it in **one** update (the repository
   has immutable releases enabled, so nothing of a published release can
   be changed afterwards; every asset is uploaded while it is a draft):
   alpha, beta and rc as a GitHub **pre-release** (never latest); a stable
   version as a release, marked **latest** only if it is newer than every
   published stable release (a fix of an older line is not). Finally it
   verifies the published release as users get it
   (`tools/release-pipeline/verify-published.sh`): every asset downloads
   from its public URL and matches `SHA256SUMS`, the channel's manifest URL
   (`releases/latest/download/latest.json` for the latest stable release,
   the release's own `prerelease.json` otherwise) announces the version and
   points only to verified assets, and the stable channel never announces a
   pre-release.

Nothing is published without the approval; a partial or unverified draft
is never published, and a pre-release never becomes the stable release.

### Publication gate and signing environment (repository settings)

The protected deployment environment is **`production-release`**. Three
jobs run in it:

- *Check configuration and release notes* (`prepare`) and the four
  *Build* jobs: the only jobs that read the signing secrets
  (`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`,
  `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_API_KEY`);
- *Publish and verify* (`publish`): the only job that publishes.

`tools/release-pipeline/check-workflow-gate.py` refuses any other job in
the environment and any job outside it that references a signing secret.
With required reviewers, a release therefore asks for approval three times:
before `prepare`, before the builds (one approval covers the four pending
build jobs), and before publication.

The workflow does not create or change the environment. GitHub creates a
named environment, **unprotected**, the first time a job refers to it, so
its mere existence proves nothing. Therefore the *Protected release
environment* job (`environment-gate`) runs right after the tag gate and
**before any job that names the environment or reads a secret**: it names
no environment, reads no secret, has `contents: read` only, and runs
`tools/release-pipeline/check-environment.sh production-release`, which
requires that

- the environment **exists** (a missing one is never created by this job);
- it has **at least one required reviewer**;
- its deployment policy allows **tags `v*` and nothing else**: *Selected
  branches and tags* (not *Protected branches*, not *No restriction*), with
  exactly one rule, of type tag, named `v*`.

Otherwise it fails and nothing else runs: `prepare`, the builds and
`publish` need it (directly or through `needs`), so no signing secret is
read and GitHub never gets to create the environment unprotected. A
release pushed before the owner has set the environment up therefore stops
at this job (re-run it once the environment is in place). The *Publication
gate* job runs the same check again right before publication, in case the
environment was changed while the release waited.
`tools/release-pipeline/check-workflow-gate.py` refuses a workflow in which
any job that uses the environment or a secret other than `GITHUB_TOKEN`
does not need `environment-gate` directly, or has an `if:` with `always()`,
`failure()` or `cancelled()` (including `!cancelled()`): such a condition
would run the job although the environment gate failed or was skipped, so
these jobs keep GitHub's implicit `success()` and fail closed. The cases (missing,
auto-created without rules, wait timer only, no tag restriction, branch or
extra rules, unreadable policies) are tested by
`tools/release-pipeline/test-publication.sh`. Until the secrets are moved (below), the
jobs keep reading the repository-level secrets — a job in an environment
sees repository secrets too, and environment secrets take precedence over
repository secrets of the same name.

Proposed configuration (Settings → Environments → New environment):

- name `production-release`;
- **Required reviewers**: the maintainers who may release (optionally
  *Prevent self-review*);
- **Deployment branches and tags**: *Selected branches and tags*, add the
  tag rule `v*` (the workflow runs for a tag);
- **Environment secrets**: see the migration below.

Equivalent API calls (an admin token; review before running):

```sh
gh api -X PUT repos/knittingcode/DeckPilot/environments/production-release \
  -F 'reviewers[][type]=User' -F "reviewers[][id]=$(gh api users/<maintainer> --jq .id)" \
  -F 'deployment_branch_policy[protected_branches]=false' \
  -F 'deployment_branch_policy[custom_branch_policies]=true'
gh api -X POST repos/knittingcode/DeckPilot/environments/production-release/deployment-branch-policies \
  -f name='v*' -f type=tag
```

### Moving the signing secrets into the environment

Done by the owner, by hand, in this order. No step prints a secret; values
are typed or piped from the offline copies, never echoed or logged.

1. **Create the environment** `production-release` as above (required
   reviewers, tag rule `v*`).
2. **Re-enter each secret in the environment** (the values cannot be read
   back from the repository secrets; use the offline copies):

   ```sh
   env=production-release
   gh secret set TAURI_SIGNING_PRIVATE_KEY --env "$env" < ~/.config/deckpilot/signing/updater.key
   gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --env "$env"   # prompts, no echo
   gh secret set APPLE_CERTIFICATE --env "$env" < <path to the base64 .p12>
   gh secret set APPLE_CERTIFICATE_PASSWORD --env "$env"           # prompts
   gh secret set APPLE_API_KEY --env "$env" < <path to AuthKey_<id>.p8>
   gh secret list --env "$env"    # names and dates only
   ```

   The variables (`DECKPILOT_UPDATER_PUBKEY`, `APPLE_SIGNING_IDENTITY`,
   `APPLE_TEAM_ID`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`,
   `DECKPILOT_ALLOW_UNSIGNED_MACOS`) are not secret and stay repository
   variables.
3. **Verify the protection**:

   ```sh
   gh api repos/knittingcode/DeckPilot/environments/production-release \
     --jq '{reviewers: [.protection_rules[] | select(.type == "required_reviewers") | .reviewers[].reviewer.login], branch_policy: .deployment_branch_policy}'
   gh api repos/knittingcode/DeckPilot/environments/production-release/deployment-branch-policies --jq '.branch_policies[] | "\(.type) \(.name)"'
   ```

   It must list at least one reviewer and only the tag rule `v*`.
4. **Run a non-publishing validation that uses the environment secrets.**
   Push an alpha tag of a `main` commit (or dispatch the release workflow
   for an existing alpha tag) and let **stage A only** run: approve
   `prepare` and the builds, which now read the environment secrets;
   check that the draft is complete (signed updater artifacts, notarized
   macOS builds, `release-meta.json`); then **reject** the *Publish and
   verify* deployment, or let it wait, so nothing is published. Discard
   the draft afterwards (`tools/release-pipeline/discard-draft.sh <tag>`).
5. **Only then, with the owner's explicit approval, remove the
   repository-level secrets** (`gh secret delete <name>` for each of the
   five). No workflow needs them any more: the required check *Updater
   signing keys match* uses no secret, and every job that reads a signing
   secret — the release's `prepare` and `build`, the *Apple signing check*
   and *Update continuity* — runs in `production-release`
   (`tools/release-pipeline/check-secret-isolation.py`). While the
   repository-level copies exist, a workflow on any branch that someone
   with write access pushes can still reference them; deleting them is
   what makes them unreachable outside the environment.

### Failure recovery

| What failed | State | What to do |
| ----------- | ----- | ---------- |
| The tag gate | nothing created | Fix the cause (wait for CI, tag a commit on `main`, fix the version) and push a **new** tag. A pushed `v*` tag cannot be deleted or moved: the *Release tag protection* ruleset blocks it and has no bypass actors ([release-readiness.md](release-readiness.md)). Leave the refused tag; only the owner can remove one, by disabling the ruleset temporarily, and only if no release exists for it |
| Stage A (a build, signing, notarization, manifest, checksums) | a draft, possibly incomplete | Re-run the failed jobs (`gh run rerun <run-id> --failed`): the draft of the same commit is reused and its assets replaced. For a clean retry, discard the draft and run again: `tools/release-pipeline/discard-draft.sh <tag>` (refuses published releases; keeps the tag), then `gh workflow run release.yml --ref <tag>` |
| Protected release environment (`environment-gate`) | nothing created, no secret read | Set up `production-release` (above: reviewers, tag rule `v*` only), then `gh run rerun <run-id> --failed` |
| Publication gate | a complete draft | Create or protect the `production-release` environment (above), then `gh run rerun <run-id> --failed` |
| Stage B before publishing | a complete draft, unpublished | Re-run the failed jobs, or `gh workflow run release.yml --ref <tag> -f publish_only=true` (skips stage A, publishes the existing draft after the same checks). Every check runs again; nothing is published until all pass |
| Stage B after publishing (verification of the public release) | published | Never delete a published release. Find the cause (for example an asset removed by hand), fix it without replacing published assets, and re-run: for an already published release of the same commit `publish.sh` changes nothing and only verifies again. If the release itself is wrong, publish a new version |

All of this is tested without GitHub by `tools/release-pipeline/test-publication.sh`
(releases kept by `gh-stub.sh`, public downloads served by `curl-stub.sh`):
draft creation and tag reuse, failed signing and notarization, missing
artifacts and manifests, channel isolation, the publication gate,
pre-release versus stable and latest, every check that keeps a draft
unpublished, idempotent re-runs and recovery.

### The first pre-release, command by command

Once: create the protected `production-release` environment
([publication gate and signing environment](#publication-gate-and-signing-environment-repository-settings)).
Then, from an up-to-date, clean checkout of `main`:

```sh
cargo run -p deckpilot-dev -- prepare-release 0.6.0-alpha.1   # branch, commit, PR; no tag
gh pr checks --watch <pr-number>                                # the owner approves and merges the PR
git fetch origin
sha=$(git rev-parse origin/main)                                # the merge commit
gh api "repos/knittingcode/DeckPilot/commits/$sha/check-runs" \
  --jq '.check_runs[] | "\(.name): \(.status) \(.conclusion)"'  # required checks passed?
git tag -s v0.6.0-alpha.1 "$sha" -m "DeckPilot 0.6.0-alpha.1"
git push origin v0.6.0-alpha.1                                  # starts stage A
gh run watch "$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
# Approve "prepare" and the builds (production-release), review the draft,
# then approve "Publish and verify" in the run (stage B).
```

### Workflow hardening

- **Runs on tags only.** A push of a `v*` tag, or a manual run *on the
  tag* (`gh workflow run release.yml --ref <tag>`). Started on a branch,
  the gate refuses it (`RELEASE_REF` must be `refs/tags/<tag>`) before any
  job that holds a secret, and the environment's deployment rule allows
  tags `v*` only.
- **Pinned actions.** Every action is pinned to a commit SHA (with its
  version as a comment); update the SHA, not the comment, when upgrading.
- **No foreign caches next to secrets.** The jobs in `production-release`
  restore no cache (no Rust cache, no pnpm store cache): caches are written
  by other workflows, pull requests included, and must not feed a signed
  build. Release builds are therefore slower.
- **Timeouts** on every job.
- The Tauri bundler, which signs, runs the frontend build of the tagged
  commit's locked dependencies in the build job; that is inherent to
  signing during bundling. The lockfile is frozen and the commit is
  reviewed and on `main`.

`tools/release-pipeline/check-workflow-gate.py` enforces the timeouts, the
pins and the absence of caches in the environment's jobs, next to the
dependency and permission rules.

### Dependency on the packaging workflow

The builds still run in this workflow's `build` job. A reusable packaging
workflow (`workflow_call`) is being added to CI separately; once it exists,
the `build` job can call it with the draft's release ID and the gate's
outputs, keeping every job behind `validate-tag`.

## Status

The updater signing key pair and the Apple Developer ID and notarization
credentials are provisioned in the `production-release` environment (and,
until the owner deletes them, as repository secrets); the release's
`prepare` job and the *Apple signing check* verify them on a release tag. Signing and
notarization ran end to end for the draft test releases `v0.5.0-1` and
`v0.5.0-2` (see [release-validation.md](release-validation.md)). Windows
code signing is not provisioned. The two-stage publication (stage B) has
not run against GitHub yet: no release has been published. The first release should be treated as a test of the
pipeline, and the first update between two published releases as the test of
installation (see [updates.md](updates.md)).
