#!/usr/bin/env python3
"""Structural check of the release workflow's gate, and of the downstream
distribution jobs.

    tools/release-pipeline/check-workflow-gate.py [<workflow.yml>]
    tools/release-pipeline/check-workflow-gate.py --downstream [<workflows dir>]

With --downstream (default directory .github/workflows): the jobs that
carry a published release to other channels (docs/distribution/README.md,
section 6.1) can never change it. A downstream job is every job of a
workflow whose file name starts with `distribute` (distribute.yml), and
every job named `distribute-*` in any workflow. Fails unless:
  * the job's token cannot write the repository's contents: its own
    permissions, or else its workflow's, are declared and grant at most
    contents: read (no write-all; no default token permissions);
  * no step writes a release: no `gh release upload|edit|delete|
    delete-asset|create`, no writing `gh api` or curl call on releases
    (-X/--method POST, PATCH, PUT or DELETE, or fields, which make gh api
    POST), no uploads.github.com, no release action, no github-script
    release call, and no tag push or tag deletion (a release follows its
    tag);
  * it runs outside the environment production-release and never reads the
    updater signing key (secrets.TAURI_SIGNING_*): it republishes, it does
    not sign updates.
A directory without downstream jobs passes.

Without --downstream:

Fails unless:
  * a job `validate-tag` exists, needs nothing, and may only read
    (contents, checks, actions);
  * every other job reaches `validate-tag` through `needs` (directly or
    through other jobs) -- an `if:` condition does not count;
  * the workflow-level permissions grant no write access, so only jobs that
    ask for it explicitly can write; every job declares its permissions and
    only the draft, build, manifest, verify and publish jobs may write;
  * a job `environment-gate` (no environment, no secret, read-only) checks
    the protected environment, and every job that uses the environment or
    a secret other than GITHUB_TOKEN needs it directly and has no `if:`
    with always(), failure() or cancelled(), so it is skipped whenever the
    environment gate did not succeed (fail closed);
  * the job `packaging` calls the Packaging workflow without secrets, with
    the gate's `prerelease` and `msi_version`, and `draft` needs it;
  * every job has timeout-minutes (except one that calls a workflow), every action is pinned to a commit SHA,
    and no job in the environment restores a cache;
  * the protected environment is `production-release`: `publish` runs in
    it, behind `publish-gate`; besides it only the jobs that need the
    signing secrets (prepare, build) do, and every job that references a
    signing secret (APPLE_*, TAURI_SIGNING_*) runs in it;
  * a job `website` starts the website workflow on main after a stable
    release only: it needs `publish` directly (so it runs only after the
    release was published and verified), its `if:` is exactly the gate's
    `prerelease == 'false'` (no status function), its permissions are
    exactly actions: write, it uses no environment, no secret and no
    action (nothing is checked out), and its one command is
    `gh workflow run website.yml --ref main` (the github-pages environment
    deploys main only). No other job may have actions: write.
"""

