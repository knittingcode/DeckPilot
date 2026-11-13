#!/usr/bin/env python3
"""Structural check of the release workflow's gate.

    tools/release-pipeline/check-workflow-gate.py [<workflow.yml>]

Fails unless:
  * a job `validate-tag` exists, needs nothing, and may only read
    (contents, checks, actions);
  * every other job reaches `validate-tag` through `needs` (directly or
    through other jobs) -- an `if:` condition does not count;
  * the workflow-level permissions grant no write access, so only jobs that
    ask for it explicitly can write; every job declares its permissions and
    only the draft, build, manifest, verify and publish jobs may write;
  * every job has timeout-minutes, every action is pinned to a commit SHA,
    and no job in the environment restores a cache;
  * the protected environment is `production-release`: `publish` runs in
    it, behind `publish-gate`; besides it only the jobs that need the
    signing secrets (prepare, build) do, and every job that references a
    signing secret (APPLE_*, TAURI_SIGNING_*) runs in it.
"""

import re
import sys

import yaml

GATE = "validate-tag"
READ_ONLY = {"contents", "checks", "actions"}
PUBLISHER = "publish"
PUBLISH_GATE = "publish-gate"
ENVIRONMENT = "production-release"
# Jobs that may run in the environment: those that need the signing
# secrets, and publication.
ENVIRONMENT_JOBS = {"prepare", "build", PUBLISHER}
PINNED = re.compile(r"^[^@]+@[0-9a-f]{40}$")
CACHES = {"Swatinem/rust-cache", "actions/cache"}
SIGNING_SECRET = re.compile(r"secrets\.(APPLE_|TAURI_SIGNING_)")
# Jobs that may write to the repository: the draft and its assets (stage
# A), and publication (stage B).
WRITERS = {"draft", "build", "manifest", "verify", PUBLISHER}


def needs_of(job):
    needs = job.get("needs", [])
    return [needs] if isinstance(needs, str) else list(needs)


def check(path):
    with open(path, encoding="utf-8") as handle:
        workflow = yaml.safe_load(handle)
    jobs = workflow.get("jobs") or {}
    problems = []
    gate = jobs.get(GATE)
    if gate is None:
        return [f"no job {GATE}"]
    if needs_of(gate):
        problems.append(f"{GATE} must not depend on other jobs")
    permissions = gate.get("permissions")
    if not isinstance(permissions, dict):
        problems.append(f"{GATE} must declare its (read-only) permissions")
    else:
        for scope, level in permissions.items():
            if scope not in READ_ONLY or level not in ("read", "none"):
                problems.append(f"{GATE} may not have {scope}: {level}")
    # Without a permissions key, jobs get the repository's default token
    # permissions, which may include write access.
    top = workflow.get("permissions")
    if isinstance(top, dict):
        for scope, level in top.items():
            if level == "write":
                problems.append(f"workflow-level permission {scope}: write")
    elif top != "read-all":
        problems.append(f"workflow-level permissions must be read-only, not {top!r}")

    def reaches_gate(name, seen):
        if name == GATE:
            return True
        if name in seen or name not in jobs:
            return False
        seen.add(name)
        return any(reaches_gate(n, seen) for n in needs_of(jobs[name]))

    # Every job ends; every action is pinned to a commit; the jobs that hold
    # signing secrets restore no cache written by other workflows.
    for name, job in jobs.items():
        if not isinstance(job.get("timeout-minutes"), int):
            problems.append(f"job {name} has no timeout-minutes")
        for step in job.get("steps", []):
            uses = step.get("uses")
            if uses and not PINNED.match(uses):
                problems.append(f"job {name}: {uses} is not pinned to a commit SHA")
            if job.get("environment") and uses and uses.split("@")[0] in CACHES:
                problems.append(f"job {name} holds secrets and restores a cache ({uses})")
            if job.get("environment") and (step.get("with") or {}).get("cache"):
                problems.append(f"job {name} holds secrets and restores a cache ({uses})")

    # Stage B: `publish` publishes, in the protected environment, behind the
    # job that checks that environment. Signing secrets only in jobs that run
    # in the environment (they keep working from repository secrets until
    # the secrets move into it).
    if jobs.get(PUBLISHER, {}).get("environment") != ENVIRONMENT:
        problems.append(f"{PUBLISHER} must run in the environment '{ENVIRONMENT}'")
    if PUBLISH_GATE not in needs_of(jobs.get(PUBLISHER, {})):
        problems.append(f"{PUBLISHER} must need {PUBLISH_GATE}")
    for name, job in jobs.items():
        environment = job.get("environment")
        if environment is not None and (environment != ENVIRONMENT or name not in ENVIRONMENT_JOBS):
            problems.append(f"job {name} may not use the environment {environment!r}")
        if SIGNING_SECRET.search(yaml.safe_dump(job)) and environment != ENVIRONMENT:
            problems.append(f"job {name} uses signing secrets outside the environment '{ENVIRONMENT}'")
    for name, job in jobs.items():
        permissions = job.get("permissions")
        if not isinstance(permissions, dict):
            problems.append(f"job {name} must declare its permissions")
            continue
        for scope, level in permissions.items():
            if level == "write" and scope == "contents" and name not in WRITERS:
                problems.append(f"job {name} may not have contents: write")

    for name, job in jobs.items():
        if name == GATE:
            continue
        for dependency in needs_of(job):
            if dependency not in jobs:
                problems.append(f"{name} needs unknown job {dependency}")
        if not reaches_gate(name, set()):
            problems.append(f"job {name} does not depend on {GATE} through needs")
    return problems


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else ".github/workflows/release.yml"
    problems = check(path)
    for problem in problems:
        print(f"::error::{path}: {problem}")
    if problems:
        return 1
    print(f"{path}: every job depends on {GATE}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
