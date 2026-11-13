# Security policy

DeckPilot is pre-1.0 software under active development. It has not had an
independent security review and is not presented as production-ready; see
the known limitations in the [README](README.md) and the threat model in
[docs/security.md](docs/security.md).

## Supported versions

| Version                         | Security fixes |
| ------------------------------- | -------------- |
| `main`                          | yes            |
| the latest published release    | yes            |
| older releases and pre-releases | no             |

Before 1.0, fixes are made on `main` and shipped in the next release; there
are no backports. Draft releases are not supported versions.

## Reporting a vulnerability

Please report vulnerabilities **privately** through GitHub's private
vulnerability reporting: open the repository's **Security** tab and choose
**Report a vulnerability**
(<https://github.com/knittingcode/DeckPilot/security/advisories/new>).

Do not open a public issue, pull request or discussion for a suspected
vulnerability, and do not include exploit details in public places.

A useful report contains:

- the affected version or commit and the operating system;
- what an attacker can do, and what they need first (local access, a
  logged-in session, a malicious module or update server, …);
- steps to reproduce, or a proof of concept;
- any known workaround.

## What to expect

- An acknowledgement within 7 days. DeckPilot is maintained by one person,
  so this is a target, not a guarantee.
- The report is assessed in the private advisory; you are kept informed
  there and may be asked for more details.
- When a fix is ready, it is released and the advisory is published, with
  credit to you unless you prefer otherwise. A CVE is requested through
  GitHub where appropriate.

Please give a reasonable time to fix an issue before disclosing it.

## Scope

In scope: the DeckPilot application and its background agent, the
encrypted configuration database and its key handling, the IPC between app
and agent, the update mechanism (manifest, signatures, installation), the
built-in modules, the release workflows and signing in this repository.

Out of scope: vulnerabilities in third-party dependencies that do not
affect DeckPilot (report those upstream), attacks that require full control
of the user's logged-in session beyond what
[docs/security.md](docs/security.md) claims to protect, and missing
platform code signing that is already listed as a known limitation.