import pathlib
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
ENVIRONMENT_GATE = "environment-gate"
PACKAGING = "packaging"
PACKAGING_WORKFLOW = "./.github/workflows/packaging.yml"
PACKAGING_INPUTS = {
    "prerelease": "${{ needs.validate-tag.outputs.prerelease == 'true' }}",
    "msi-version": "${{ needs.validate-tag.outputs.msi_version }}",
}
PINNED = re.compile(r"^[^@]+@[0-9a-f]{40}$")
CACHES = {"Swatinem/rust-cache", "actions/cache"}
SIGNING_SECRET = re.compile(r"secrets\.(APPLE_|TAURI_SIGNING_)")
# Status check functions that let a job run although a dependency failed,
# was skipped or was cancelled.
STATUS_OVERRIDE = re.compile(r"\b(always|failure|cancelled)\s*\(")
# Jobs that may write to the repository: the draft and its assets (stage
# A), and publication (stage B).
WRITERS = {"draft", "build", "manifest", "verify", PUBLISHER}
# After a stable release: the website rebuild (a release published with the
# workflow token raises no `release` event for other workflows).
WEBSITE = "website"
WEBSITE_IF = "needs.validate-tag.outputs.prerelease=='false'"
WEBSITE_RUN = re.compile(r"^gh workflow run website\.yml --ref main$")


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
        # A job that calls a reusable workflow cannot have a timeout; the
        # called workflow's jobs have their own.
        if "uses" not in job and not isinstance(job.get("timeout-minutes"), int):
            problems.append(f"job {name} has no timeout-minutes")
        for step in job.get("steps", []):
            uses = step.get("uses")
            if uses and not PINNED.match(uses):
                problems.append(f"job {name}: {uses} is not pinned to a commit SHA")
            if job.get("environment") and uses and uses.split("@")[0] in CACHES:
                problems.append(f"job {name} holds secrets and restores a cache ({uses})")
            if job.get("environment") and (step.get("with") or {}).get("cache"):
                problems.append(f"job {name} holds secrets and restores a cache ({uses})")

    # Nothing names the protected environment or reads a signing secret
    # before the environment gate has checked it: GitHub would otherwise
    # create the environment unprotected and the job would run unapproved.
    gate_job = jobs.get(ENVIRONMENT_GATE)
    if gate_job is None:
        problems.append(f"no job {ENVIRONMENT_GATE}")
    else:
        if "environment" in gate_job:
            problems.append(f"{ENVIRONMENT_GATE} must not use an environment")
        if re.search(r"secrets\.(?!GITHUB_TOKEN)", yaml.safe_dump(gate_job)):
            problems.append(f"{ENVIRONMENT_GATE} must not read secrets")
        for scope, level in (gate_job.get("permissions") or {}).items():
            if level not in ("read", "none"):
                problems.append(f"{ENVIRONMENT_GATE} may not have {scope}: {level}")

    def reaches(name, target, seen):
        if name == target:
            return True
        if name in seen or name not in jobs:
            return False
        seen.add(name)
        return any(reaches(n, target, seen) for n in needs_of(jobs[name]))

    for name, job in jobs.items():
        if name == ENVIRONMENT_GATE:
            continue
        holds_secrets = "environment" in job or re.search(
            r"secrets\.(?!GITHUB_TOKEN)", yaml.safe_dump(job)
        )
        if not holds_secrets:
            continue
        if not reaches(name, ENVIRONMENT_GATE, set()):
            problems.append(f"job {name} uses the environment or secrets without needing {ENVIRONMENT_GATE}")
        # Fail closed: `needs` alone does not stop a job. It runs after a
        # failed or skipped dependency when its `if:` uses always(),
        # failure() or cancelled() (`!cancelled()` too), and a dependency
        # reached only through another job is bypassed when that job does.
        # So the job needs the environment gate itself and keeps the
        # implicit success() check.
        elif ENVIRONMENT_GATE not in needs_of(job):
            problems.append(f"job {name} uses the environment or secrets and must need {ENVIRONMENT_GATE} directly")
        if STATUS_OVERRIDE.search(str(job.get("if", ""))):
            problems.append(
                f"job {name} uses the environment or secrets and may not run after a failed dependency "
                f"(always(), failure() or cancelled() in its if: {job['if']!r})"
            )

    # The unsigned installer checks run on the tagged commit before anything
    # is signed: the draft (and through it everything after) needs them, and
    # they get no secret.
    packaging = jobs.get(PACKAGING)
    if not packaging or packaging.get("uses") != PACKAGING_WORKFLOW:
        problems.append(f"no job {PACKAGING} calling {PACKAGING_WORKFLOW}")
    else:
        if "secrets" in packaging:
            problems.append(f"{PACKAGING} must not pass secrets")
        # The gate's classification and MSI version, as the signed build
        # uses them.
        inputs = packaging.get("with") or {}
        for name, expected in PACKAGING_INPUTS.items():
            if str(inputs.get(name, "")).replace(" ", "") != expected.replace(" ", ""):
                problems.append(f"{PACKAGING} must pass {name}: {expected}")
        if PACKAGING not in needs_of(jobs.get("draft", {})):
            problems.append(f"draft must need {PACKAGING}")

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

    problems.extend(check_website(jobs))

    for name, job in jobs.items():
        if name == GATE:
            continue
        for dependency in needs_of(job):
            if dependency not in jobs:
                problems.append(f"{name} needs unknown job {dependency}")
        if not reaches_gate(name, set()):
            problems.append(f"job {name} does not depend on {GATE} through needs")
    return problems


