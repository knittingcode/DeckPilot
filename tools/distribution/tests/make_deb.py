#!/usr/bin/env python3
"""A minimal, reproducible Debian package for the APT tests: an ar archive
with debian-binary, control.tar (the control file) and an empty data.tar.
Uncompressed (dpkg accepts that), so its bytes do not depend on the zlib
version; the Tauri bundler writes the same members gzip-compressed.

    make_deb.py <out.deb> <Package> <Version>
"""

import io
import sys
import tarfile


def tar(files):
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.GNU_FORMAT) as archive:
        for name, data in files:
            info = tarfile.TarInfo(name)
            info.size, info.mtime, info.mode = len(data), 0, 0o644
            archive.addfile(info, io.BytesIO(data))
    return raw.getvalue()


def ar_member(name, data):
    header = f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(data):<10}`\n".encode("ascii")
    return header + data + (b"\n" if len(data) % 2 else b"")


def main():
    out, package, version = sys.argv[1:4]
    control = (
        f"Package: {package}\nVersion: {version}\nArchitecture: amd64\nInstalled-Size: 1\n"
        "Maintainer: knittingcode\nPriority: optional\nDescription: Stream Deck management\n"
    ).encode("utf-8")
    body = b"!<arch>\n"
    body += ar_member("debian-binary", b"2.0\n")
    body += ar_member("control.tar", tar([("./control", control)]))
    body += ar_member("data.tar", tar([]))
    with open(out, "wb") as handle:
        handle.write(body)


if __name__ == "__main__":
    main()
