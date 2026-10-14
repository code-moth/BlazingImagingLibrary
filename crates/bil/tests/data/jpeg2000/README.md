<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# JPEG 2000 test codestreams

Small lossless codestreams for the tests of `bil::codec::jpeg2000` (`tests/jpeg2000.rs`), written
by `make_fixtures.c` with OpenJPEG 2.5.0's encoder. Each sample is set by `value` in that file,
which the tests repeat, so a decoded sample can be checked without a second decoder.

| File | Size | Components | Precision | Other |
|---|---|---|---|---|
| `gray8.j2k` | 37 × 29 | 1 | 8, unsigned | 16 × 16 tiles |
| `gray8.jp2` | 37 × 29 | 1 | 8, unsigned | the same in a JP2 file |
| `rgb8.j2k` | 23 × 19 | 3 | 8, unsigned | |
| `gray16.j2k` | 21 × 17 | 1 | 16, unsigned | |
| `signed12.j2k` | 20 × 18 | 1 | 12, signed | |

To write them again, with OpenJPEG's header and library installed:

    cc make_fixtures.c -lopenjp2 -o make_fixtures && ./make_fixtures
