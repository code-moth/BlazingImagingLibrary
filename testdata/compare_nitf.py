# SPDX-License-Identifier: LGPL-2.1-or-later
"""Compares BIL's NITF reader with GDAL's, file by file.

For each NITF file given, runs BIL's `nitf_dump` example and the GDAL reference program
`nitf_reference`, and compares what they report: the version, the number of Images, header
fields, each Image's size and sample type, the extensions present, and a hash of every
uncompressed Image's pixels, band after band.

    python3 -I testdata/compare_nitf.py NITF_DUMP NITF_REFERENCE FILE...

Exits with status 1 when any file differs. Under GitHub Actions each difference is also raised
as an annotation, which can be read without the job's log.
"""

import collections
import os
import re
import subprocess
import sys


def run(program, path):
    out = subprocess.run([program, path], capture_output=True, text=True, errors="replace")
    if out.returncode != 0:
        return [f"program failed with status {out.returncode}: {out.stderr.strip()[:300]}"]
    return out.stdout.splitlines()


def facts(lines):
    """The lines as facts to compare. Extensions are gathered per Image: GDAL reports the file
    header's with each Image's, so BIL's are gathered the same way, as tags without numbers."""
    plain, file_ext, image_ext, missing = set(), [], collections.defaultdict(list), {}
    for line in lines:
        m = re.match(r"(?:image (\d+) )?(all |file )?extension (\S+)", line)
        if m:
            tag = re.sub(r"\[\d+\]$", "", m.group(3))
            if m.group(2) == "file ":
                file_ext.append(tag)
            else:
                image_ext[int(m.group(1))].append(tag)
            continue
        m = re.match(r"image (\d+) missing (\d+)", line)
        if m:
            missing[int(m.group(1))] = int(m.group(2))
            continue
        plain.add(line)
    return plain, file_ext, image_ext, missing


def compare(bil_lines, gdal_lines):
    # A file both refuse agrees, whatever words each uses.
    if bil_lines[:1] and gdal_lines[:1] and bil_lines[0].startswith("error") and gdal_lines[0].startswith("error"):
        return [], [f"both refuse it: BIL {bil_lines[0][6:]!r}; GDAL {gdal_lines[0][6:]!r}"]
    bil, bil_file_ext, bil_ext, missing = facts(bil_lines)
    gdal, _, gdal_ext, _ = facts(gdal_lines)
    problems = [f"BIL only: {x}" for x in sorted(bil - gdal)]
    problems += [f"GDAL only: {x}" for x in sorted(gdal - bil)]
    for i in sorted(set(bil_ext) | set(gdal_ext) | {int(x.split()[1]) for x in bil if x.startswith("image ")}):
        ours = sorted(bil_file_ext + bil_ext.get(i, []))
        theirs = sorted(gdal_ext.get(i, []))
        if ours != theirs:
            problems.append(f"image {i} extensions: BIL {ours}, GDAL {theirs}")
    notes = [f"image {i}: {n} pixels never recorded, marked missing" for i, n in sorted(missing.items())]
    return problems, notes


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    dump, reference, paths = sys.argv[1], sys.argv[2], sys.argv[3:]
    failed = 0
    actions = "GITHUB_ACTIONS" in os.environ
    for path in paths:
        bil_lines, gdal_lines = run(dump, path), run(reference, path)
        problems, notes = compare(bil_lines, gdal_lines)
        name = os.path.basename(path)
        summary = next((l for l in bil_lines if l.startswith("images ")), "?")
        if problems:
            failed += 1
            print(f"{name}: differs ({summary})")
            for p in problems:
                print(f"    {p}")
                if actions:
                    print(f"::error::{name}: {p}"[:900])
        else:
            print(f"{name}: agrees ({summary})")
        for n in notes:
            print(f"    {n}")
    print(f"{len(paths) - failed} of {len(paths)} files agree")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
