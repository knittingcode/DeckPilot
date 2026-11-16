#!/usr/bin/env python3
"""A published release shaped like v0.6.0-alpha.3, for the gh stub.

    fixture.py build <stub dir> [--apt-deb-version <debian version>]
    fixture.py mutate <stub dir> <mutation>

`build` writes, from the packaging matrix and the release-meta.json of
v0.6.0-alpha.3 (tools/release-pipeline/fixtures/release-meta), what
verify-release-input.sh reads: release.json (the API's release), ref.json
(the tag), assets/ (one small file per asset name of the shipping targets,
SHA256SUMS, and release-meta.json with the `artifacts` list computed by
tools/release-pipeline/release-artifacts.py, as PR D's release does).
With --apt-deb-version it adds the planned APT variant
(DeckPilot_<version>_amd64_apt.deb, distribution channel apt), which no
release has yet.

`mutate` changes one thing that verification must catch.
"""

import json
import os
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent.parent
sys.path.insert(0, str(HERE.parent))
import distlib  # noqa: E402

TAG = "v0.6.0-alpha.3"
VERSION = TAG[1:]
TARGETS = ROOT / "docs" / "distribution" / "targets.toml"
META = ROOT / "tools" / "release-pipeline" / "fixtures" / "release-meta" / f"{TAG}.json"
RUST_TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]
SHIPPING_ASSETS = [
    "DeckPilot_0.6.0-alpha.3_aarch64.dmg", "DeckPilot_aarch64.app.tar.gz", "DeckPilot_aarch64.app.tar.gz.sig",
    "DeckPilot_0.6.0-alpha.3_x64.dmg", "DeckPilot_x64.app.tar.gz", "DeckPilot_x64.app.tar.gz.sig",
    "DeckPilot_0.6.0-alpha.3_x64_en-US.msi", "DeckPilot_0.6.0-alpha.3_x64_en-US.msi.sig",
    "DeckPilot_0.6.0-alpha.3_x64-setup.exe", "DeckPilot_0.6.0-alpha.3_x64-setup.exe.sig",
    "DeckPilot_0.6.0-alpha.3_amd64.deb", "DeckPilot_0.6.0-alpha.3_amd64.deb.sig",
    "DeckPilot_0.6.0-alpha.3_amd64.AppImage", "DeckPilot_0.6.0-alpha.3_amd64.AppImage.sig",
]
APT_DEB = "DeckPilot_0.6.0-alpha.3_amd64_apt.deb"


def write_sums(assets):
    lines = []
    for path in sorted(assets.iterdir()):
        if path.name != "SHA256SUMS":
            lines.append(f"{distlib.sha256_file(path)}  {path.name}")
    (assets / "SHA256SUMS").write_text("\n".join(lines) + "\n", encoding="utf-8")


def build(stub, apt_version):
    assets = stub / "assets"
    assets.mkdir(parents=True)
    for name in SHIPPING_ASSETS:
        (assets / name).write_text(f"DeckPilot test asset {name}\n", encoding="utf-8")
    (assets / "prerelease.json").write_text('{"version": "0.6.0-alpha.3"}\n', encoding="utf-8")
    records = stub / "records"
    records.mkdir()
    for target in RUST_TARGETS:
        distlib.write_json(records / f"{target}.json", {
            "target": target, "application_version": VERSION, "distribution_channel": "github", "package_format": "",
        })
    environment = dict(os.environ, VERSION=VERSION, PRERELEASE="true", MANIFEST="prerelease.json")
    artifacts = json.loads(subprocess.run(
        [sys.executable, "-I", str(ROOT / "tools" / "release-pipeline" / "release-artifacts.py"),
         str(TARGETS), str(assets), str(records)],
        check=True, capture_output=True, text=True, env=environment,
    ).stdout)
    if apt_version:
        subprocess.run([sys.executable, "-I", str(HERE / "make_deb.py"), str(assets / APT_DEB), "deck-pilot", apt_version], check=True)
        path = assets / APT_DEB
        artifacts.append({
            "name": APT_DEB, "size": path.stat().st_size, "sha256": distlib.sha256_file(path), "matrix_id": "apt-deb",
            "target": "x86_64-unknown-linux-gnu", "platform": "linux", "arch": "x86_64", "package_format": "deb",
            "distribution_channel": "apt", "update_provider": "external_package_manager", "signature": None,
        })
    meta = distlib.load_json(META)
    meta["artifacts"] = artifacts
    distlib.write_json(assets / "release-meta.json", meta)
    write_sums(assets)
    distlib.write_json(stub / "release.json", {
        "tag_name": TAG, "draft": False, "prerelease": True, "immutable": True,
        "assets": [{"name": p.name} for p in sorted(assets.iterdir())],
    })
    distlib.write_json(stub / "ref.json", {"ref": f"refs/tags/{TAG}", "object": {"type": "commit", "sha": meta["commit"]}})


