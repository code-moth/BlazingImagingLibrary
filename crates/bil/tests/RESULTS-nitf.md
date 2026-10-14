<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# Results of the NITF reader's tests

The NITF container reader (`crates/bil/src/nitf/`) is tested three ways. This file records the runs that are not repeated in every CI run, and how to repeat them.

## Synthetic files

`tests/nitf.rs`, in every CI run on all four systems, without optimization: 17 tests over files written field by field by the tests' own writer, beside 6 unit tests of the field definitions, codes and values in the modules. They pass.

## Corrupt files

The test `corrupt_files_are_errors_never_panics` makes corrupt files from two valid ones by seeded mutation: bits flipped, bytes set, a digit changed or a run of digits set to nines or zeros, cuts and insertions. Every file must open or be refused, and every read must deliver or fail, with no panic. Every CI run makes 20,000, without optimization, so an integer overflow panics and fails the test.

A million were run on 14 October 2026, in this workspace, with optimization and overflow checks on:

```sh
BIL_CORRUPT_FILES=1000000 CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true \
  cargo test -p bil --release --test nitf corrupt -- --nocapture
```

```text
corrupt files: 319753 opened, 680247 refused, 530629 Images read whole
test result: ok.
```

The checking agent then found an overflow the mutations never made: a header claiming blocks too large to count in 64 bits (NROWS and NCOLS of 99,999,999 in one block of 30 bands of 64 bits). The reader now refuses such blocks as corrupt, and that file is the test `blocks_too_large_to_count_are_corrupt`. The million were run again after the fix, with the same counts.

## Against GDAL

`testdata/compare_nitf.py` runs `examples/nitf_dump.rs` and the GDAL program `testdata/nitf_reference.c` on each file and compares what they report: the version, the number of Images, header fields, each Image's size and sample type, the extensions present, and an FNV-1a hash of every uncompressed Image's pixels, band after band.

**GDAL's own test files, in this workspace.** On 14 October 2026, against GDAL 3.8.4 built here from its source with no JPEG 2000 driver, on the 19 NITF files in GDAL 3.8.4's `autotest/gdrivers/data/nitf`:

| File | Result |
| --- | --- |
| `SENSRB_TRE.ntf`, `fake_nsif.ntf`, `i_3034c.ntf`, `i_6130a_truncated.ntf`, `invalid_udid.ntf`, `rgb.ntf`, `sar_sicd.ntf`, `two_images_jpeg.ntf`, `valid_udid.ntf`, `U_1050A.NTF`, `U_4017A.NTF` | Agree |
| `i_3034f.ntf`, `ns3034d.nsf` | Agree, with 460 pixels never recorded, which BIL marks missing |
| `U_0006A.NTF`, `ns3114a.nsf` | Agree: no image segments |
| `oss_fuzz_1525.ntf`, `U_0002A.NTF` | Agree: both refuse them |
| `test_jp2_ecw33.ntf`, `two_images_jp2.ntf` | Differ: the GDAL built here cannot open JPEG 2000 |

17 of 19 agree. `i_6130a_truncated.ntf` holds four extensions in a TRE_OVERFLOW segment, and `invalid_udid.ntf` a damaged extension area; both agree only since the reader reads overflow and keeps reading past damage (proposed as M27 in the user manual's chapter 24).

To repeat it, with GDAL installed:

```sh
git clone --depth 1 --branch v3.8.4 --filter=blob:none --sparse https://github.com/OSGeo/gdal gdal-tests
git -C gdal-tests sparse-checkout set autotest/gdrivers/data/nitf
cc testdata/nitf_reference.c -o nitf_reference $(gdal-config --cflags) $(gdal-config --libs)
cargo build --release --example nitf_dump
python3 -I testdata/compare_nitf.py target/release/examples/nitf_dump ./nitf_reference gdal-tests/autotest/gdrivers/data/nitf/*.{ntf,NTF,nsf}
```

**In CI.** The `test data` job runs the same comparison with Ubuntu 24.04's GDAL 3.8.4, which reads JPEG 2000, on the 22 real NITF files of `testdata/manifest.toml` and on GDAL's 19, and raises its summary as notices. First run that passed, on 14 October 2026 (commit 4a08a8d):

| Files | Result |
| --- | --- |
| GDAL's 19 | 19 agree: the two JPEG 2000 files too, now that GDAL can open them |
| The manifest's 22 | 22 agree |

Two of the manifest's files hold blocks never recorded, which BIL marks missing: `v_3301c.ntf`, 329,543 pixels, and `v_3301f.ntf`, 202,608. In `v_3301f.ntf` 12 of its 16 blocks are left out, and there GDAL writes the file's pad value where BIL leaves 0. A pixel never recorded holds no value, so the comparison counts differences only at recorded pixels, where the two agree. The first runs, before the comparison made that distinction, reported `v_3301f.ntf` as differing in 589,824 bytes: exactly those 12 blocks of 128 × 128 pixels in 3 bands.
