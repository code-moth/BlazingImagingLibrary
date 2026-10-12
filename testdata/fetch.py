#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
"""Download BIL's real test files and check them against testdata/manifest.toml.

    python3 testdata/fetch.py              # download and check every file
    python3 testdata/fetch.py --tier 1     # only the files Tier 1 needs
    python3 testdata/fetch.py --list       # also list the files each source's pages offer

A file whose checksum is not yet recorded is downloaded and its checksum printed, as a GitHub
annotation when run in CI, so it can be pinned in the manifest. Uses the standard library only.
"""

import argparse
import hashlib
import os
import re
import sys
import tomllib
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(HERE, "cache")
IN_CI = os.environ.get("GITHUB_ACTIONS") == "true"


REPORT = {"error": [], "warning": [], "notice": []}


def note(kind, message):
    """Collect a message. In CI, all messages of one kind become one annotation at the end,
    because GitHub shows only ten annotations of each kind for a step."""
    print(f"{kind}: {message}")
    REPORT[kind].append(message)


def flush():
    if not IN_CI:
        return
    for kind, messages in REPORT.items():
        if messages:
            text = "%0A".join(m.replace("%", "%25").replace("\n", " ") for m in messages)
            print(f"::{kind}::{text}")


def get(url):
    request = urllib.request.Request(url, headers={"User-Agent": "BIL test data fetch"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def fetch(bases, path):
    errors = []
    for base in bases:
        try:
            return get(base + path), base
        except Exception as error:  # try the next mirror
            errors.append(f"{base}: {error}")
    raise RuntimeError("; ".join(errors))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tier", type=int, help="only files needed by this tier or earlier")
    parser.add_argument("--list", action="store_true", help="list the files each source offers")
    args = parser.parse_args()

    with open(os.path.join(HERE, "manifest.toml"), "rb") as f:
        manifest = tomllib.load(f)
    bases = manifest["sources"]["gdal_cache"]["bases"]
    os.makedirs(CACHE, exist_ok=True)

    failed = 0
    for entry in manifest["file"]:
        if args.tier is not None and entry["tier"] > args.tier:
            continue
        path = entry["path"]
        target = os.path.join(CACHE, path)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        if os.path.exists(target):
            data = open(target, "rb").read()
        else:
            try:
                data, base = fetch(bases, path)
            except RuntimeError as error:
                note("error", f"could not download {path}: {error}")
                failed += 1
                continue
            open(target, "wb").write(data)
        digest = hashlib.sha256(data).hexdigest()
        expected = entry.get("sha256", "")
        if not expected:
            note("notice", f"unpinned {path} sha256={digest} bytes={len(data)}")
        elif digest != expected:
            note("error", f"checksum mismatch for {path}: expected {expected}, got {digest}")
            failed += 1
        else:
            print(f"ok {path}")

    if args.list:
        for page in manifest["sources"]["gdal_cache"]["listing_pages"]:
            try:
                html, base = fetch(bases, page)
            except RuntimeError as error:
                note("warning", f"could not list {page}: {error}")
                continue
            links = re.findall(rb'href="([^"?#]+\.(?:ntf|nitf|nsf|j2k|jp2))"', html, re.I)
            names = sorted(set(link.decode().rsplit("/", 1)[-1] for link in links))
            note("notice", f"{page}: {len(names)} files: " + " ".join(names))

    flush()
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
