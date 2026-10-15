# SPDX-License-Identifier: LGPL-2.1-or-later
"""Compares BIL's GeoTIFF reader with GDAL's, file by file.

For each TIFF file given, runs BIL's `geotiff_dump` example and the GDAL reference program
`geotiff_reference`, and compares what they report: the number of Images, each Image's size,
bands and sample type, its overviews' sizes, and a hash of its pixels, band after band, where BIL
decodes them. An Image whose samples or compression BIL does not read is noted, not counted, so
the run shows what remains to be read.

    python3 -I testdata/compare_geotiff.py GEOTIFF_DUMP GEOTIFF_REFERENCE FILE...

Exits with status 1 when any file differs. Under GitHub Actions each difference is raised as an
error annotation, and the summary and notes as notices, which can be read without the job's log.
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


def compare(bil_lines, gdal_lines):
    """The differences between the two reports, and notes on what was not compared."""
    if bil_lines[:1] and gdal_lines[:1] and bil_lines[0].startswith("error") and gdal_lines[0].startswith("error"):
        return [], [f"both refuse it: BIL {bil_lines[0][6:]!r}; GDAL {gdal_lines[0][6:]!r}"]
    notes = []
    bil = [l for l in bil_lines if not re.match(r"image \d+ (layout|missing) ", l)]
    gdal = list(gdal_lines)
    for line in bil_lines:
        m = re.match(r"image (\d+) missing (\d+)", line)
        if m:
            notes.append(f"image {m.group(1)}: {m.group(2)} pixels hold the no-data value, marked missing")
    # What BIL does not read is a known limit: its pixels are not compared, and a sample type
    # BIL does not read is not compared either.
    for line in list(bil):
        m = re.match(r"image (\d+) pixels refused (.*)$", line)
        if m:
            i = m.group(1)
            bil.remove(line)
            gdal = [g for g in gdal if not g.startswith(f"image {i} pixels ")]
            notes.append(f"image {i}: not compared: {m.group(2)}")
        m = re.match(r"image (\d+) size (\d+ x \d+ x \d+) unsupported$", line)
        if m:
            i, dims = m.group(1), m.group(2)
            same = [g for g in gdal if re.match(rf"image {i} size {re.escape(dims)} ", g)]
            if same:
                bil.remove(line)
                gdal.remove(same[0])
                notes.append(f"image {i}: sample type not compared: GDAL reads {same[0].split()[-1]}")
    count = collections.Counter
    ours, theirs = count(bil), count(gdal)
    problems = [f"BIL only: {x}" for x in sorted((ours - theirs).elements())]
    problems += [f"GDAL only: {x}" for x in sorted((theirs - ours).elements())]
    if problems:
        notes += [l for l in bil_lines if re.match(r"image \d+ layout ", l)]
    else:
        notes += [
            f"image {m.group(1)}: pixels agree"
            for l in bil
            if (m := re.match(r"image (\d+) pixels [0-9a-f]{16}$", l))
        ]
    return problems, notes


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    dump, reference, paths = sys.argv[1], sys.argv[2], sys.argv[3:]
    failed, compared = 0, 0
    actions = "GITHUB_ACTIONS" in os.environ
    for path in paths:
        problems, notes = compare(run(dump, path), run(reference, path))
        name = os.path.basename(path)
        compared += sum(n.endswith("pixels agree") for n in notes)
        if problems:
            failed += 1
            print(f"{name}: differs")
            for p in problems:
                print(f"    {p}")
                if actions:
                    print(f"::error::{name}: {p}"[:900])
            for n in notes:
                print(f"    {n}")
                if actions:
                    print(f"::warning::{name}: {n}"[:900])
        else:
            print(f"{name}: agrees")
            for n in notes:
                print(f"    {n}")
    summary = f"GeoTIFF reader against GDAL: {len(paths) - failed} of {len(paths)} files agree, {compared} Images pixel for pixel"
    print(summary)
    if actions:
        print(f"::notice::{summary}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