def check_website(jobs):
    """The website rebuild after a stable release, and nothing more."""
    problems = []
    for name, job in jobs.items():
        if name != WEBSITE and (job.get("permissions") or {}).get("actions") == "write":
            problems.append(f"job {name} may not have actions: write (only {WEBSITE} starts a workflow)")
    job = jobs.get(WEBSITE)
    if job is None:
        return problems + [f"no job {WEBSITE} (the website rebuild after a stable release)"]
    if PUBLISHER not in needs_of(job):
        problems.append(f"{WEBSITE} must need {PUBLISHER} directly")
    condition = re.sub(r"\s+", "", str(job.get("if", "")))
    condition = re.sub(r"^\$\{\{(.*)\}\}$", r"\1", condition)
    if condition != WEBSITE_IF:
        problems.append(
            f"{WEBSITE} must run for stable releases only, after a successful publication: "
            f"if: needs.validate-tag.outputs.prerelease == 'false', not {job.get('if')!r}"
        )
    if job.get("permissions") != {"actions": "write"}:
        problems.append(f"{WEBSITE} must have exactly the permissions actions: write, not {job.get('permissions')!r}")
    if "environment" in job:
        problems.append(f"{WEBSITE} may not use an environment")
    if re.search(r"secrets\.(?!GITHUB_TOKEN)", yaml.safe_dump(job)):
        problems.append(f"{WEBSITE} may not read secrets")
    steps = job.get("steps") or []
    if any("uses" in step for step in steps):
        problems.append(f"{WEBSITE} may not use actions: it checks out and runs nothing of the repository")
    commands = [line.strip() for step in steps for line in str(step.get("run", "")).splitlines() if line.strip()]
    if len(commands) != 1 or not WEBSITE_RUN.match(commands[0]):
        problems.append(f"{WEBSITE} must run only `gh workflow run website.yml --ref main`, not {commands!r}")
    return problems


# Downstream distribution jobs (docs/distribution/README.md, section 6.1).
DOWNSTREAM_WORKFLOW = re.compile(r"^distribute")
DOWNSTREAM_JOB = re.compile(r"^distribute-")
READ_LEVELS = ("read", "none")
# Shell commands that change a release or the tag it follows.
RELEASE_WRITE = re.compile(
    r"\bgh\s+release\s+(upload|edit|delete-asset|delete|create)\b"
    r"|\bgh\s+api\b(?=[^\n]*/releases\b)(?![^\n]*\s(-X|--method)[\s=]*['\"]?GET\b)"
    r"(?=[^\n]*(\s(-X|--method)[\s=]*['\"]?(POST|PATCH|PUT|DELETE)\b"
    r"|\s(-f|-F|--field|--raw-field|--input)[\s=]))"
    r"|\bcurl\b(?=[^\n]*/releases\b)(?=[^\n]*\s(-X|--request)[\s=]*['\"]?(POST|PATCH|PUT|DELETE)\b)"
    r"|uploads\.github\.com"
    r"|\bgit\s+push\b[^\n]*(\s--tags\b|\s--delete\b|\s-d\s|refs/tags/|\s:v\d|\s\+?v\d[^\s]*:)",
    re.IGNORECASE,
)
RELEASE_ACTION = re.compile(
    r"^(softprops/action-gh-release|ncipollo/release-action|svenstaro/upload-release-action"
    r"|actions/upload-release-asset|actions/create-release|tauri-apps/tauri-action"
    r"|xresloader/upload-to-github-release)@"
)
SCRIPT_RELEASE_WRITE = re.compile(
    r"\b(createRelease|updateRelease|deleteRelease|uploadReleaseAsset|updateReleaseAsset|deleteReleaseAsset"
    r"|generateReleaseNotes|createRef|updateRef|deleteRef)\b"
)
UPDATER_KEY = re.compile(r"secrets\.TAURI_SIGNING_")


