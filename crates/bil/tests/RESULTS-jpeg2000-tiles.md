<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# JPEG 2000 one tile a call

M62, accepted in specification version 80, keeps a JPEG 2000 Image's tiles in the Fragment
cache, each decoded alone, "to be measured … before the choice is final". This is that
measurement, made on 17 October 2026 before the NITF reader was switched to it.

**How.** `examples/jpeg2000_tiles.rs` decodes an area in one call of `bil::codec::jpeg2000::decode`,
then the same area one tile a call, each tile whole, and gives the best of three runs. OpenJPEG
2.5.0 on one thread, on the workspace's x86 machine, release build. The codestreams are written
by `tests/data/jpeg2000/make_fixtures.c timing`: lossless, three resolutions.

    cargo run --release --features openjpeg --example jpeg2000_tiles -- big_gray.j2k big_rgb.j2k

| Codestream | Area | Level | One call | One tile a call | Ratio |
| --- | --- | --- | --- | --- | --- |
| 4096 × 4096 gray, 64 tiles of 512 | Whole Image | 0 | 2091 ms | 2074 ms (64 calls) | 0.99 |
| | One tile | 0 | 33.7 ms | 31.9 ms | 0.95 |
| | Four tiles' corners, one tile's area | 0 | 90.6 ms | 120.7 ms (4) | 1.33 |
| | Whole Image | 2 | 152 ms | 205 ms (64) | 1.35 |
| | One tile | 2 | 3.4 ms | 3.2 ms | 0.93 |
| | Four tiles' corners | 2 | 10.3 ms | 13.4 ms (4) | 1.30 |
| 2048 × 2048 RGB, 64 tiles of 256 | Whole Image | 0 | 1589 ms | 1513 ms (64) | 0.95 |
| | One tile | 0 | 25.2 ms | 24.7 ms | 0.98 |
| | Four tiles' corners | 0 | 98.7 ms | 112.9 ms (4) | 1.14 |
| | Whole Image | 2 | 117 ms | 154 ms (64) | 1.32 |
| | One tile | 2 | 2.7 ms | 2.5 ms | 0.96 |
| | Four tiles' corners | 2 | 8.1 ms | 10.5 ms (4) | 1.29 |

On the larger codestream, reading the main header, which each call repeats, cost 0.13 ms, and
walking the tile-parts 0.01 ms.

**What it shows.** At full resolution, decoding a tile alone costs what decoding it as part of
an area does. A first read whose Window crosses tiles costs 1.14 to 1.33 times as much at full resolution, since each
tile it touches is decoded whole where one call decodes only the code-blocks under the Window;
at level 2, where each tile is cheap, OpenJPEG's own work for each call brings the whole Image
and the corners to 1.29 to 1.35 times. Every later read that touches a kept tile costs a copy,
not measured here. Readers move through an Image by neighboring Windows, which share tiles, so
the cache is expected to repay the first read's extra cost by the second. The NITF reader was
switched to it.
