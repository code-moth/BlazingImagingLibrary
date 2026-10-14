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
import tempfile


# Where both programs write each Image's pixels, band after band, for a closer look at a
# difference.
PIXELS = tempfile.mkdtemp(prefix="nitf-pixels-")


def run(program, path):
    env = dict(os.environ, NITF_PIXELS_DIR=PIXELS)
    out = subprocess.run([program, path], capture_output=True, text=True, errors="replace", env=env)
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
        if re.match(r"image \d+ layout ", line):
            continue
        m = re.match(r"image (\d+) missing (\d+)", line)
        if m:
            missing[int(m.group(1))] = int(m.group(2))
            continue
        plain.add(line)
    return plain, file_ext, image_ext, missing


def compare(bil_lines, gdal_lines, path):
    # A file both refuse agrees, whatever words each uses.
    if bil_lines[:1] and gdal_lines[:1] and bil_lines[0].startswith("error") and gdal_lines[0].startswith("error"):
        return [], [f"both refuse it: BIL {bil_lines[0][6:]!r}; GDAL {gdal_lines[0][6:]!r}"]
    bil, bil_file_ext, bil_ext, missing = facts(bil_lines)
    gdal, _, gdal_ext, _ = facts(gdal_lines)
    notes = []
    # Pixels never recorded hold no values: BIL leaves 0 and marks them, GDAL fills in its pad
    # value. Images whose pixels differ only there agree.
    for line in sorted(bil - gdal):
        m = re.match(r"image (\d+) pixels [0-9a-f]{16}$", line)
        other = [g for g in gdal - bil if m and re.match(rf"image {m.group(1)} pixels [0-9a-f]{{16}}$", g)]
        if not m or not other:
            continue
        found = pixel_differences(path, m.group(1), bil_lines)
        if found is None:
            continue
        recorded, unrecorded, text = found
        if recorded == 0:
            bil.discard(line)
            gdal.discard(other[0])
            notes.append(f"image {m.group(1)}: pixels agree where recorded; {unrecorded} bytes of pixels never recorded differ, which GDAL fills")
        else:
            notes.append(explain(path, m.group(1), bil_lines, text))
    problems = [f"BIL only: {x}" for x in sorted(bil - gdal)]
    problems += [f"GDAL only: {x}" for x in sorted(gdal - bil)]
    for i in sorted(set(bil_ext) | set(gdal_ext) | {int(x.split()[1]) for x in bil if x.startswith("image ")}):
        ours = sorted(bil_file_ext + bil_ext.get(i, []))
        theirs = sorted(gdal_ext.get(i, []))
        if ours != theirs:
            problems.append(f"image {i} extensions: BIL {ours}, GDAL {theirs}")
    notes += [f"image {i}: {n} pixels never recorded, marked missing" for i, n in sorted(missing.items())]
    return problems, notes


def pixel_differences(path, i, bil_lines):
    """Where BIL's and GDAL's pixels of Image i differ: (bytes that differ at pixels BIL holds as
    recorded, bytes that differ at pixels BIL marks never recorded, a description), or None
    where the pixel files are missing."""
    name = os.path.basename(path)
    try:
        ours = open(os.path.join(PIXELS, f"{name}.{i}.bil"), "rb").read()
        theirs = open(os.path.join(PIXELS, f"{name}.{i}.gdal"), "rb").read()
    except OSError:
        return None
    if len(ours) != len(theirs):
        return len(ours) or 1, 0, f"{len(ours)} bytes from BIL, {len(theirs)} from GDAL"
    size_line = next((l for l in bil_lines if l.startswith(f"image {i} size ")), "")
    m = re.match(r"image \d+ size (\d+) x (\d+) x (\d+)", size_line)
    pixels = int(m.group(1)) * int(m.group(2)) if m else 0
    bands = int(m.group(3)) if m else 1
    band_bytes = len(ours) // max(bands, 1)
    size = band_bytes // pixels if pixels else 1
    try:
        mask = open(os.path.join(PIXELS, f"{name}.{i}.mask"), "rb").read()
    except OSError:
        mask = None
    recorded = missing = 0
    first = []
    for k in range(len(ours)):
        if ours[k] != theirs[k]:
            # Band after band, so the pixel is the byte's place within its band, in samples.
            p = (k % band_bytes) // size if band_bytes and size else 0
            if mask is not None and p < len(mask) and mask[p] == 0:
                missing += 1
            else:
                recorded += 1
                if len(first) < 4:
                    first.append(f"byte {k}: BIL {ours[k]}, GDAL {theirs[k]}")
    text = f"{recorded} bytes differ at recorded pixels, {missing} at pixels never recorded"
    if first:
        text += "; first " + "; ".join(first)
    return recorded, missing, text


def explain(path, i, bil_lines, text):
    """How Image i is stored, with what differs: one line, within the annotations CI shows."""
    layout = next((l for l in bil_lines if l.startswith(f"image {i} layout ")), "")
    return f"image {i}: {text}; {layout[len(f'image {i} '):]}"


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    dump, reference, paths = sys.argv[1], sys.argv[2], sys.argv[3:]
    failed = 0
    actions = "GITHUB_ACTIONS" in os.environ
    for path in paths:
        bil_lines, gdal_lines = run(dump, path), run(reference, path)
        problems, notes = compare(bil_lines, gdal_lines, path)
        name = os.path.basename(path)
        summary = next((l for l in bil_lines if l.startswith("images ")), "?")
        if problems:
            failed += 1
            print(f"{name}: differs ({summary})")
            for p in problems:
                print(f"    {p}")
                if actions:
                    print(f"::error::{name}: {p}"[:900])
            for n in notes:
                if actions:
                    print(f"::warning::{name}: {n}"[:900])
        else:
            print(f"{name}: agrees ({summary})")
        for n in notes:
            print(f"    {n}")
            if actions and not problems:
                print(f"::notice::{name}: {n}"[:900])
    print(f"{len(paths) - failed} of {len(paths)} files agree")
    if actions:
        print(f"::notice::NITF reader against GDAL: {len(paths) - failed} of {len(paths)} files agree")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
