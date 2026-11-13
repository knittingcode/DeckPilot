# Contributing to DeckPilot

Thank you for your interest. DeckPilot is maintained by one person,
[@knittingcode](https://github.com/knittingcode); contributions are welcome,
but please read this first.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).
Security vulnerabilities are **not** reported in issues or pull requests;
see [SECURITY.md](SECURITY.md).

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
4. Run the same checks as CI before you push:

   ```sh
   pnpm install --frozen-lockfile
   pnpm format:check && pnpm lint && pnpm typecheck && pnpm test && pnpm build
   cargo fmt --all --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo test --workspace --locked
   ```

5. Open the pull request against `main` and fill in the template.

Commit messages and pull request titles follow the
[Conventional Commits](https://www.conventionalcommits.org/) style used in
the history, for example `fix(agent): …`, `feat(desktop): …`,
`docs(development): …`.

Do not add files that are specific to one tool or assistant (editor
configuration, AI assistant instructions and the like) and do not add
generated attribution lines to commits or pull request descriptions.

### Required checks

`main` is protected by a ruleset: changes land only through a pull request,
with a linear history (squash merge), resolved review conversations, and
these checks passing on the up-to-date branch:

- `Frontend`
- `Rust (ubuntu-latest)`, `Rust (macos-latest)`, `Rust (windows-latest)`
- `Storage with SQLCipher`
- `Updater signing keys match` (uses repository secrets, so it is skipped
  for pull requests from forks)

Other workflows (compatibility, packaging, credential store) run when the
paths they cover change.

The first workflow run of a new contributor's pull request waits for the
maintainer's approval; this is a GitHub setting and not a judgement of the
change.

### Review and merging

Only the maintainer merges pull requests, by squash merge, after review.
The maintainer may ask for changes, rebase or edit a pull request before
merging, or close it if it does not fit the project.

## Releases

Releases are made by the maintainer only, from version tags, by the release
workflow; see [docs/releasing.md](docs/releasing.md). Contributors do not
create tags or releases and do not change version numbers unless asked.

## License

By contributing you agree that your contributions are licensed under the
[Apache License, Version 2.0](LICENSE), the license of this project.
