## What and why

<!-- What this pull request changes and why. Link the issue it addresses. -->

## How it was tested

<!-- Tests added or run, and manual checks (platform, device). -->

## Checklist

- [ ] One topic; the title follows Conventional Commits (`fix(agent): …`)
- [ ] `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, `pnpm test` pass
- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` pass
- [ ] Documentation in `docs/` updated where behavior changes
- [ ] No secrets, credentials or personal data in code, tests, logs or screenshots
- [ ] Security-relevant changes (encryption, keys, IPC, updates, release workflows) are called out above

<!-- Security vulnerabilities are reported privately, not in pull requests: see SECURITY.md. -->