def contents_problem(permissions):
    """Why `permissions` may let the token write contents, or None."""
    if permissions is None:
        return "no permissions are declared (the default token may write)"
    if permissions == "read-all":
        return None
    if isinstance(permissions, str):
        return f"permissions: {permissions}"
    if isinstance(permissions, dict):
        level = permissions.get("contents", "none")
        if level not in READ_LEVELS:
            return f"contents: {level}"
        return None
    return f"permissions {permissions!r}"


def check_downstream(directory):
    """Problems of the downstream jobs in `directory`, and their number."""
    problems, count = [], 0
    files = sorted([*directory.glob("*.yml"), *directory.glob("*.yaml")])
    for path in files:
        workflow = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        jobs = workflow.get("jobs") or {}
        whole = bool(DOWNSTREAM_WORKFLOW.match(path.name))
        top = workflow.get("permissions")
        if whole:
            reason = contents_problem(top)
            if reason:
                problems.append(
                    f"{path.name}: the workflow-level permissions of a downstream workflow must grant at most "
                    f"contents: read ({reason}): downstream jobs never modify the release"
                )
        for name, job in jobs.items():
            if not isinstance(job, dict) or not (whole or DOWNSTREAM_JOB.match(name)):
                continue
            count += 1
            where = f"{path.name}: downstream job {name}"
            own = job.get("permissions")
            reason = contents_problem(own if own is not None else top)
            if reason:
                problems.append(f"{where} may not have contents: write ({reason}): downstream jobs never modify the release")
            environment = job.get("environment")
            if isinstance(environment, dict):
                environment = environment.get("name")
            if environment == ENVIRONMENT:
                problems.append(f"{where} may not use the environment {ENVIRONMENT}: it holds the application's signing secrets")
            if UPDATER_KEY.search(yaml.safe_dump(job, width=10**6)):
                problems.append(f"{where} uses the updater signing key: downstream jobs republish, they never sign updates")
            for index, step in enumerate(job.get("steps") or []):
                if not isinstance(step, dict):
                    continue
                label = f"{where}, step {step.get('name') or step.get('uses') or index + 1}"
                uses = str(step.get("uses", ""))
                # One line per command: continuation lines joined.
                run = re.sub(r"\\\r?\n", " ", str(step.get("run", "")))
                if RELEASE_ACTION.match(uses):
                    problems.append(f"{label} writes the release ({uses.split('@')[0]}): downstream jobs never modify the release")
                if uses.startswith("actions/github-script@") and SCRIPT_RELEASE_WRITE.search(
                    str((step.get("with") or {}).get("script", ""))
                ):
                    problems.append(f"{label} writes the release or a tag (github-script): downstream jobs never modify the release")
                match = RELEASE_WRITE.search(run)
                if match:
                    problems.append(
                        f"{label} writes the release ({match.group(0).strip()[:60]}): downstream jobs never modify the release"
                    )
    return problems, count


def main_downstream(arguments):
    directory = pathlib.Path(arguments[0] if arguments else ".github/workflows")
    if not directory.is_dir():
        print(f"::error::{directory} is not a directory")
        return 1
    problems, count = check_downstream(directory)
    for problem in problems:
        print(f"::error::{directory}/{problem}")
    if problems:
        return 1
    if count:
        print(f"{directory}: {count} downstream job(s), none can write the release")
    else:
        print(f"{directory}: no downstream distribution jobs")
    return 0


def main():
    if sys.argv[1:2] == ["--downstream"]:
        return main_downstream(sys.argv[2:])
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
