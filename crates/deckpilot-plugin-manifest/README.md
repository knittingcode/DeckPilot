# deckpilot-plugin-manifest

**Status: experimental.** Not used by DeckPilot yet. No app, agent or module
crate depends on it, and plugins stay disabled; nothing a user sees changes.

This crate is the manifest contract of the plugin marketplace design
(`docs/marketplace/architecture.md`, proposed in pull request #191),
sections 5.2 (permissions), 6.5 (versioning), 7 (manifest) and 8 (package
format). It provides:

- `schema/plugin-manifest.schema.json`: the manifest JSON Schema
  (`manifestVersion` 1), byte-identical to #191 at `aa7140b`.
- A typed serde model (`Manifest`) that refuses unknown fields everywhere
  and serializes back to the same JSON value it was read from.
- `Manifest::validate`: every schema constraint plus the cross-field rules
  of section 7. It reports all problems at once, each with a JSON path and
  a message you can show to a user.
- `Manifest::is_compatible(app_version, platform)` and
  `Manifest::check_compatibility(host)`, which also checks the plugin API
  level and explains every mismatch.
- `PackageIntegrity`: the package digests (registry `manifest_sha256`,
  package `sha256` and `size`, and the per-file hashes recorded at install),
  checked against the manifest. #191 keeps hashes out of the manifest
  because the manifest is itself inside the package.

## Tests

`tests/manifest.rs` checks the #191 example manifest and a fixture that uses
every field. Over 70 negative fixtures run against both the Rust model and
the bundled schema (through the `jsonschema` dev-dependency), so the two
cannot drift apart. A fixture marked `schema_rejects: false` is a rule the
schema cannot express, or a check stricter than the schema.

## Differences from the schema

The schema is the shape. These rules go beyond it (the agent is
authoritative, section 7):

- Cross-field rules 1-7, 9, 10 and 13 of section 7 are implemented. Rules
  8 (static bundle analysis), 11 (Public Suffix List), 12 (PNG decoding) and
  14 (comparison with the previous release) need inputs this crate does not
  have. `PackageIntegrity::validate` covers the file-existence part of
  rule 12.
- Host ports must be 1-65535. The schema pattern also accepts `:0` and
  `:99999`.
- A repository named `.` or `..` is refused. The schema pattern accepts it,
  though GitHub has no such repositories.
- `engines.deckpilot` with two comparators must be one lower and one upper
  bound (`=1.0.0 <2.0.0` and `>=1 >=2` are refused). A bare version means
  exactly that version, as in npm.
- Version ranges compare by semantic-version precedence, including for
  pre-release DeckPilot builds (`0.9.0-alpha.1` satisfies `>=0.8.0`).
  Cargo's `VersionReq` does not.
- Numbers written with a fraction (`20000.0`) are refused. JSON Schema
  counts them as integers.
- `manifest.json` is limited to 256 KiB. #191 sets no limit.
