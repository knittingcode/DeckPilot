"""Shared rules of the downstream distribution tools (docs/distribution/README.md,
sections 6.3 and 6.4): which verified release artifacts each channel
takes, version mappings, hashing. Standard library only."""

import hashlib
import json
import re

VERSION = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta|rc)\.(\d+))?$")

# The verified inputs of each downstream job: matrix ids (targets.toml) and
# the distribution channel compiled into those builds. Channels that build
# from the release's commit (Flathub, the Mac App Store, MSIX) take no
# artifact; the Microsoft Store MSI listing points at the GitHub MSI.
SELECTIONS = {
    "homebrew": {"matrix_ids": ["github-macos-arm64", "github-macos-x64"], "suffix": ".dmg", "channel": "github"},
    "winget": {"matrix_ids": ["github-windows-msi"], "suffix": ".msi", "channel": "github"},
    "apt": {"matrix_ids": ["apt-deb"], "suffix": "_apt.deb", "channel": "apt"},
    "msstore": {"matrix_ids": ["github-windows-msi"], "suffix": ".msi", "channel": "github"},
    "flathub": None,
    "mas": None,
}
CHANNELS = tuple(SELECTIONS)


class Failure(Exception):
    """A check failed; the message says which and why."""


def parse_version(version):
    match = VERSION.match(version or "")
    if not match:
        raise Failure(f"{version!r} is not a DeckPilot version (X.Y.Z or X.Y.Z-alpha|beta|rc.N)")
    major, minor, patch, kind, number = match.groups()
    return int(major), int(minor), int(patch), kind, None if number is None else int(number)


def release_channel(version):
    """`release_channel` of docs/distribution/README.md, section 1."""
    kind = parse_version(version)[3]
    return {None: "stable", "alpha": "alpha", "beta": "beta", "rc": "beta"}[kind]


def debian_version(version):
    """The Debian package version: a pre-release suffix after `~`, which
    dpkg sorts before the release (0.6.0~alpha.3 < 0.6.0). The bundler's
    verbatim 0.6.0-alpha.3 would make the alpha an upstream 0.6.0 with
    revision alpha.3, sorting after 0.6.0."""
    major, minor, patch, kind, number = parse_version(version)
    base = f"{major}.{minor}.{patch}"
    return base if kind is None else f"{base}~{kind}.{number}"


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def select(meta_artifacts, channel):
    """The release artifacts `channel` takes, from release-meta.json's
    `artifacts`. Raises Failure when one is missing or was built for
    another distribution channel."""
    rule = SELECTIONS[channel]
    if rule is None:
        return []
    chosen = []
    for matrix_id in rule["matrix_ids"]:
        candidates = [a for a in meta_artifacts if a.get("matrix_id") == matrix_id and a.get("name", "").endswith(rule["suffix"])]
        if len(candidates) != 1:
            if matrix_id == "apt-deb":
                raise Failure(
                    "the release has no APT variant (matrix id apt-deb, *_apt.deb) in release-meta.json's "
                    "artifacts: the GitHub .deb is never republished to the APT repository (rule R7)"
                )
            raise Failure(f"release-meta.json has {len(candidates)} artifacts of {matrix_id} ending in {rule['suffix']}, expected 1")
        artifact = candidates[0]
        if artifact.get("distribution_channel") != rule["channel"]:
            raise Failure(
                f"{artifact['name']} was built for the distribution channel {artifact.get('distribution_channel')!r}, "
                f"but {channel} takes only {rule['channel']!r} builds"
            )
        chosen.append(artifact)
    return chosen


def load_json(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2, sort_keys=False)
        handle.write("\n")
