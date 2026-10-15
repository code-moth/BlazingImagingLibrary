<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# GeoTIFF reader: results

The reader (`bil::geotiff`) against its own tests and against GDAL 3.8.4, as of 15 October 2026.

## Its own tests

`crates/bil/tests/geotiff.rs`, 15 tests, in every CI run on all four systems, unoptimized so that an integer overflow fails:

- 396 layouts read whole and through random Windows, sample for sample: every sample type the reader reads, F16 among them, one to four bands in turn, strips and tiles, chunky and planar, uncompressed and Deflate, with and without predictors, little-endian, big-endian and BigTIFF.
- Overviews and masks; storage in file order; no-data in every sample and in one; tags and GeoTIFF keys as fields; unsupported compression; claims past the file; a looping IFD chain; chunks never written; WhiteIsZero; an extra sample the `tiff` crate would drop.
- Layouts whose chunks cannot be counted, out-of-range sample formats, a bit count with no values: each is corrupt, not a panic. The checking agent found these three panics, which the seeded corruptions had missed.
- 20,000 seeded corruptions: each opens or is refused, and each read delivers or fails, with no panic.

## Against GDAL

`testdata/compare_geotiff.py` compares BIL's `geotiff_dump` with GDAL's `geotiff_reference.c` on the 232 TIFF files of GDAL 3.8.4's `autotest/gcore/data` and `autotest/gdrivers/data/gtiff`, in CI's test data job:

| Category | Files | Meaning |
| --- | --- | --- |
| agree | 139 | Images, sizes, sample types, overviews and pixels agree; 129 Images pixel for pixel |
| limit | 81 | BIL lists the file and refuses, with its reason, what it does not yet read |
| stricter | 4 | BIL refuses as corrupt a file GDAL reads (M42) |
| lenient | 7 | GDAL refuses a file or Image BIL opens |
| converts | 1 | CMYK, which GDAL converts to RGB and BIL returns as stored |
| differ | 0 | |

Float16 is compared widened to float32, as GDAL 3.8 reads it. Images too large to read whole on a CI runner are listed and not hashed. The comparison runs a release build, where an overflow would wrap rather than fail; the tests above run unoptimized to catch that. A failure of BIL's program counts as a difference.

**Stricter:** `byte_bigtiff_invalid_slong8_for_stripoffsets`, `one_strip_nobytecount`, `size_of_stripbytecount_at_1_and_lower_than_stripcount`, `size_of_stripbytecount_lower_than_stripcount`: a table of the wrong type, missing, or shorter than its count. GDAL infers what is missing.

**Lenient:** `block_width_above_32bit`, `hugeblocksize`, `image_width_above_32bit`, `second_image_width_above_32bit` (its second Image), `int12_ycbcr_contig`, `uint33`, `unknown_compression`. BIL lists them and refuses their reads where it cannot make them (M35's rule).

**What BIL refuses,** over all 232 files, by reason:

| Reason | Images | Proposal |
| --- | --- | --- |
| LZW, JPEG, PackBits, ZSTD, WebP, LERC, fax, unknown compression | 20, 15, 6, 3, 2, 1, 1, 1 | M40 |
| Sample widths other than 8, 16, 32 and 64 bits; complex samples | 22 | M43 |
| What the `tiff` crate alters or refuses: CIELab, palettes, WhiteIsZero floats, a dropped sample, an unknown or missing photometric interpretation | 13 | M39 |
| Tables beyond the `tiff` crate's limits | 2 | M41 |

## Found while comparing

- A chunk whose byte count is 0 was never written (`arcgis93_geodataxform_gcp.tif`, a sparse file); its pixels are now missing, and the rest agree with GDAL.
- The `tiff` crate inverts WhiteIsZero samples as it decodes; integers are inverted back, floats refused.
- The `tiff` crate drops an extra sample whose meaning is unspecified (`stefan_full_rgba_photometric_rgb.tif`, RGB and a fourth sample); the read is now refused rather than short.
- SubIFDs hold overviews (`tiff_with_subifds.tif`); they are now stored levels.
