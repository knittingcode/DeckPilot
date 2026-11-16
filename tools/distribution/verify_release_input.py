#!/usr/bin/env python3
"""The common first step of every downstream distribution job
(docs/distribution/README.md, section 6.3).

    tools/distribution/verify-release-input.sh <tag> <channel> <out dir>

Environment: GITHUB_REPOSITORY (owner/name), GH_TOKEN for gh. Reads only:
every GitHub call is a GET or a download.

1. The release of <tag> must be published (not a draft), immutable, and
   flagged pre-release exactly when the tag is a pre-release.
2. SHA256SUMS and release-meta.json are downloaded; release-meta.json must
   describe <tag> and its version, and the tag must still point at its
   `commit`.
3. Only the assets <channel> needs (distlib.SELECTIONS) are downloaded and
   checked against SHA256SUMS and release-meta.json's `artifacts`
   (sha256, size, distribution_channel).
4. <out dir>/input.json is written: tag, version, release_channel,
   prerelease, commit, repository, and the verified assets (path, sha256,
   size, matrix id, target, package format, distribution channel).

Exit status 1 with a ::error:: line on the first failed check.
"""

import json
import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import distlib  # noqa: E402
from distlib import Failure  # noqa: E402


def gh(*args):
    result = subprocess.run(["gh", *args], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise Failure(f"gh {' '.join(args)} failed: {result.stderr.strip()[:300]}")
    return result.stdout


def api(path):
    return json.loads(gh("api", "-H", "Accept: application/vnd.github+json", path))


def download(repository, tag, names, directory):
    args = ["release", "download", tag, "--repo", repository, "--dir", str(directory), "--clobber"]
    for name in names:
        args += ["--pattern", name]
    gh(*args)
    for name in names:
        if not (directory / name).is_file():
            raise Failure(f"{name} was not downloaded from the release {tag}")


def tag_commit(repository, tag):
    ref = api(f"repos/{repository}/git/ref/tags/{tag}")
    target = ref.get("object") or {}
    # An annotated tag points at a tag object; follow it to the commit.
    for _ in range(5):
        if target.get("type") == "commit":
            return target.get("sha")
        if target.get("type") != "tag":
            raise Failure(f"the tag {tag} points at a {target.get('type')!r}, not a commit")
        target = (api(f"repos/{repository}/git/tags/{target.get('sha')}").get("object")) or {}
    raise Failure(f"the tag {tag} is nested too deeply")


def parse_sums(path):
    sums = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        digest, _, name = line.partition(" ")
        name = name.lstrip(" *")
        if len(digest) != 64 or not name:
            raise Failure(f"SHA256SUMS: malformed line {line!r}")
        sums[name] = digest.lower()
    return sums


def verify(tag, channel, out, repository):
    if channel not in distlib.CHANNELS:
        raise Failure(f"unknown channel {channel!r}; one of {', '.join(distlib.CHANNELS)}")
    if not tag.startswith("v"):
        raise Failure(f"{tag!r} is not a release tag (v<version>)")
    version = tag[1:]
    distlib.parse_version(version)
    prerelease = "-" in version

    release = api(f"repos/{repository}/releases/tags/{tag}")
    if release.get("tag_name") != tag:
        raise Failure(f"the API returned the release of {release.get('tag_name')!r}, not {tag}")
    if release.get("draft") is not False:
        raise Failure(f"the release {tag} is a draft: downstream jobs take published releases only")
    if release.get("immutable") is not True:
        raise Failure(f"the release {tag} is not immutable: its assets could still change")
    if bool(release.get("prerelease")) != prerelease:
        raise Failure(f"the release {tag} has prerelease={release.get('prerelease')!r}, but the tag says {prerelease}")

    meta_dir = out / "meta"
    meta_dir.mkdir(parents=True, exist_ok=True)
    download(repository, tag, ["SHA256SUMS", "release-meta.json"], meta_dir)
    sums = parse_sums(meta_dir / "SHA256SUMS")
    meta = distlib.load_json(meta_dir / "release-meta.json")
    if meta.get("tag") != tag or meta.get("version") != version:
        raise Failure(f"release-meta.json describes {meta.get('tag')!r} {meta.get('version')!r}, not {tag}")
    commit = meta.get("commit")
    actual = tag_commit(repository, tag)
    if not commit or actual != commit:
        raise Failure(f"the tag {tag} points at {actual}, but release-meta.json records {commit!r}")
    if "release-meta.json" in sums and sums["release-meta.json"] != distlib.sha256_file(meta_dir / "release-meta.json"):
        raise Failure("release-meta.json does not match SHA256SUMS")
    artifacts = meta.get("artifacts")
    if not isinstance(artifacts, list) or not artifacts:
        raise Failure(
            "release-meta.json has no artifacts list: the release predates the packaging matrix "
            "(docs/distribution/README.md, section 6.2), so its assets cannot be verified per channel"
        )

    chosen = distlib.select(artifacts, channel)
    asset_dir = out / "assets"
    asset_dir.mkdir(parents=True, exist_ok=True)
    if chosen:
        download(repository, tag, [a["name"] for a in chosen], asset_dir)
    verified = []
    for artifact in chosen:
        name = artifact["name"]
        path = asset_dir / name
        digest = distlib.sha256_file(path)
        if sums.get(name) != digest:
            raise Failure(f"{name}: sha256 {digest} does not match SHA256SUMS ({sums.get(name)})")
        if artifact.get("sha256") != digest:
            raise Failure(f"{name}: sha256 {digest} does not match release-meta.json ({artifact.get('sha256')})")
        if artifact.get("size") is not None and artifact["size"] != path.stat().st_size:
            raise Failure(f"{name}: size {path.stat().st_size} does not match release-meta.json ({artifact['size']})")
        verified.append({
            "name": name,
            "path": str(path),
            "sha256": digest,
            "size": path.stat().st_size,
            "matrix_id": artifact.get("matrix_id"),
            "target": artifact.get("target"),
            "arch": artifact.get("arch"),
            "package_format": artifact.get("package_format"),
            "distribution_channel": artifact.get("distribution_channel"),
            "url": f"https://github.com/{repository}/releases/download/{tag}/{name}",
        })

    value = {
        "tag": tag,
        "version": version,
        "release_channel": distlib.release_channel(version),
        "prerelease": prerelease,
        "commit": commit,
        "repository": repository,
        "channel": channel,
        "artifacts": verified,
    }
    distlib.write_json(out / "input.json", value)
    return value


def main():
    if len(sys.argv) != 4:
        print("usage: verify-release-input.sh <tag> <channel> <out dir>", file=sys.stderr)
        return 2
    tag, channel, out = sys.argv[1], sys.argv[2], pathlib.Path(sys.argv[3])
    repository = os.environ.get("GITHUB_REPOSITORY", "")
    if "/" not in repository:
        print("::error::GITHUB_REPOSITORY (owner/name) is not set")
        return 1
    try:
        value = verify(tag, channel, out, repository)
    except Failure as failure:
        print(f"::error::{tag} ({channel}): {failure}")
        return 1
    names = ", ".join(a["name"] for a in value["artifacts"]) or "none (built from the commit)"
    print(f"{tag} ({channel}): published, immutable, commit {value['commit'][:12]}; verified assets: {names}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