def mutate(stub, mutation):
    assets = stub / "assets"
    release = distlib.load_json(stub / "release.json")
    meta = distlib.load_json(assets / "release-meta.json")
    if mutation == "draft":
        release["draft"] = True
    elif mutation == "mutable":
        release["immutable"] = False
    elif mutation == "not-prerelease":
        release["prerelease"] = False
    elif mutation == "moved-tag":
        distlib.write_json(stub / "ref.json", {"object": {"type": "commit", "sha": "0" * 40}})
    elif mutation == "annotated-tag":
        sha = "a" * 40
        distlib.write_json(stub / "ref.json", {"object": {"type": "tag", "sha": sha}})
        distlib.write_json(stub / f"tag-{sha}.json", {"object": {"type": "commit", "sha": meta["commit"]}})
    elif mutation == "tampered-dmg":
        (assets / "DeckPilot_0.6.0-alpha.3_aarch64.dmg").write_text("tampered\n", encoding="utf-8")
    elif mutation == "tampered-dmg-and-sums":
        (assets / "DeckPilot_0.6.0-alpha.3_aarch64.dmg").write_text("tampered\n", encoding="utf-8")
        write_sums(assets)
    elif mutation == "no-artifacts":
        meta.pop("artifacts")
    elif mutation == "published-alpha3-meta":
        # The release-meta.json v0.6.0-alpha.3 was published with: no artifacts.
        meta = distlib.load_json(META)
    elif mutation == "github-deb-as-apt":
        for artifact in meta["artifacts"]:
            if artifact["name"] == "DeckPilot_0.6.0-alpha.3_amd64.deb":
                artifact["matrix_id"], artifact["name"] = "apt-deb", APT_DEB
        (assets / APT_DEB).write_bytes((assets / "DeckPilot_0.6.0-alpha.3_amd64.deb").read_bytes())
    elif mutation == "msi-for-store":
        for artifact in meta["artifacts"]:
            if artifact["matrix_id"] == "github-windows-msi":
                artifact["distribution_channel"] = "microsoft_store"
    else:
        sys.exit(f"unknown mutation {mutation}")
    distlib.write_json(stub / "release.json", release)
    if mutation not in ("draft", "mutable", "not-prerelease", "moved-tag", "annotated-tag", "tampered-dmg", "tampered-dmg-and-sums"):
        distlib.write_json(assets / "release-meta.json", meta)
        write_sums(assets)


def main():
    command, stub = sys.argv[1], pathlib.Path(sys.argv[2])
    if command == "build":
        apt_version = sys.argv[4] if len(sys.argv) > 4 and sys.argv[3] == "--apt-deb-version" else None
        build(stub, apt_version)
    elif command == "mutate":
        mutate(stub, sys.argv[3])
    else:
        sys.exit(f"unknown command {command}")


if __name__ == "__main__":
    main()
