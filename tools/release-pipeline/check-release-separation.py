#!/usr/bin/env python3
"""Application releases never ship plugin packages or registry metadata.

    tools/release-pipeline/check-release-separation.py [<workflows dir>]

The distribution contract (docs/marketplace/distribution.md):
knittingcode/DeckPilot builds and releases the application only. Plugin
packages and the signed registry metadata are published, and signed, by
the registry repository knittingcode/deckpilot-plugins, with its own keys
and its own environment (registry-publish), never from here.

Fails unless, in every workflow of the directory (default
.github/workflows):
  * no step publishes plugin packages or registry metadata: no release
    upload, release action, Pages upload or deployment, release asset API
    call or workflow artifact whose files name *.dpplugin, site/v1,
    roots.json, timestamp.json, index or key set documents or *.minisig;
  * no step runs a registry publishing or signing command (deckpilot-registry
    keys, keyset, package, add-version, build-index, sign-index, timestamp,
    yank, revoke, ...; verify-site and help only read), signs a plugin
    package (minisign or `tauri signer sign` on a .dpplugin), or uses a
    registry secret (secrets.REGISTRY_*) or a registry environment
    (registry-*), whatever environment the job runs in: registry and plugin
    signing happen only in the registry repository;
  * such a step, or a job holding one, never references the application's
    signing secrets (APPLE_*, TAURI_SIGNING_*) or the environment
    production-release (reported in addition, as the more serious mistake).

Running the registry crate's tests (cargo test -p deckpilot-plugin-registry
..., throwaway keys made inside the tests) is not a registry command and
stays allowed. So does a plugin host executable staged into the
application bundle (for example as a Tauri externalBin): it is a component
of the application installers, not an independently installable package.
"""

import pathlib
import re
import sys

import yaml

APP_ENVIRONMENT = "production-release"
APP_SECRET = re.compile(r"secrets\.(APPLE_|TAURI_SIGNING_)")

# Files of the plugin registry repository.
REGISTRY_FILE = re.compile(
    r"\.dpplugin\b"
    r"|(^|[\s\"'/=])site/v1\b"
    r"|\broots\.json\b"
    r"|\btimestamp\.json\b"
    r"|\bindex/[^\s\"']*\.json\b"
    r"|\bkeys/[^\s\"']*\.json\b"
    r"|\.minisig\b"
)

# Steps that publish files somewhere users or other workflows fetch them.
PUBLISHING_ACTION = re.compile(
    r"^(actions/upload-artifact|actions/upload-pages-artifact|actions/deploy-pages"
    r"|softprops/action-gh-release|ncipollo/release-action|svenstaro/upload-release-action"
    r"|actions/upload-release-asset|tauri-apps/tauri-action|peaceiris/actions-gh-pages"
    r"|JamesIves/github-pages-deploy-action)@"
)
PUBLISHING_COMMAND = re.compile(
    r"\bgh\s+release\s+(upload|create|edit)\b"
    r"|\bgh\s+api\b[^\n]*/releases/[^\n]*assets"
    r"|uploads\.github\.com"
    r"|\bgit\s+push\b[^\n]*\bgh-pages\b"
)

# deckpilot-registry subcommands that write or sign; verify-site and help only read.
REGISTRY_COMMAND = re.compile(
    r"\bdeckpilot-registry\b[^\n]*?\s(keys|keyset|package|publisher|add-version|yank|revoke"
    r"|lift-revocation|build-index|sign-index|timestamp)\b"
    r"|\bcargo\s+run\b[^\n]*deckpilot-plugin-registry[^\n]*\s--\s+(?!verify-site\b|help\b)\S"
)
PLUGIN_SIGNING = re.compile(r"\b(minisign|rsign|signer\s+sign)\b[^\n]*\.dpplugin\b")
REGISTRY_SECRET = re.compile(r"secrets\.REGISTRY_")
REGISTRY_ENVIRONMENT = re.compile(r"^registry-")


def environment_name(job):
    environment = job.get("environment")
    if isinstance(environment, dict):
        return environment.get("name")
    return environment


def step_text(step):
    """Everything a step runs or passes, as text."""
    return yaml.safe_dump({k: v for k, v in step.items() if k != "name"}, width=10**6)


def check(path, workflow):
    problems = []
    for job_name, job in (workflow.get("jobs") or {}).items():
        if not isinstance(job, dict):
            continue
        environment = environment_name(job) or ""
        job_env_text = yaml.safe_dump(job.get("env") or {}, width=10**6)
        app_job = environment == APP_ENVIRONMENT or bool(APP_SECRET.search(yaml.safe_dump(job, width=10**6)))
        where = f"{path.name}: job {job_name}"
        registry_job = False
        if REGISTRY_ENVIRONMENT.match(str(environment)):
            registry_job = True
            problems.append(
                f"{where} runs in the registry environment {environment}: registry signing happens only "
                "in knittingcode/deckpilot-plugins (environment registry-publish), never in this repository"
            )
        if REGISTRY_SECRET.search(job_env_text):
            registry_job = True
            problems.append(f"{where} uses a registry secret: registry keys never reach this repository")
        for index, step in enumerate(job.get("steps") or []):
            if not isinstance(step, dict):
                continue
            label = f"{where}, step {step.get('name') or step.get('uses') or index + 1}"
            text = step_text(step)
            publishes = bool(PUBLISHING_ACTION.match(str(step.get("uses", "")))) or bool(
                PUBLISHING_COMMAND.search(str(step.get("run", "")))
            )
            registry_step = False
            if publishes and REGISTRY_FILE.search(text):
                registry_step = True
                problems.append(
                    f"{label} publishes plugin packages or registry metadata: they are distributed by "
                    "knittingcode/deckpilot-plugins, never with an application release"
                )
            if REGISTRY_COMMAND.search(text):
                registry_step = True
                problems.append(
                    f"{label} runs a registry publishing or signing command: registry signing happens only "
                    "in knittingcode/deckpilot-plugins (environment registry-publish)"
                )
            if PLUGIN_SIGNING.search(text):
                registry_step = True
                problems.append(f"{label} signs a plugin package: plugin packages are signed only by the registry")
            if REGISTRY_SECRET.search(text):
                registry_step = True
                problems.append(f"{label} uses a registry secret: registry keys never reach this repository")
            if registry_step and APP_SECRET.search(text + job_env_text):
                problems.append(f"{label} gives the application signing secrets to registry or plugin signing")
            registry_job = registry_job or registry_step
        if registry_job and app_job:
            problems.append(
                f"{where} mixes registry or plugin publishing with the application's signing secrets or "
                f"the environment {APP_ENVIRONMENT}"
            )
    return problems


def main():
    directory = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".github/workflows")
    files = sorted([*directory.glob("*.yml"), *directory.glob("*.yaml")])
    if not files:
        print(f"no workflows in {directory}")
        return 1
    problems = []
    for path in files:
        problems += check(path, yaml.safe_load(path.read_text()) or {})
    for problem in problems:
        print(problem)
    if problems:
        return 1
    print(f"no workflow publishes or signs plugin packages or registry metadata ({len(files)} workflows)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
