#!/usr/bin/env python3
"""The open results of run.sh's reports as one Markdown table, one row per
HID interface (product, usage page and usage), one column per variant.

    experiments/distribution/macos-sandbox-hid/summarize.py [<results dir>]

Each cell is `<exclusive>/<shared>`: ok when hidapi opened the device in
that mode, denied when it did not. Devices are matched by product and
usage, not by index: hidapi's enumeration order changes between runs.
"""

import os
import re
import sys

VARIANTS = ["unsandboxed-direct", "sandbox-usb-open", "sandbox-usb-direct", "sandbox-only-open"]
DEVICE = re.compile(
    r'^device\[(\d+)\] vid=(\S+) pid=(\S+) usage_page=(\S+) usage=(\S+) interface=\S+ product="([^"]*)"', re.M
)
OPEN = re.compile(r"^device\[(\d+)\] open (\w+) = (\w+)", re.M)


def main():
    directory = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "results")
    table = {}
    for variant in VARIANTS:
        with open(os.path.join(directory, variant + ".txt"), encoding="utf-8") as handle:
            text = handle.read()
        devices = {m.group(1): (m.group(6), m.group(4), m.group(5)) for m in DEVICE.finditer(text)}
        for m in OPEN.finditer(text):
            table.setdefault(devices[m.group(1)], {}).setdefault(variant, {})[m.group(2)] = m.group(3)
    print("| product | usage_page:usage | " + " | ".join(VARIANTS) + " |")
    print("| --- | --- |" + " --- |" * len(VARIANTS))
    for key in sorted(table):
        cells = []
        for variant in VARIANTS:
            result = table[key].get(variant, {})
            cells.append("/".join(
                {"opened": "ok", "failed": "denied"}.get(result.get(mode), "-") for mode in ("exclusive", "shared")
            ))
        print(f"| {key[0]} | {key[1]}:{key[2]} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
