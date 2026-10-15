<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# GeoTIFF reader: results

The reader (`bil::geotiff`) against its own tests and against GDAL 3.8.4, as of 15 October 2026.

## Its own tests

`crates/bil/tests/geotiff.rs`, 21 tests, and 4 unit tests in `geotiff/chunks.rs`, in every CI run on all four systems, unoptimized so that an integer overflow fails:

- 660 layouts read whole and through random Windows, sample for sample: every sample type of 8 to 64 bits, F16 among them, one to four bands in turn, strips and tiles, chunky and planar, classic and BigTIFF, both byte orders, uncompressed, Deflate and LZW with and without predictors, and PackBits.
- Integers of 1, 4, 7, 12, 24, 31, 40 and 63 bits, widened; complex samples; a 24-bit float, refused. WhiteIsZero, integer and float, and an extra sample of unspecified meaning, as stored. A first IFD of CIELab or of an unknown interpretation, read; an undefined predictor, listed and refused. Latin-1 text of several strings; no-data rounded to float16.
- Overviews and masks; storage in file order; no-data in every sample and in one; tags and GeoTIFF keys as fields; unsupported compression; claims past the file; a looping IFD chain; chunks never written; a chunk claiming more than its bytes can make, refused before memory is taken; an IFD cut short.
- Layouts whose chunks cannot be counted, out-of-range sample formats, a bit count with no values: each is corrupt, not a panic.
- 20,000 seeded corruptions: each opens or is refused, and each read delivers or fails, with no panic.

## Against GDAL

`testdata/compare_geotiff.py` compares BIL's `geotiff_dump` with GDAL's `geotiff_reference.c` on the 232 TIFF files of GDAL 3.8.4's `autotest/gcore/data` and `autotest/gdrivers/data/gtiff`, in CI's test data job:

| Category | Files | Meaning |
| --- | --- | --- |
| agree | 155 | Images, sizes, sample types, overviews and pixels agree; 160 Images pixel for pixel |
| refused | 16 | Both refuse the file as damaged, or BIL refuses it and GDAL cannot read its pixels |
| limit | 45 | BIL lists the file and refuses, with its reason, what it does not yet read |
| stricter | 6 | BIL refuses as corrupt a file GDAL reads (M42) |
| lenient | 7 | GDAL refuses a file or Image BIL opens |
| converts | 3 | CMYK, CIELab and YCbCr, which GDAL converts to RGB and BIL returns as stored |
| differ | 0 | |

Float16 is compared widened to float32, as GDAL 3.8 reads it, and signed integers of a width of their own unsigned, masked to their width, as GDAL 3.8 reads them (`int24.tif`). Images too large to read whole on a CI runner are listed and not hashed. The comparison runs a release build, where an overflow would wrap rather than fail; the tests above run unoptimized to catch that. A failure of BIL's program counts as a difference.

**Stricter:** `byte_bigtiff_invalid_slong8_for_stripoffsets`, `huge-implied-number-strips`, `minimum_tiff_tags_with_warning`, `one_strip_nobytecount`, `size_of_stripbytecount_at_1_and_lower_than_stripcount`, `size_of_stripbytecount_lower_than_stripcount`: a chunk table of the wrong type, missing, or shorter than its count, which fails the open (M42). GDAL infers what is missing.

**Lenient:** `block_width_above_32bit`, `hugeblocksize`, `image_width_above_32bit`, `second_image_width_above_32bit` (its second Image), `int12_ycbcr_contig`, `uint33`, `unknown_compression`. BIL lists them and refuses their reads where it cannot make them (M35's rule).

**Converts:** `cielab.tif`, `rgbsmall_cmyk.tif`, `ycbcr_11_lzw.tif`: GDAL converts their color; BIL returns the stored samples (section 4).

**What BIL refuses** is now JPEG, ZSTD, WebP, LERC, the fax codes and unknown compression, which wait for their codecs (M40); 24-bit floats and 32-bit complex integers, which have no type; YCbCr subsampled outside JPEG; and tables beyond the `tiff` crate's limits (M41). Version 74's rulings brought LZW, PackBits, other integer widths, complex samples, CIELab, palettes, WhiteIsZero floats, extra samples and unknown interpretations into what is read: the files of those kinds moved from `limit` to `agree` or `converts`. Each refusal's reason is in the CI job's log.

## Found while comparing

- A chunk whose byte count is 0 was never written (`arcgis93_geodataxform_gcp.tif`, a sparse file); its pixels are now missing, and the rest agree with GDAL.
- The `tiff` crate inverts WhiteIsZero samples as it decodes; BIL now reads chunks itself (M39 (b)), so samples come as stored.
- The `tiff` crate drops an extra sample whose meaning is unspecified (`stefan_full_rgba_photometric_rgb.tif`, RGB and a fourth sample); BIL now keeps it.
- SubIFDs hold overviews (`tiff_with_subifds.tif`); they are now stored levels.
- Samples of a width of their own are a stream of bits, most significant first, whatever the file's byte order (`int24.tif`, a little-endian file).
