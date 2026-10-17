// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.
//! Times `read_slice` for `pixel.raw` against a copy, `read_fragment` of the same Window, and
//! against the arithmetic alone, the pass on the same samples held in memory (the speed rule,
//! version 66). An uncompressed 16-bit TIFF in strips is built in memory.
//!
//! `cargo run --release --example read_slice_timing`

use std::time::Instant;

use bil::pass::{Func, Image, LevelMethod, Pass, ReadOptions, Samples};
use bil::simd::Isa;
use bil::types::Window;

const ROWS: u32 = 4096;
const COLS: u32 = 4096;
const STRIP: u32 = 16;

/// A little-endian TIFF of one 16-bit band in strips of `STRIP` rows.
fn tiff() -> (Vec<u8>, Vec<u16>) {
    let values: Vec<u16> = (0..ROWS * COLS)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 17) as u16)
        .collect();
    let strips = ROWS.div_ceil(STRIP);
    let strip_bytes = STRIP * COLS * 2;
    let mut f = b"II*\0".to_vec();
    let ifd = 8u32;
    f.extend(ifd.to_le_bytes());
    let entries: u16 = 9;
    let arrays = ifd + 2 + u32::from(entries) * 12 + 4;
    let offsets_at = arrays;
    let counts_at = arrays + 4 * strips;
    let data_at = counts_at + 4 * strips;
    f.extend(entries.to_le_bytes());
    let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
        f.extend(tag.to_le_bytes());
        f.extend(kind.to_le_bytes());
        f.extend(count.to_le_bytes());
        f.extend(value.to_le_bytes());
    };
    entry(256, 4, 1, COLS);
    entry(257, 4, 1, ROWS);
    entry(258, 3, 1, 16);
    entry(259, 3, 1, 1);
    entry(262, 3, 1, 1);
    entry(273, 4, strips, offsets_at);
    entry(277, 3, 1, 1);
    entry(278, 4, 1, STRIP);
    entry(279, 4, strips, counts_at);
    f.extend(0u32.to_le_bytes());
    for s in 0..strips {
        f.extend((data_at + s * strip_bytes).to_le_bytes());
    }
    for s in 0..strips {
        let rows = STRIP.min(ROWS - s * STRIP);
        f.extend((rows * COLS * 2).to_le_bytes());
    }
    for v in &values {
        f.extend(v.to_le_bytes());
    }
    (f, values)
}

fn best_of<T>(n: usize, mut f: impl FnMut() -> T) -> f64 {
    (0..n)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

fn main() {
    let (file, values) = tiff();
    let source = bil::OpenOptions::default().open_from(file).unwrap();
    source.set_cache_bytes(1 << 30);
    let image = source.image(0).unwrap();
    let window = Window {
        row_start: 0,
        col_start: 0,
        row_count: ROWS,
        col_count: COLS,
    };
    let memory = Image {
        rows: ROWS as usize,
        cols: COLS as usize,
        comps: 1,
        samples: Samples::U16(values),
        valid: None,
    };
    let w = bil::pass::Window {
        row_start: 0,
        col_start: 0,
        row_count: ROWS as usize,
        col_count: COLS as usize,
    };
    let isa = Isa::best();
    println!("{ROWS} × {COLS} U16, strips of {STRIP} rows, {isa:?}; best of 5, warm cache\n");
    println!("| Read | Level 0 (ms) | Level 3 (ms) |");
    println!("| --- | --- | --- |");
    image.read_fragment(window, "level0").unwrap();
    let copy = best_of(5, || image.read_fragment(window, "level0").unwrap());
    println!("| `read_fragment`, a copy | {:.1} | |", copy * 1e3);
    let mut times = Vec::new();
    for n in [0u32, 3] {
        let mut pass = Pass::for_levels(&memory, Func::Raw, w, n, n, LevelMethod::Bilinear);
        let mut out = pass.outputs();
        times.push(best_of(5, || {
            pass.run(&memory, isa, ReadOptions::default(), &mut out);
        }));
    }
    println!(
        "| The pass in memory, the arithmetic alone | {:.1} | {:.1} |",
        times[0] * 1e3,
        times[1] * 1e3
    );
    let slice: Vec<f64> = ["level0", "level3"]
        .iter()
        .map(|level| {
            best_of(5, || {
                image
                    .read_slice("pixel.raw.intensity", window, level)
                    .unwrap()
            })
        })
        .collect();
    println!(
        "| `read_slice` through the cache | {:.1} | {:.1} |",
        slice[0] * 1e3,
        slice[1] * 1e3
    );
}
