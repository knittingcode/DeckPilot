# Contributing to DeckPilot

Thank you for your interest. DeckPilot is maintained by one person,
[@knittingcode](https://github.com/knittingcode); contributions are welcome,
but please read this first.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).
Security vulnerabilities are **not** reported in issues or pull requests;
see [SECURITY.md](SECURITY.md).

## Reporting bugs and proposing features

Use the [issue forms](https://github.com/knittingcode/DeckPilot/issues/new/choose):
**Bug report** for something that does not work as documented, **Feature
request** for an idea or a change in behavior. Include the DeckPilot version
or commit, the operating system and, for device problems, the Stream Deck
model. Security issues are never reported there; see
[SECURITY.md](SECURITY.md).

## Before you start

- For anything larger than a small fix, open an issue first and describe
  the problem or the proposal, so that we can agree on the approach before
  you spend time on it.
- Check [docs/roadmap.md](docs/roadmap.md) and the known limitations in the
  [README](README.md).
- Set up the development environment as described in
  [docs/development.md](docs/development.md).

## Pull requests

Contributions come as pull requests **from a fork**: only the maintainer has
write access to this repository.

1. Fork the repository and create a branch from `main`.
2. Keep the change focused: one topic per pull request.
3. Add or update tests for what you change, and the documentation in
   `docs/` when behavior changes.
4. Run the [local checks](#local-checks) before you push.
5. Open the pull request against `main` and fill in the template.

Do not add files that are specific to one tool or assistant (editor
configuration, AI assistant instructions and the like) and do not add
generated attribution lines to commits or pull request descriptions.

### Local checks

`make check` runs the formatting checks, lints, type checks and tests of
the Rust workspace and the frontend, and the version consistency check. CI also runs the storage tests against
SQLCipher:

```sh
make check
cargo test -p deckpilot-storage --features sqlcipher --locked
```

The full list of checks, and what each CI job runs, is in
[docs/development.md](docs/development.md#checks).

### Code style

Formatting and lint rules are configured in the repository and checked in
CI: [`rustfmt.toml`](rustfmt.toml) and [`clippy.toml`](clippy.toml) for
Rust, [`eslint.config.js`](eslint.config.js) and
[`.prettierrc.json`](.prettierrc.json) for the frontend, and
[`.editorconfig`](.editorconfig) for every file. Further conventions are in
[docs/development.md](docs/development.md#conventions).

Pull request titles follow
[Conventional Commits](https://www.conventionalcommits.org/), as in the
history: `fix(agent): …`, `feat(desktop): …`, `docs(development): …`. Pull
requests are squash merged, so the title becomes the commit message on
`main`.

### Security-sensitive changes

Changes to encryption and storage (`crates/deckpilot-storage`), the key
store (`crates/deckpilot-keystore`), the IPC between app and agent
(`crates/deckpilot-ipc`), the updater (`crates/deckpilot-update`), signing
and the release workflows protect users' configuration and the integrity
of what they install. A mistake there can expose credentials or let a
tampered update through, so describe the change and its security impact in
the pull request, add tests for the failure cases, and expect a slower,
more careful review. Workflows of pull requests from forks never receive
secrets: the signing keys and Apple credentials exist only as secrets of
the protected `production-release` environment, which accepts release tags
only.

### Required checks

`main` is protected by a ruleset: changes land only through a pull request,
with a linear history (squash merge), resolved review conversations, and
these checks passing on the up-to-date branch:

- `Frontend`
- `Rust (ubuntu-latest)`, `Rust (macos-latest)`, `Rust (windows-latest)`
- `Storage with SQLCipher`
- `Updater signing keys match` (uses no secret and runs for every pull
  request, including pull requests from forks)

Other workflows (compatibility, packaging, credential store) run when the
paths they cover change.

The first workflow run of a new contributor's pull request waits for the
maintainer's approval; this is a GitHub setting and not a judgement of the
change.

### Review and merging

Only the maintainer merges pull requests, by squash merge, after review.
DeckPilot has a single maintainer, so reviews are best effort and may take
a while; there is no guaranteed response time. The maintainer may ask for
changes, edit a pull request before merging, or close it if it does not
fit the project.

## Releases

Releases are made by the maintainer only, from version tags, by the release
workflow; see [docs/releasing.md](docs/releasing.md). Contributors do not
create tags or releases and do not change version numbers unless asked.

## License

No CLA or DCO sign-off is required; contributions are licensed under
Apache-2.0 (section 5 of the [license](LICENSE)), the license of this
project.
