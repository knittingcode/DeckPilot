#!/usr/bin/env python3
"""A `gh` stand-in for tools/distribution/test.sh: answers the read-only
calls verify-release-input.sh makes from a fixture directory
($GH_STUB_DIR) and logs every call to $GH_STUB_DIR/calls.log.

    gh api [-H <header>] repos/<owner>/<name>/releases/tags/<tag>   -> release.json
    gh api [-H <header>] repos/<owner>/<name>/git/ref/tags/<tag>    -> ref.json
    gh api [-H <header>] repos/<owner>/<name>/git/tags/<sha>        -> tag-<sha>.json
    gh release download <tag> --repo R --dir D --clobber --pattern N...  -> copies assets/N

Anything else (a write, a method, a field) fails the test.
"""

import os
import pathlib
import re
import shutil
import sys

root = pathlib.Path(os.environ["GH_STUB_DIR"])
args = sys.argv[1:]
with open(root / "calls.log", "a", encoding="utf-8") as log:
    log.write(" ".join(args) + "\n")


def fail(message):
    print(f"gh stub: {message}", file=sys.stderr)
    sys.exit(1)


if args[:1] == ["api"]:
    rest = args[1:]
    if len(rest) == 3 and rest[0] == "-H":
        rest = rest[2:]
    if len(rest) != 1:
        fail(f"unexpected api arguments {args!r} (only plain GETs are allowed)")
    path = rest[0]
    for pattern, name in (
        (r"^repos/[^/]+/[^/]+/releases/tags/[^/]+$", "release.json"),
        (r"^repos/[^/]+/[^/]+/git/ref/tags/[^/]+$", "ref.json"),
        (r"^repos/[^/]+/[^/]+/git/tags/([0-9a-f]+)$", None),
    ):
        match = re.match(pattern, path)
        if match:
            name = name or f"tag-{match.group(1)}.json"
            file = root / name
            if not file.is_file():
                fail(f"HTTP 404: {path}")
            sys.stdout.write(file.read_text(encoding="utf-8"))
            sys.exit(0)
    fail(f"unexpected api path {path}")

if args[:2] == ["release", "download"]:
    directory, patterns, index = None, [], 3
    while index < len(args):
        if args[index] in ("--dir", "-D"):
            directory = pathlib.Path(args[index + 1])
            index += 2
        elif args[index] in ("--pattern", "-p"):
            patterns.append(args[index + 1])
            index += 2
        elif args[index] in ("--repo", "-R"):
            index += 2
        elif args[index] == "--clobber":
            index += 1
        else:
            fail(f"unexpected download argument {args[index]}")
    if directory is None or not patterns:
        fail("download without --dir or --pattern")
    directory.mkdir(parents=True, exist_ok=True)
    for pattern in patterns:
        source = root / "assets" / pattern
        if not source.is_file():
            fail(f"no asset matches {pattern}")
        shutil.copyfile(source, directory / pattern)
    sys.exit(0)

fail(f"unexpected command {args!r}")
