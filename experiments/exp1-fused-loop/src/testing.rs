// SPDX-License-Identifier: LGPL-2.1-or-later
//! Test images, Windows and comparisons shared by the programs of experiments 1 and 1b.

use crate::{Image, Samples, Slice, Window};

/// A small, fixed random number generator, so every run sees the same data.
pub struct Rng(pub u64);
impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Smooth structure plus noise, so filtering has something to do.
fn field(rng: &mut Rng, r: usize, c: usize) -> f64 {
    let x = r as f64 * 0.013 + (c as f64 * 0.007).sin() * 3.0;
    let y = c as f64 * 0.011 + (r as f64 * 0.005).cos() * 2.0;
    0.5 + 0.3 * (x.sin() * y.cos()) + 0.2 * rng.unit()
}

/// A test Image of the given sample type: `U16`, `F32`, `ComplexI16`, `ComplexF32`, `U8x3` or
/// `U16x3`.
pub fn make(kind: &str, rows: usize, cols: usize, seed: u64) -> Image {
    let mut rng = Rng(seed);
    match kind {
        "U16" => {
            let v = (0..rows * cols)
                .map(|i| (field(&mut rng, i / cols, i % cols) * 4095.0) as u16)
                .collect();
            Image {
                rows,
                cols,
                comps: 1,
                samples: Samples::U16(v),
            }
        }
        "F32" => {
            let v = (0..rows * cols)
                .map(|i| (field(&mut rng, i / cols, i % cols) * 1000.0) as f32)
                .collect();
            Image {
                rows,
                cols,
                comps: 1,
                samples: Samples::F32(v),
            }
        }
        "ComplexI16" | "ComplexF32" => {
            let mut v = Vec::with_capacity(rows * cols * 2);
            for i in 0..rows * cols {
                let a = field(&mut rng, i / cols, i % cols) * 9000.0;
                let p = rng.unit() * std::f64::consts::TAU;
                v.push(a * p.cos());
                v.push(a * p.sin());
            }
            if kind == "ComplexI16" {
                Image {
                    rows,
                    cols,
                    comps: 1,
                    samples: Samples::ComplexI16(v.iter().map(|x| *x as i16).collect()),
                }
            } else {
                Image {
                    rows,
                    cols,
                    comps: 1,
                    samples: Samples::ComplexF32(v.iter().map(|x| *x as f32).collect()),
                }
            }
        }
        "U8x3" | "U16x3" => {
            let scale = if kind == "U8x3" { 255.0 } else { 65535.0 };
            let mut v = Vec::with_capacity(rows * cols * 3);
            for i in 0..rows * cols {
                let base = field(&mut rng, i / cols, i % cols);
                for k in 0..3 {
                    // Some gray pixels, where hue is undefined.
                    let x = if i % 97 == 0 {
                        base
                    } else {
                        (base + 0.25 * k as f64 * rng.unit()).min(1.0)
                    };
                    v.push(x * scale);
                }
            }
            if kind == "U8x3" {
                Image {
                    rows,
                    cols,
                    comps: 3,
                    samples: Samples::U8(v.iter().map(|x| *x as u8).collect()),
                }
            } else {
                Image {
                    rows,
                    cols,
                    comps: 3,
                    samples: Samples::U16(v.iter().map(|x| *x as u16).collect()),
                }
            }
        }
        _ => unreachable!(),
    }
}

/// The Window covering the whole Image.
pub fn full(img: &Image) -> Window {
    Window {
        row_start: 0,
        col_start: 0,
        row_count: img.rows,
        col_count: img.cols,
    }
}

/// Number of values that differ in their bits between two Slices over their common indices.
pub fn bit_diffs(window_slice: &Slice, whole: &Slice) -> usize {
    let mut d = 0;
    for r in window_slice.rows.0..=window_slice.rows.1 {
        for c in window_slice.cols.0..=window_slice.cols.1 {
            if window_slice.at(r, c).to_bits() != whole.at(r, c).to_bits() {
                d += 1;
            }
        }
    }
    d
}

/// The `t`-th random Window of a test: some small, some at each edge, some smaller than a
/// Level's spacing.
pub fn random_window(rng: &mut Rng, rows: usize, cols: usize, t: usize) -> Window {
    let rc = 1 + rng.below(if t.is_multiple_of(4) { 20 } else { 400 });
    let cc = 1 + rng.below(if t.is_multiple_of(4) { 20 } else { 400 });
    let r0 = match t % 5 {
        0 => 0,
        1 => rows - rc,
        _ => rng.below(rows - rc + 1),
    };
    let c0 = match t % 7 {
        0 => 0,
        1 => cols - cc,
        _ => rng.below(cols - cc + 1),
    };
    Window {
        row_start: r0,
        col_start: c0,
        row_count: rc,
        col_count: cc,
    }
}
