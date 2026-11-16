#!/usr/bin/env python3
"""The `artifacts` list of release-meta.json, from the packaging matrix and
the files of the draft (docs/distribution/README.md, section 6.2).

    tools/release-pipeline/release-artifacts.py <targets.toml> <assets dir> <records dir>

Environment: VERSION, PRERELEASE (true/false), MANIFEST (latest.json or
prerelease.json).

Prints a JSON array with one entry per installable or updater file of the
release, in the order of docs/distribution/targets.toml:

    {"name", "size", "sha256", "matrix_id", "target", "platform", "arch",
     "package_format", "distribution_channel", "update_provider", "signature"}

`signature` is the name of the updater signature beside the file, or null.
Each target of the matrix that ships in this release (status "shipping";
"stable_only" for stable releases only) must have every asset it lists,
and every file of the release must belong to one of them, except the
update manifest, release-meta.json and SHA256SUMS.

`distribution_channel` is the one compiled into the build of the target:
<records dir>/<rust target>.json, written by the release build from the
binary's embedded record (tools/ci/check-packages.sh record). It must be
the matrix's channel; a GitHub build carries no package format (the
bundler patches it into each package, checked by the Packaging workflow),
and a channel with its own build must have the matrix's format compiled
in. Nothing here is typed in by hand.
"""

import hashlib
import json
import os
import pathlib
import sys

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11
    sys.exit("release-artifacts.py needs Python 3.11 or newer (tomllib)")

SHIPPING = "shipping"
STABLE_ONLY = "stable_only"
NOT_PACKAGES = {"release-meta.json", "SHA256SUMS"}


def fail(message):
    print(f"::error::{message}", file=sys.stderr)
    sys.exit(1)


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def main():
    if len(sys.argv) != 4:
        fail("usage: release-artifacts.py <targets.toml> <assets dir> <records dir>")
    targets_path, assets, records = (pathlib.Path(arg) for arg in sys.argv[1:])
    version = os.environ.get("VERSION") or fail("VERSION is not set")
    prerelease = os.environ.get("PRERELEASE")
    if prerelease not in ("true", "false"):
        fail("PRERELEASE must be true or false")
    manifest = os.environ.get("MANIFEST") or fail("MANIFEST is not set")

    matrix = tomllib.loads(targets_path.read_text(encoding="utf-8"))
    if matrix.get("schema") != 1:
        fail(f"{targets_path}: unknown schema {matrix.get('schema')!r}")
    providers = matrix.get("provider_for_channel") or {}
    statuses = {SHIPPING} if prerelease == "true" else {SHIPPING, STABLE_ONLY}
    files = {path.name for path in assets.iterdir() if path.is_file()} if assets.is_dir() else set()
    if not files:
        fail(f"no release files in {assets}")

    artifacts, described = [], set()
    for target in matrix.get("target", []):
        if target.get("status") not in statuses:
            continue
        where = f"{targets_path}: target {target['id']}"
        channel = target["distribution_channel"]
        provider = providers.get(channel)
        if provider is None or target.get("update_provider") != provider:
            fail(f"{where}: update_provider {target.get('update_provider')!r} is not the provider of {channel} ({provider!r})")
        names = [name.replace("{version}", version) for name in target.get("assets", [])]
        if not names:
            fail(f"{where} ships but lists no assets")
        missing = [name for name in names if name not in files]
        if missing:
            fail(f"{where} lists {', '.join(missing)}, which the release lacks")

        record_path = records / f"{target['rust_target']}.json"
        if not record_path.is_file():
            fail(f"no distribution record of the {target['rust_target']} build ({record_path}); did its build finish?")
        record = json.loads(record_path.read_text(encoding="utf-8"))
        if record.get("target") != target["rust_target"]:
            fail(f"{record_path} is the record of {record.get('target')!r}, not {target['rust_target']}")
        if record.get("application_version") != version:
            fail(f"the {target['rust_target']} build records version {record.get('application_version')!r}, expected {version}")
        recorded = record.get("distribution_channel")
        if recorded != channel:
            fail(f"the {target['rust_target']} build was compiled for the distribution channel {recorded!r}, "
                 f"but {where} is {channel!r}")
        recorded_format = record.get("package_format") or ""
        expected_format = "" if channel == "github" else target["package_format"]
        if recorded_format != expected_format:
            fail(f"the {target['rust_target']} build has the package format {recorded_format!r} compiled in, "
                 f"expected {expected_format!r} for {where}")

        for name in names:
            if name in described:
                fail(f"{name} belongs to more than one target of {targets_path}")
            described.add(name)
            if name.endswith(".sig"):
                continue
            signature = f"{name}.sig"
            path = assets / name
            artifacts.append({
                "name": name,
                "size": path.stat().st_size,
                "sha256": sha256(path),
                "matrix_id": target["id"],
                "target": target["rust_target"],
                "platform": target["os"],
                "arch": target["arch"],
                "package_format": target["package_format"],
                "distribution_channel": recorded,
                "update_provider": provider,
                "signature": signature if signature in names else None,
            })

    undescribed = sorted(files - described - NOT_PACKAGES - {manifest})
    if undescribed:
        fail(f"release files outside the packaging matrix ({targets_path}): {', '.join(undescribed)}")
    json.dump(artifacts, sys.stdout, indent=2)
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
