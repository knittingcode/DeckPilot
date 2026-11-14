#!/usr/bin/env python3
"""Signing secrets stay in the protected environment, in every workflow.

    tools/release-pipeline/check-secret-isolation.py [<workflows dir>]

Fails unless, in every workflow of the directory (default
.github/workflows):
  * no workflow is triggered by pull_request_target or workflow_run (they
    run with the repository's secrets for code or artifacts of a pull
    request);
  * a workflow triggered by pull requests or merge queue groups
    (pull_request, merge_group) references no signing secret (APPLE_*,
    TAURI_SIGNING_*) at all, so no pull request, from a fork or from this
    repository, gets a job that holds one;
  * every job that references a signing secret runs in the environment
    production-release (tags v* only, required reviewer).

The required check "Updater signing keys match" therefore needs no secret.
Repository-level copies of the secrets are no longer referenced by any
job outside the environment, so they can be deleted.
"""

import pathlib
import re
import sys

import yaml

ENVIRONMENT = "production-release"
SIGNING_SECRET = re.compile(r"secrets\.(APPLE_|TAURI_SIGNING_)")
FORBIDDEN_TRIGGERS = {"pull_request_target", "workflow_run"}
PULL_REQUEST_TRIGGERS = {"pull_request", "merge_group"}


def triggers(workflow):
    # YAML 1.1 reads the key `on` as True.
    on = workflow.get("on", workflow.get(True, {}))
    if isinstance(on, str):
        return {on}
    if isinstance(on, list):
        return set(on)
    return set(on or {})


def environment_name(job):
    environment = job.get("environment")
    if isinstance(environment, dict):
        return environment.get("name")
    return environment


def main():
    directory = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".github/workflows")
    files = sorted([*directory.glob("*.yml"), *directory.glob("*.yaml")])
    if not files:
        print(f"no workflows in {directory}")
        return 1
    problems = []
    for path in files:
        workflow = yaml.safe_load(path.read_text()) or {}
        on = triggers(workflow)
        for trigger in sorted(on & FORBIDDEN_TRIGGERS):
            problems.append(f"{path.name}: trigger {trigger} is not allowed")
        for name, job in (workflow.get("jobs") or {}).items():
            if not SIGNING_SECRET.search(yaml.safe_dump(job)):
                continue
            if on & PULL_REQUEST_TRIGGERS:
                problems.append(
                    f"{path.name}: job {name} uses signing secrets in a workflow that pull requests trigger"
                )
            if environment_name(job) != ENVIRONMENT:
                problems.append(f"{path.name}: job {name} uses signing secrets outside the environment '{ENVIRONMENT}'")
    for problem in problems:
        print(problem)
    if problems:
        return 1
    print(f"signing secrets only in the environment '{ENVIRONMENT}' ({len(files)} workflows)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
