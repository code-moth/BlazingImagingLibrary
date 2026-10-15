# SPDX-License-Identifier: LGPL-2.1-or-later
"""Compares BIL's GeoTIFF reader with GDAL's, file by file.

For each TIFF file given, runs BIL's `geotiff_dump` example and the GDAL reference program
`geotiff_reference`, and compares what they report: the number of Images, each Image's size,
bands and sample type, its overviews' sizes, and a hash of its pixels, band after band, where BIL
decodes them. An Image whose samples or compression BIL does not read is noted, not counted, so
the run shows what remains to be read.

    python3 -I testdata/compare_geotiff.py GEOTIFF_DUMP GEOTIFF_REFERENCE FILE...

Each file falls in one of seven categories, which `compare` lists. Exits with status 1 when any
file differs, which includes BIL's program failing. Under GitHub Actions differences are raised as error annotations,
and the summary, with the files that are stricter or differ, as notices, which can be read
without the job's log.
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
    """How the two reports compare: a category, the differences, and notes. The categories:
    `agree`; `refused`, both refuse the file; `limit`, BIL does not read what GDAL does, as it says; `stricter`, BIL refuses as
    corrupt a file or Image that GDAL reads; `lenient`, GDAL refuses a file or
    Image BIL opens; `converts`, GDAL converts the color of CMYK, YCbCr or CIELab samples that
    BIL returns as stored; `differ`, a disagreement, which fails the run."""
    if bil_lines[:1] and bil_lines[0].startswith("program failed"):
        return "differ", [f"BIL's {bil_lines[0]}"], []
    bil_error = bil_lines[:1] and bil_lines[0].startswith("error")
    gdal_error = gdal_lines[:1] and gdal_lines[0].startswith("error")
    gdal_pixels_fail = all(" pixels error" in l for l in gdal_lines if " pixels " in l)
    if bil_error and gdal_error:
        return "refused", [], [f"both refuse it: BIL {bil_lines[0][6:]!r}; GDAL {gdal_lines[0][6:]!r}"]
    if bil_error and bil_lines[0].startswith("error not supported"):
        return "limit", [], [f"BIL does not read it: {bil_lines[0][6:]}"]
    if bil_error and gdal_pixels_fail:
        return "refused", [], [f"BIL refuses it and GDAL cannot read its pixels: {bil_lines[0][6:]}"]
    if bil_error:
        return "stricter", [], [f"BIL refuses it, GDAL reads it: {bil_lines[0][6:]}"]
    if gdal_error:
        return "lenient", [], [f"GDAL refuses it, BIL opens it: GDAL {gdal_lines[0][6:]!r}"]
    category = "agree"
    notes = []
    bil = [l for l in bil_lines if not re.match(r"image \d+ (layout|missing|widened|unsigned|photometric) ", l)]
    gdal = list(gdal_lines)
    # GDAL 3.8 reads float16 as float32: compare BIL's values widened the same way.
    for line in list(bil):
        m = re.match(r"image (\d+) size (\d+ x \d+ x \d+) F16$", line)
        wide = [l for l in bil_lines if m and l.startswith(f"image {m.group(1)} widened F32 ")]
        if m and wide and f"image {m.group(1)} size {m.group(2)} F32" in gdal:
            i = m.group(1)
            bil[bil.index(line)] = f"image {i} size {m.group(2)} F32"
            bil = [l if not l.startswith(f"image {i} pixels ") else f"image {i} pixels {wide[0].split()[-1]}" for l in bil]
            notes.append(f"image {i}: float16, compared widened to float32 as GDAL reads it")
    # GDAL 3.8 reads signed integers of a width of their own as unsigned, their sign not extended;
    # BIL extends it (M43). Compare BIL's values masked to the stored width.
    for line in list(bil):
        m = re.match(r"image (\d+) size (\d+ x \d+ x \d+) I(8|16|32|64)$", line)
        masked = [l for l in bil_lines if m and l.startswith(f"image {m.group(1)} unsigned U{m.group(3)} ")]
        if m and masked and f"image {m.group(1)} size {m.group(2)} U{m.group(3)}" in gdal:
            i = m.group(1)
            bil[bil.index(line)] = f"image {i} size {m.group(2)} U{m.group(3)}"
            bil = [l if not l.startswith(f"image {i} pixels ") else f"image {i} pixels {masked[0].split()[-1]}" for l in bil]
            notes.append(f"image {i}: signed samples of their own width, compared unsigned as GDAL reads them")
    for line in bil_lines:
        m = re.match(r"image (\d+) missing (\d+)", line)
        if m:
            notes.append(f"image {m.group(1)}: {m.group(2)} pixels never recorded or holding no-data, marked missing")
    # What BIL does not read is a known limit: its pixels are not compared, and a sample type
    # BIL does not read is not compared either.
    for line in list(bil):
        m = re.match(r"image (\d+) pixels (refused|error) (.*)$", line)
        if m:
            i = m.group(1)
            theirs = [g for g in gdal if g.startswith(f"image {i} pixels ")]
            if m.group(2) == "refused":
                category = "limit"
                notes.append(f"image {i}: not compared: {m.group(3)}")
            elif all(" pixels error" in g for g in theirs):
                notes.append(f"image {i}: neither reads its pixels")
            else:
                category = "stricter"
                notes.append(f"image {i}: BIL refuses its pixels, GDAL reads them: {m.group(3)}")
            bil.remove(line)
            gdal = [g for g in gdal if g not in theirs]
        m = re.match(r"image (\d+) size (\d+ x \d+ x \d+) unsupported$", line)
        if m:
            i, dims = m.group(1), m.group(2)
            same = [g for g in gdal if re.match(rf"image {i} size {re.escape(dims)} ", g)]
            if same:
                bil.remove(line)
                gdal.remove(same[0])
                notes.append(f"image {i}: sample type not compared: GDAL reads {same[0].split()[-1]}")
    # Stored levels, each read whole (M38): what BIL does not read is a known limit, and an
    # overview neither reads is not compared.
    for line in list(bil):
        m = re.match(r"image (\d+) overview (\d+ x \d+) pixels (refused|error) ?(.*)$", line)
        if not m:
            continue
        i, k = m.group(1), m.group(2)
        theirs = [g for g in gdal if g.startswith(f"image {i} overview {k} pixels ")]
        if m.group(3) == "refused":
            if category == "agree":
                category = "limit"
            notes.append(f"image {i}: overview {k} not compared: {m.group(4)}")
        elif all(" pixels error" in g for g in theirs):
            notes.append(f"image {i}: neither reads overview {k}")
        else:
            category = "stricter"
            notes.append(f"image {i}: BIL refuses overview {k}, GDAL reads it: {m.group(4)}")
        bil.remove(line)
        gdal = [g for g in gdal if g not in theirs]
    # GDAL lists only the Images it can open; one it leaves out that BIL lists is lenience, not
    # a difference, as long as the Images both list agree.
    m = re.match(r"images (\d+)$", gdal[0]) if gdal else None
    if m and f"images {m.group(1)}" not in bil:
        listed = int(m.group(1))
        numbers = {int(x.group(1)) for l in bil if (x := re.match(r"image (\d+) ", l))}
        extra = sorted(n for n in numbers if n >= listed)
        if extra:
            bil = [l for l in bil if not l.startswith("images ")]
            bil = [l for l in bil if int(re.match(r"image (\d+) ", l).group(1)) < listed]
            gdal = gdal[1:]
            category = "lenient"
            notes.append(f"GDAL does not list image(s) {extra}, which BIL opens")
    # GDAL converts CMYK (5), YCbCr (6) and CIELab (8, 9, 10) to RGB or RGBA; BIL returns the stored
    # samples (section 4). Those Images' sizes and pixels are not compared.
    for line in bil_lines:
        m = re.match(r"image (\d+) photometric Some\((5|6|8|9|10)\)$", line)
        if m:
            i = m.group(1)
            ours_i = [l for l in bil if re.match(rf"image {i} (size|pixels|overview \d+ x \d+ pixels) ", l)]
            theirs_i = [l for l in gdal if re.match(rf"image {i} (size|pixels|overview \d+ x \d+ pixels) ", l)]
            if ours_i != theirs_i:
                bil = [l for l in bil if l not in ours_i]
                gdal = [l for l in gdal if l not in theirs_i]
                if category == "agree":
                    category = "converts"
                notes.append(f"image {i}: photometric {m.group(2)}, which GDAL converts to RGB or RGBA; not compared")
    ours, theirs = collections.Counter(bil), collections.Counter(gdal)
    problems = [f"BIL only: {x}" for x in sorted((ours - theirs).elements())]
    problems += [f"GDAL only: {x}" for x in sorted((theirs - ours).elements())]
    if problems:
        category = "differ"
        notes += [l for l in bil_lines if re.match(r"image \d+ layout ", l)]
    else:
        notes += [
            f"image {m.group(1)}: pixels agree"
            for l in bil
            if (m := re.match(r"image (\d+) pixels [0-9a-f]{16}$", l))
        ]
        notes += [
            f"image {m.group(1)}: overview {m.group(2)} agrees"
            for l in bil
            if (m := re.match(r"image (\d+) overview (\d+ x \d+) pixels [0-9a-f]{16}$", l))
        ]
    return category, problems, notes


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    dump, reference, paths = sys.argv[1], sys.argv[2], sys.argv[3:]
    actions = "GITHUB_ACTIONS" in os.environ
    by = collections.defaultdict(list)
    compared = overviews = 0
    details = []
    for path in paths:
        category, problems, notes = compare(run(dump, path), run(reference, path))
        name = os.path.basename(path)
        by[category].append(name)
        if problems:
            details.append(f"{name}: " + "; ".join(problems + [n for n in notes if " layout " in n]))
        compared += sum(n.endswith("pixels agree") for n in notes)
        overviews += sum(n.endswith(" agrees") and " overview " in n for n in notes)
        print(f"{name}: {category}")
        for p in problems:
            print(f"    {p}")
            if actions:
                print(f"::error::{name}: {p}"[:900])
        for n in notes:
            print(f"    {n}")
            if actions and category == "differ":
                print(f"::warning::{name}: {n}"[:900])
    summary = (
        f"GeoTIFF reader against GDAL, {len(paths)} files: {len(by['agree'])} agree, "
        f"{compared} Images and {overviews} stored levels pixel for pixel; {len(by['refused'])} both refuse; "
        f"{len(by['limit'])} hold what BIL does not yet read; "
        f"{len(by['stricter'])} BIL refuses as corrupt where GDAL reads; "
        f"{len(by['lenient'])} GDAL refuses where BIL opens; "
        f"{len(by['converts'])} GDAL converts color BIL returns as stored; {len(by['differ'])} differ"
    )
    print(summary)
    if actions:
        print(f"::notice::{summary}")
        # The files behind each category, in one notice each, so the run can be read without
        # its log.
        for category in ("stricter", "lenient", "converts", "differ"):
            if by[category]:
                print(f"::notice::{category}: {' '.join(by[category])}"[:4000])
        # Each difference in one line, all in one notice, since GitHub shows only the first ten
        # error annotations of a step.
        if details:
            print("::notice::differences: " + " || ".join(details)[:30000])
    sys.exit(1 if by["differ"] else 0)


if __name__ == "__main__":
    main()
