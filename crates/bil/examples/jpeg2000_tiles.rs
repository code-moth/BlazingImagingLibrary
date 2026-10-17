// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Times decoding a JPEG 2000 codestream one tile a call against one call for a whole area, the
//! measurement M62 asks for before the Fragment cache keeps JPEG 2000 tiles (specification,
//! section 4).
//!
//!     cargo run --release --features openjpeg --example jpeg2000_tiles -- FILE.j2k...

use std::time::{Duration, Instant};

use bil::codec::jpeg2000::{decode, read_header, Format, Request};
use bil::codec::tiles;

fn best(mut f: impl FnMut()) -> Duration {
    (0..3)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .min()
        .unwrap()
}

fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).expect("a readable file");
        let len = bytes.len() as u64;
        let header = read_header(&bytes, 0, len, Format::Codestream).expect("a codestream");
        let t = tiles::read(&bytes, 0, len).expect("its tile-parts").tiling;
        let (across, down) = t.counts();
        let (w, h) = (header.width(), header.height());
        let (tw, th) = (u64::from(t.tile_width), u64::from(t.tile_height));
        let one = |area: (u64, u64, u64, u64), reduce: u32| {
            decode(
                &bytes,
                0,
                len,
                Format::Codestream,
                Request {
                    area: Some(area),
                    reduce,
                },
                &|_| Ok(()),
            )
            .expect("a decode");
        };
        // Every tile touched by an area, as its own area.
        let tiles_of = |a: (u64, u64, u64, u64)| -> Vec<(u64, u64, u64, u64)> {
            let mut v = Vec::new();
            for ty in a.1 / th..a.3.div_ceil(th) {
                for tx in a.0 / tw..a.2.div_ceil(tw) {
                    v.push((
                        tx * tw,
                        ty * th,
                        ((tx + 1) * tw).min(w),
                        ((ty + 1) * th).min(h),
                    ));
                }
            }
            v
        };
        println!("{path}: {w} × {h}, {across} × {down} tiles of {tw} × {th}");
        let walk = best(|| {
            tiles::read(&bytes, 0, len).expect("its tile-parts");
        });
        let head = best(|| {
            read_header(&bytes, 0, len, Format::Codestream).expect("a codestream");
        });
        println!(
            "  each call: walking the tile-parts {:.2} ms, reading the header {:.2} ms",
            walk.as_secs_f64() * 1e3,
            head.as_secs_f64() * 1e3
        );
        let cases = [
            ("whole Image", (0, 0, w, h)),
            ("one tile", (0, 0, tw, th)),
            (
                "four tiles' corners",
                (tw / 2, th / 2, tw / 2 + tw, th / 2 + th),
            ),
        ];
        for reduce in [0u32, 2] {
            for (name, area) in cases {
                let region = best(|| one(area, reduce));
                let parts = tiles_of(area);
                let tiled = best(|| {
                    for &a in &parts {
                        one(a, reduce);
                    }
                });
                println!(
                    "  {name}, level {reduce}: one call {:.1} ms; {} tile calls {:.1} ms; ratio {:.2}",
                    region.as_secs_f64() * 1e3,
                    parts.len(),
                    tiled.as_secs_f64() * 1e3,
                    tiled.as_secs_f64() / region.as_secs_f64()
                );
            }
        }
    }
}
