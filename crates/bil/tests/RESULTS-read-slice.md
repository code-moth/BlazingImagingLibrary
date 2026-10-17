<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# `read_slice` against a copy and the arithmetic alone

`cargo run --release --example read_slice_timing` (17 October 2026, this workspace: Linux on x86, two cores, AVX-512). One 4096 × 4096 band of 16-bit samples, uncompressed, in strips of 16 rows, built in memory; the cache warm and large enough to hold every strip; best of five.

| Read | Level 0 (ms) | Level 3 (ms) |
| --- | --- | --- |
| `read_fragment`, a copy | 57.8 | |
| The pass in memory, the arithmetic alone | 13.2 | 8.3 |
| `read_slice` through the cache | 80.8 | 53.2 |

- **`read_slice` costs a copy and the arithmetic.** At Level 0 it is the copy and the pass added, with about 10 ms besides for reading row by row. At Level 3 `bilinear` still reads every pixel under the Window: 53.2 ms against the arithmetic's 8.3 ms, the rest reading and copying the bands.
- **The copy is the cost to remove.** The row source reads each band with `read_fragment`, which copies the cached strips into a Fragment before the pass converts them (M77). Reading the cached blocks in place, converting each row straight from them, would leave the arithmetic and one conversion. That is the next step for speed, and `read_fragment` itself, at about 0.6 GB/s here, is worth measuring on its own.
- **One thread.** Bands on threads (`BandedPass`) are not used yet (M77).
