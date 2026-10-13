// SPDX-License-Identifier: LGPL-2.1-or-later
//! Test images, Windows and comparisons shared by the programs of experiments 1 and 1b.

use crate::{Func, Image, Samples, Slice, Window};

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
                valid: None,
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
                valid: None,
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
                    valid: None,
                }
            } else {
                Image {
                    rows,
                    cols,
                    comps: 1,
                    samples: Samples::ComplexF32(v.iter().map(|x| *x as f32).collect()),
                    valid: None,
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
                    valid: None,
                }
            } else {
                Image {
                    rows,
                    cols,
                    comps: 3,
                    samples: Samples::U16(v.iter().map(|x| *x as u16).collect()),
                    valid: None,
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

/// Gives `img` a validity mask: scattered missing pixels, a missing block, a missing first row
/// and last column, so that every Level meets missing data at the edges and inside.
pub fn with_holes(mut img: Image, seed: u64) -> Image {
    let mut rng = Rng(seed);
    let (rows, cols) = (img.rows, img.cols);
    let mut v = vec![1u8; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            let block = r >= rows / 3
                && r < rows / 3 + rows / 8
                && c >= cols / 4
                && c < cols / 4 + cols / 6;
            if block || r == 0 || c + 1 == cols || rng.below(10) == 0 {
                v[r * cols + c] = 0;
            }
        }
    }
    img.valid = Some(v);
    img
}

/// The Slices of `func` for the whole Image at Levels 0 to `top`, with missing data, made the
/// plain way: whole planes, indexed directly, one Level after another, in scalar code. It shares
/// only the arithmetic of the rolling buffer, not its structure, and so checks it.
pub fn masked_reference(
    img: &Image,
    func: Func,
    top: u32,
    weights: crate::MissingWeights,
) -> Vec<Vec<Slice>> {
    assert!(img.valid.is_some(), "an Image with a validity mask");
    reference(img, func, top, weights, crate::LevelMethod::Bilinear)
}

/// `masked_reference` for a filtering level method, `bilinear` or `binomial5`, and for an Image
/// with or without a mask: with none, every pixel is present. Weights of exactly 1 give the bits
/// of a halving with nothing missing, so this checks both paths of the rolling buffer.
pub fn reference(
    img: &Image,
    func: Func,
    top: u32,
    weights: crate::MissingWeights,
    method: crate::LevelMethod,
) -> Vec<Vec<Slice>> {
    use crate::simd::Isa::Scalar;
    assert!(
        method != crate::LevelMethod::Decimate,
        "the reference makes only filtering methods"
    );
    let ones = vec![1u8; img.rows * img.cols];
    let mask = img.valid.as_ref().unwrap_or(&ones);
    let taps = method.taps(func) as i64;
    let radius = taps / 2;
    // A halving's arithmetic, in the order of the kernels: `halve` for three taps, `blur5` for
    // five.
    let filter = |x: &[f32]| -> f32 {
        if taps == 3 {
            ((x[0] + x[2]) + (x[1] + x[1])) * 0.25
        } else {
            ((x[0] + x[4]) * 0.0625 + (x[1] + x[3]) * 0.25) + x[2] * 0.375
        }
    };
    let (carried, extra) = func.level0_planes();
    let (rows, cols) = (img.rows, img.cols);
    // Level 0 planes, row by row through the Level 0 kernels.
    let mut planes = vec![vec![0f32; rows * cols]; carried + extra];
    let mut inputs = vec![Vec::new(); func.inputs()];
    let mut row = vec![vec![0f32; cols]; carried + extra];
    for r in 0..rows {
        crate::convert_row(img, r, 0, cols - 1, &mut inputs);
        let o: Vec<*mut f32> = row.iter_mut().map(|p| p.as_mut_ptr()).collect();
        // Every row holds cols values.
        unsafe {
            match func {
                Func::Raw => row[0].copy_from_slice(&inputs[0]),
                Func::Polar => crate::polar0(
                    Scalar,
                    cols,
                    inputs[0].as_ptr(),
                    inputs[1].as_ptr(),
                    [o[0], o[1], o[2], o[3]],
                ),
                Func::Hsv => crate::hsv0(
                    Scalar,
                    cols,
                    inputs[0].as_ptr(),
                    inputs[1].as_ptr(),
                    inputs[2].as_ptr(),
                    [o[0], o[1], o[2], o[3], o[4]],
                ),
                Func::Mask | Func::Labels | Func::Range => {
                    unreachable!("the reference makes only the averaged functions")
                }
            }
        }
        for (p, plane) in planes.iter_mut().enumerate() {
            for c in 0..cols {
                plane[r * cols + c] = if mask[r * cols + c] == 0 {
                    f32::NAN
                } else {
                    row[p][c]
                };
            }
        }
    }
    let mut out = Vec::new();
    let (mut h, mut w) = (rows, cols);
    // Each pixel's weight: 1 present, 0 missing at Level 0.
    let mut weight: Vec<f32> = mask
        .iter()
        .map(|&b| if b == 0 { 0.0 } else { 1.0 })
        .collect();
    // A value that is not a number is missing too, in every plane of its pixel.
    for (i, w) in weight.iter_mut().enumerate() {
        if planes.iter().any(|p| p[i].is_nan()) {
            *w = 0.0;
        }
    }
    for plane in planes.iter_mut() {
        for (x, w) in plane.iter_mut().zip(&weight) {
            if *w == 0.0 {
                *x = f32::NAN;
            }
        }
    }
    for k in 0..=top {
        if k > 0 {
            let (h2, w2) = (
                crate::grid_len(rows, k) as usize,
                crate::grid_len(cols, k) as usize,
            );
            let (jr, jc) = (h as i64, w as i64);
            let at = |i: i64, j: i64| crate::reflect(i, j) as usize;
            // The sources of grid point j along an axis of length n, mirrored at the edges.
            let sources = |j: i64, n: i64| -> Vec<usize> {
                (-radius..=radius).map(|d| at(2 * j + d, n)).collect()
            };
            // Along rows, the weights.
            let mut wt = vec![0f32; h * w2];
            for r in 0..h {
                for j in 0..w2 as i64 {
                    let c: Vec<f32> = sources(j, jc).iter().map(|&c| weight[r * w + c]).collect();
                    wt[r * w2 + j as usize] = filter(&c);
                }
            }
            // Down columns, the weights of the new Level.
            let mut q = vec![0f32; h2 * w2];
            for i in 0..h2 as i64 {
                for j in 0..w2 {
                    let c: Vec<f32> = sources(i, jr).iter().map(|&r| wt[r * w2 + j]).collect();
                    q[i as usize * w2 + j] = filter(&c);
                }
            }
            for plane in planes.iter_mut().take(carried) {
                // Along rows: the weighted sum of the present values.
                let mut sum = vec![0f32; h * w2];
                for r in 0..h {
                    for j in 0..w2 as i64 {
                        let x: Vec<f32> = sources(j, jc)
                            .iter()
                            .map(|&c| {
                                let x = plane[r * w + c];
                                (if x.is_nan() { 0.0 } else { x }) * weight[r * w + c]
                            })
                            .collect();
                        sum[r * w2 + j as usize] = filter(&x);
                    }
                }
                // Down columns: combined, then divided once by the new weight.
                let mut next = vec![0f32; h2 * w2];
                for i in 0..h2 as i64 {
                    for j in 0..w2 {
                        let s: Vec<f32> = sources(i, jr).iter().map(|&r| sum[r * w2 + j]).collect();
                        let s = filter(&s);
                        let q = q[i as usize * w2 + j];
                        next[i as usize * w2 + j] = if q == 0.0 { f32::NAN } else { s / q };
                    }
                }
                *plane = next;
            }
            weight = match weights {
                crate::MissingWeights::Equal => {
                    q.iter().map(|&q| if q > 0.0 { 1.0 } else { 0.0 }).collect()
                }
                crate::MissingWeights::PixelCount => q,
            };
            (h, w) = (h2, w2);
        }
        let derive =
            |x: &[f32],
             y: &[f32],
             f: unsafe fn(crate::simd::Isa, usize, *const f32, *const f32, *mut f32)| {
                let mut d = vec![0f32; h * w];
                // x, y and d each hold h × w values.
                unsafe { f(Scalar, h * w, x.as_ptr(), y.as_ptr(), d.as_mut_ptr()) };
                for (d, x) in d.iter_mut().zip(x) {
                    if x.is_nan() {
                        *d = f32::NAN;
                    }
                }
                d
            };
        let data: Vec<Vec<f32>> = match (func, k) {
            (Func::Raw, _) => vec![planes[0].clone()],
            (Func::Polar, 0) => vec![planes[0].clone(), planes[3].clone()],
            (Func::Polar, _) => vec![
                planes[0].clone(),
                derive(&planes[1], &planes[2], crate::phase_from),
            ],
            (Func::Hsv, 0) => vec![planes[4].clone(), planes[0].clone(), planes[1].clone()],
            (Func::Hsv, _) => vec![
                derive(&planes[2], &planes[3], crate::hue_from),
                planes[0].clone(),
                planes[1].clone(),
            ],
            _ => unreachable!("the reference makes only the averaged functions"),
        };
        out.push(
            func.slice_names()
                .iter()
                .zip(data)
                .map(|(&name, data)| Slice {
                    name,
                    level: k,
                    rows: (0, h as i64 - 1),
                    cols: (0, w as i64 - 1),
                    data,
                })
                .collect(),
        );
    }
    out
}
