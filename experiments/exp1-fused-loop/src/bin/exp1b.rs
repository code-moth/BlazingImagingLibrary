// SPDX-License-Identifier: LGPL-2.1-or-later
//! Runs experiment 1b, the rolling buffer, and prints its report.
//!
//! `--check` runs the correctness tests: experiment 1's bitwise tests through the rolling buffer,
//! small Images, reused buffers, missing data, the other kinds of Slice, and `binomial5`. The full report adds speed, measured against a
//! copy into warm memory and against each function's own arithmetic, as section 12 asks.

use bil_exp1::simd::Isa;
use bil_exp1::testing::{
    bit_diffs, full, make, masked_reference, random_window, reference, with_holes, Rng,
};
use bil_exp1::{
    band_threads, hsv0, plan_outputs, polar0, read_levels, read_levels_into, BandedPass, Func,
    Image, LevelMethod, MissingWeights, Pass, ReadOptions, Slice, Window, MIN_BAND_PIXELS,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Counts every allocation the program makes, so that a check can hold a read to allocating
/// nothing.
struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// Every call goes straight to the system allocator; only the count is added.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const TOP: u32 = 4;

/// Reads through the rolling buffer into freshly planned buffers.
fn rolling(img: &Image, func: Func, isa: Isa, window: Window, top: u32) -> Vec<Vec<Slice>> {
    rolling_with(img, func, isa, window, top, ReadOptions::default())
}

fn rolling_with(
    img: &Image,
    func: Func,
    isa: Isa,
    window: Window,
    top: u32,
    options: ReadOptions,
) -> Vec<Vec<Slice>> {
    let mut out = plan_outputs(img, func, window, top);
    read_levels_into(img, func, isa, window, top, options, &mut out);
    out
}

/// Values whose bits differ between two results of the same request, which must also have the
/// same shape.
fn differ(a: &[Vec<Slice>], b: &[Vec<Slice>]) -> usize {
    assert_eq!(a.len(), b.len(), "the two results have different Levels");
    let mut d = 0;
    for (la, lb) in a.iter().zip(b) {
        assert_eq!(la.len(), lb.len());
        for (x, y) in la.iter().zip(lb) {
            assert!(x.name == y.name && x.rows == y.rows && x.cols == y.cols);
            d += x
                .data
                .iter()
                .zip(&y.data)
                .filter(|(p, q)| p.to_bits() != q.to_bits())
                .count();
        }
    }
    d
}

fn main() {
    let isas = Isa::available();
    println!("# Experiment 1b report\n");
    println!("Instruction sets available: {isas:?}. Levels 0 to {TOP} in one pass, on a rolling buffer of three rows for each Level above Level 0, five for `binomial5`. Single thread.\n");

    let cases = [
        (Func::Raw, "U16"),
        (Func::Raw, "F32"),
        (Func::Polar, "ComplexI16"),
        (Func::Polar, "ComplexF32"),
        (Func::Hsv, "U8x3"),
        (Func::Hsv, "U16x3"),
    ];
    let (rows, cols) = (1037, 1291);
    let windows_per_case = 400;

    println!("## 1. Bitwise agreement\n");
    println!("Experiment 1's tests, unchanged, through the rolling buffer: Image {rows} × {cols}, the same {windows_per_case} random Windows for each case, every Level. Each Window is also compared with experiment 1's whole planes.\n");
    println!("| Function | Sample type | Instruction set | Values compared | Differing bits, Window vs whole | Differing bits, vs scalar | Differing bits, vs whole planes |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    let mut all_ok = true;
    for (case_no, (func, kind)) in cases.iter().enumerate() {
        let img = make(kind, rows, cols, 7 + case_no as u64);
        let scalar_whole = rolling(&img, *func, Isa::Scalar, full(&img), TOP);
        for isa in &isas {
            let whole = rolling(&img, *func, *isa, full(&img), TOP);
            let vs_scalar = differ(&whole, &scalar_whole);
            let mut vs_planes = differ(&whole, &read_levels(&img, *func, *isa, full(&img), TOP));
            let mut rng = Rng(1000 + case_no as u64);
            let (mut compared, mut diffs) = (0usize, 0usize);
            for t in 0..windows_per_case {
                let win = random_window(&mut rng, rows, cols, t);
                let got = rolling(&img, *func, *isa, win, TOP);
                vs_planes += differ(&got, &read_levels(&img, *func, *isa, win, TOP));
                for (lw, lwhole) in got.iter().zip(&whole) {
                    for (s, w) in lw.iter().zip(lwhole) {
                        assert_eq!(s.name, w.name);
                        compared += s.data.len();
                        diffs += bit_diffs(s, w);
                    }
                }
            }
            all_ok &= diffs == 0 && vs_scalar == 0 && vs_planes == 0;
            println!(
                "| `{}` | {kind} | {isa:?} | {compared} | {diffs} | {vs_scalar} | {vs_planes} |",
                func.name()
            );
        }
    }
    println!("\nAll bitwise checks passed: **{all_ok}**\n");

    all_ok &= small_images(&isas);
    all_ok &= reuse(&isas);
    all_ok &= missing_data(&isas, &cases, rows, cols);
    all_ok &= other_kinds(&isas, rows, cols);
    all_ok &= binomial(&isas, &cases, rows, cols);
    all_ok &= bands(&isas, &cases, rows, cols);
    all_ok &= coarse_alone(&isas, &cases, rows, cols);

    if std::env::args().any(|a| a == "--check") {
        if !all_ok {
            std::process::exit(1);
        }
        return;
    }
    benchmarks(&isas);
}

/// Every Window of small Images, with more Levels asked for than the Image has, where the ring
/// and the mirroring meet their edge cases: single rows and columns, and Levels of one point.
fn small_images(isas: &[Isa]) -> bool {
    println!("## 2. Small Images\n");
    let sizes = [1, 2, 3, 5, 9];
    let (mut reads, mut diffs) = (0usize, 0usize);
    for (func, kind) in [
        (Func::Raw, "U16"),
        (Func::Polar, "ComplexF32"),
        (Func::Hsv, "U8x3"),
    ] {
        for &rows in &sizes {
            for &cols in &sizes {
                let img = make(kind, rows, cols, (rows * 31 + cols) as u64);
                for isa in isas {
                    for top in 0..=6 {
                        for r0 in 0..rows {
                            for c0 in 0..cols {
                                for rc in 1..=rows - r0 {
                                    for cc in 1..=cols - c0 {
                                        let win = Window {
                                            row_start: r0,
                                            col_start: c0,
                                            row_count: rc,
                                            col_count: cc,
                                        };
                                        let got = rolling(&img, func, *isa, win, top);
                                        diffs +=
                                            differ(&got, &read_levels(&img, func, *isa, win, top));
                                        reads += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let ok = diffs == 0;
    println!("Images of 1, 2, 3, 5 and 9 rows by 1, 2, 3, 5 and 9 columns, every Window, Levels 0 to 0 up to 0 to 6, against experiment 1's whole planes: {reads} reads, {diffs} values differing in their bits. Passed: **{ok}**\n");
    ok
}

/// Buffers filled with NaN and read into again must come back complete and equal, so a read
/// writes every value it owns and depends on nothing left in the buffers.
fn reuse(isas: &[Isa]) -> bool {
    println!("## 3. Reused buffers\n");
    let mut ok = true;
    for (func, kind) in [
        (Func::Raw, "U16"),
        (Func::Polar, "ComplexF32"),
        (Func::Hsv, "U8x3"),
    ] {
        let img = make(kind, 333, 517, 21);
        for isa in isas {
            let win = Window {
                row_start: 5,
                col_start: 9,
                row_count: 300,
                col_count: 480,
            };
            let mut out = plan_outputs(&img, func, win, TOP);
            read_levels_into(&img, func, *isa, win, TOP, ReadOptions::default(), &mut out);
            let first: Vec<Vec<Vec<u32>>> = out
                .iter()
                .map(|l| {
                    l.iter()
                        .map(|s| s.data.iter().map(|v| v.to_bits()).collect())
                        .collect()
                })
                .collect();
            for s in out.iter_mut().flatten() {
                s.data.fill(f32::NAN);
            }
            read_levels_into(&img, func, *isa, win, TOP, ReadOptions::default(), &mut out);
            let same = out.iter().zip(&first).all(|(l, f)| {
                l.iter().zip(f).all(|(s, fb)| {
                    s.data
                        .iter()
                        .zip(fb)
                        .all(|(v, b)| v.to_bits() == *b && !v.is_nan())
                })
            });
            ok &= same;
        }
    }
    println!(
        "Every value rewritten, and equal to the first read, on every instruction set: **{ok}**\n"
    );
    ok
}

/// Missing data: NaN in every Slice of a missing pixel, NaN skipped with the weights rescaled at
/// every halving, and `missing_value` applied last. Checked bitwise against a plain reference
/// made from whole planes, between Windows and the whole Image, and between instruction sets.
fn missing_data(isas: &[Isa], cases: &[(Func, &str)], rows: usize, cols: usize) -> bool {
    println!("## 4. Missing data\n");
    println!("The same Images with a validity mask: about one pixel in ten missing at random, a missing block, and a missing first row and last column. The reference makes whole planes, one Level after another, in scalar code.\n");
    println!("| Function | Sample type | `missing_weights` | Instruction set | Values compared | Missing at Level 0 | Missing at Level {TOP} | Whole vs reference | Windows vs reference | No mask vs all present | `missing_value` |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    let mut ok = true;
    for weights in [MissingWeights::Equal, MissingWeights::PixelCount] {
        let base = ReadOptions {
            missing_weights: weights,
            ..ReadOptions::default()
        };
        for (case_no, (func, kind)) in cases.iter().enumerate() {
            let plain = make(kind, rows, cols, 7 + case_no as u64);
            let img = with_holes(
                make(kind, rows, cols, 7 + case_no as u64),
                50 + case_no as u64,
            );
            let mut all_present = make(kind, rows, cols, 7 + case_no as u64);
            all_present.valid = Some(vec![1u8; rows * cols]);
            let reference = masked_reference(&img, *func, TOP, weights);
            let nan_count = |l: &Vec<Slice>| l[0].data.iter().filter(|x| x.is_nan()).count();
            let (missing0, missing_top) = (
                nan_count(&reference[0]),
                nan_count(&reference[TOP as usize]),
            );
            for isa in isas {
                let whole = rolling_with(&img, *func, *isa, full(&img), TOP, base);
                let vs_reference = differ(&whole, &reference);
                // Windows, against the reference at the same grid points.
                let mut rng = Rng(2000 + case_no as u64);
                let (mut compared, mut window_diffs) = (0usize, 0usize);
                for t in 0..200 {
                    let win = random_window(&mut rng, rows, cols, t);
                    let got = rolling_with(&img, *func, *isa, win, TOP, base);
                    for (lw, lref) in got.iter().zip(&reference) {
                        for (s, r) in lw.iter().zip(lref) {
                            compared += s.data.len();
                            window_diffs += bit_diffs(s, r);
                        }
                    }
                }
                // A mask with nothing missing gives exactly the values of no mask at all.
                let present = differ(
                    &rolling_with(&all_present, *func, *isa, full(&img), TOP, base),
                    &rolling(&plain, *func, *isa, full(&img), TOP),
                );
                // The caller's number replaces NaN and nothing else.
                let fill = ReadOptions {
                    missing_value: Some(-7.0),
                    ..base
                };
                let filled = rolling_with(&img, *func, *isa, full(&img), TOP, fill);
                let mut fill_ok = true;
                for (lf, lw) in filled.iter().zip(&whole) {
                    for (f, w) in lf.iter().zip(lw) {
                        for (a, b) in f.data.iter().zip(&w.data) {
                            let expect = if b.is_nan() { -7.0f32 } else { *b };
                            fill_ok &= a.to_bits() == expect.to_bits();
                        }
                    }
                }
                ok &= vs_reference == 0 && window_diffs == 0 && present == 0 && fill_ok;
                println!(
                "| `{}` | {kind} | `{weights:?}` | {isa:?} | {compared} | {missing0} | {missing_top} | {vs_reference} | {window_diffs} | {present} | {} |",
                func.name(),
                if fill_ok { "applied last" } else { "**wrong**" }
            );
            }
        }
    }
    // Small Images with random masks, including some with every pixel missing.
    let (mut reads, mut diffs) = (0usize, 0usize);
    let sizes = [1, 2, 3, 5, 9];
    for (func, kind) in [
        (Func::Raw, "U16"),
        (Func::Polar, "ComplexF32"),
        (Func::Hsv, "U8x3"),
    ] {
        for &h in &sizes {
            for &w in &sizes {
                let mut img = make(kind, h, w, (h * 31 + w) as u64);
                let mut rng = Rng((h * 7 + w) as u64);
                let every = (h + w) % 4 == 0;
                img.valid = Some(
                    (0..h * w)
                        .map(|_| u8::from(!every && rng.below(5) >= 2))
                        .collect(),
                );
                for weights in [MissingWeights::Equal, MissingWeights::PixelCount] {
                    let base = ReadOptions {
                        missing_weights: weights,
                        ..ReadOptions::default()
                    };
                    let reference = masked_reference(&img, func, 6, weights);
                    for isa in isas {
                        for top in 0..=6 {
                            for r0 in 0..h {
                                for c0 in 0..w {
                                    for rc in 1..=h - r0 {
                                        for cc in 1..=w - c0 {
                                            let win = Window {
                                                row_start: r0,
                                                col_start: c0,
                                                row_count: rc,
                                                col_count: cc,
                                            };
                                            for (lw, lref) in
                                                rolling_with(&img, func, *isa, win, top, base)
                                                    .iter()
                                                    .zip(&reference)
                                            {
                                                for (s, r) in lw.iter().zip(lref) {
                                                    diffs += bit_diffs(s, r);
                                                }
                                            }
                                            reads += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let small_ok = diffs == 0;
    ok &= small_ok;
    println!("\nSmall Images of 1, 2, 3, 5 and 9 rows by 1, 2, 3, 5 and 9 columns, with masks missing two pixels in five or every pixel, every Window, both weightings, against the reference: {reads} reads, {diffs} values differing in their bits. Passed: **{small_ok}**\n");
    // A NaN stored at a pixel the mask calls valid counts as missing: the result is exactly that
    // of masking the pixel instead, for both weightings and for the mask Slices.
    let mut stored_diffs = 0;
    for (func, kind) in [
        (Func::Raw, "F32"),
        (Func::Polar, "ComplexF32"),
        (Func::Mask, "F32"),
        (Func::Mask, "ComplexF32"),
    ] {
        let base = with_holes(make(kind, rows, cols, 70), 71);
        let mut stored = with_holes(make(kind, rows, cols, 70), 71);
        let mut masked = with_holes(make(kind, rows, cols, 70), 71);
        let mut rng = Rng(72);
        let per = if kind == "ComplexF32" { 2 } else { 1 };
        for i in 0..rows * cols {
            if base.valid.as_ref().unwrap()[i] != 0 && rng.below(50) == 0 {
                if let bil_exp1::Samples::F32(v) | bil_exp1::Samples::ComplexF32(v) =
                    &mut stored.samples
                {
                    // In either component of a complex sample.
                    v[i * per + i % per] = f32::NAN;
                }
                masked.valid.as_mut().unwrap()[i] = 0;
            }
        }
        for weights in [MissingWeights::Equal, MissingWeights::PixelCount] {
            let options = ReadOptions {
                missing_weights: weights,
                ..ReadOptions::default()
            };
            for isa in isas {
                stored_diffs += differ(
                    &rolling_with(&stored, func, *isa, full(&stored), TOP, options),
                    &rolling_with(&masked, func, *isa, full(&masked), TOP, options),
                );
            }
        }
    }
    let stored_ok = stored_diffs == 0;
    ok &= stored_ok;
    println!("NaN stored at about one valid pixel in fifty, in either component of F32 and ComplexF32 data, against the same pixels masked instead, for `pixel.raw`, `complex.polar` and `pixel.mask`, both weightings, every instruction set: {stored_diffs} values differing in their bits. Passed: **{stored_ok}**\n");
    // Near the top of the float32 range, a mask with nothing missing must still change nothing.
    let mut big = make("F32", 97, 131, 3);
    if let bil_exp1::Samples::F32(v) = &mut big.samples {
        for x in v.iter_mut() {
            *x *= 3.0e34;
        }
    }
    let mut big_masked = make("F32", 97, 131, 3);
    big_masked.samples = match &big.samples {
        bil_exp1::Samples::F32(v) => bil_exp1::Samples::F32(v.clone()),
        _ => unreachable!(),
    };
    big_masked.valid = Some(vec![1u8; 97 * 131]);
    let mut range_diffs = 0;
    for isa in isas {
        range_diffs += differ(
            &rolling(&big_masked, Func::Raw, *isa, full(&big), TOP),
            &rolling(&big, Func::Raw, *isa, full(&big), TOP),
        );
    }
    let range_ok = range_diffs == 0;
    ok &= range_ok;
    println!("Values up to about 2.5 × 10³⁷, near the top of the float32 range, with a mask that has nothing missing, against no mask, on every instruction set: {range_diffs} values differing in their bits. Passed: **{range_ok}**\n");
    // `PixelCount` is the pyramid of the masked data divided by the pyramid of the mask: each
    // Level is the average of the present full-resolution pixels under the whole filter.
    let img = with_holes(make("F32", rows, cols, 8), 51);
    let counted = rolling_with(
        &img,
        Func::Raw,
        Isa::best(),
        full(&img),
        TOP,
        ReadOptions {
            missing_weights: MissingWeights::PixelCount,
            ..ReadOptions::default()
        },
    );
    let mask = img.valid.as_ref().unwrap();
    let bil_exp1::Samples::F32(v) = &img.samples else {
        unreachable!()
    };
    let as_image = |data: Vec<f32>| bil_exp1::Image {
        rows,
        cols,
        comps: 1,
        samples: bil_exp1::Samples::F32(data),
        valid: None,
    };
    let masked_data = as_image(
        v.iter()
            .zip(mask)
            .map(|(x, &m)| if m == 0 { 0.0 } else { *x })
            .collect(),
    );
    let mask_image = as_image(mask.iter().map(|&m| f32::from(m)).collect());
    let numerator = rolling(&masked_data, Func::Raw, Isa::best(), full(&img), TOP);
    let denominator = rolling(&mask_image, Func::Raw, Isa::best(), full(&img), TOP);
    let (mut worst, mut nan_agree, mut equal_differs) = (0f64, true, 0usize);
    let equal = rolling(&img, Func::Raw, Isa::best(), full(&img), TOP);
    for k in 0..=TOP as usize {
        let (got, num, den) = (
            &counted[k][0].data,
            &numerator[k][0].data,
            &denominator[k][0].data,
        );
        for i in 0..got.len() {
            if den[i] == 0.0 {
                nan_agree &= got[i].is_nan();
            } else {
                let want = num[i] as f64 / den[i] as f64;
                worst = worst.max(((got[i] as f64 - want) / want.abs().max(1.0)).abs());
            }
            if got[i].to_bits() != equal[k][0].data[i].to_bits() {
                equal_differs += 1;
            }
        }
    }
    let counted_ok = nan_agree && worst < 1e-5 && equal_differs > 0;
    ok &= counted_ok;
    println!("`PixelCount` against the unmasked pyramid of the data with missing pixels as 0, divided by the unmasked pyramid of the mask, at Levels 0 to {TOP}: largest relative difference {worst:.1e}; NaN exactly where the mask's pyramid is 0: {nan_agree}. It differs from `Equal` at {equal_differs} values, near missing pixels. Passed: **{counted_ok}**\n");
    // The rule itself, in float64: Level 1 is the 1 2 1 by 1 2 1 average of the pixels present
    // around each grid point, weights rescaled, and NaN only where none is present.
    let img = with_holes(make("U16", rows, cols, 7), 50);
    let got = rolling(&img, Func::Raw, Isa::best(), full(&img), 1);
    let bil_exp1::Samples::U16(v) = &img.samples else {
        unreachable!()
    };
    let mask = img.valid.as_ref().unwrap();
    let mirror = |i: i64, n: i64| {
        let p = 2 * (n - 1);
        let m = i.rem_euclid(p);
        if m >= n {
            p - m
        } else {
            m
        }
    };
    let (mut worst, mut nan_agree) = (0f64, true);
    let l1 = &got[1][0];
    for i in l1.rows.0..=l1.rows.1 {
        for j in l1.cols.0..=l1.cols.1 {
            let (mut sum, mut weight) = (0f64, 0f64);
            for (di, wi) in [(-1i64, 1f64), (0, 2.0), (1, 1.0)] {
                for (dj, wj) in [(-1i64, 1f64), (0, 2.0), (1, 1.0)] {
                    let r = mirror(2 * i + di, rows as i64) as usize;
                    let c = mirror(2 * j + dj, cols as i64) as usize;
                    if mask[r * cols + c] != 0 {
                        sum += wi * wj * v[r * cols + c] as f64;
                        weight += wi * wj;
                    }
                }
            }
            let x = l1.at(i, j) as f64;
            if weight == 0.0 {
                nan_agree &= x.is_nan();
            } else {
                nan_agree &= !x.is_nan();
                worst = worst.max(((x - sum / weight) / (sum / weight).max(1.0)).abs());
            }
        }
    }
    let rule_ok = nan_agree && worst < 1e-6;
    ok &= rule_ok;
    println!("The rule itself: Level 1 of `pixel.raw` against the average of the present pixels with weights 1 2 1 by 1 2 1, rescaled, computed in float64. Largest relative difference {worst:.1e}; NaN exactly where no pixel is present: {nan_agree}. Passed: **{rule_ok}**\n");
    println!("All missing-data checks passed: **{ok}**\n");
    ok
}

/// Bitwise differences between every Window's Slices and the whole Image's, at the same grid
/// points, over the random Windows of the other tests, and between this instruction set and
/// scalar code on the whole Image.
fn independence(
    img: &Image,
    func: Func,
    isa: Isa,
    options: ReadOptions,
    seed: u64,
) -> (usize, usize, usize) {
    let whole = rolling_with(img, func, isa, full(img), TOP, options);
    let vs_scalar = differ(
        &whole,
        &rolling_with(img, func, Isa::Scalar, full(img), TOP, options),
    );
    let mut rng = Rng(seed);
    let (mut compared, mut diffs) = (0, 0);
    for t in 0..200 {
        let win = random_window(&mut rng, img.rows, img.cols, t);
        for (lw, lwhole) in rolling_with(img, func, isa, win, TOP, options)
            .iter()
            .zip(&whole)
        {
            for (s, w) in lw.iter().zip(lwhole) {
                compared += s.data.len();
                diffs += bit_diffs(s, w);
            }
        }
    }
    (compared, diffs, vs_scalar)
}

/// Values at Level k that differ from Level 0's value at the same grid point, which is what
/// keeping every second value means.
fn kept_diffs(levels: &[Vec<Slice>]) -> usize {
    let mut d = 0;
    for l in &levels[1..] {
        for (s, s0) in l.iter().zip(&levels[0]) {
            let step = 1i64 << s.level;
            for i in s.rows.0..=s.rows.1 {
                for j in s.cols.0..=s.cols.1 {
                    d += usize::from(s.at(i, j).to_bits() != s0.at(i * step, j * step).to_bits());
                }
            }
        }
    }
    d
}

/// Range reduced the plain way: whole planes, each Level the least present value of the 3 × 3
/// block of the Level before, mirrored at the edges.
fn range_reference(img: &Image, top: u32) -> Vec<Vec<f32>> {
    let (mut h, mut w) = (img.rows, img.cols);
    let bil_exp1::Samples::F32(v) = &img.samples else {
        unreachable!()
    };
    let mut plane: Vec<f32> = match &img.valid {
        Some(m) => v
            .iter()
            .zip(m)
            .map(|(x, &b)| if b == 0 { f32::NAN } else { *x })
            .collect(),
        None => v.clone(),
    };
    let mirror = |i: i64, n: usize| {
        let n = n as i64;
        if n == 1 {
            return 0;
        }
        let p = 2 * (n - 1);
        let m = i.rem_euclid(p);
        (if m >= n { p - m } else { m }) as usize
    };
    let least = |x: [f32; 3]| {
        x.into_iter()
            .filter(|v| !v.is_nan())
            .fold(f32::NAN, |a, b| if a.is_nan() || b < a { b } else { a })
    };
    let mut levels = vec![plane.clone()];
    for _ in 0..top {
        let (h2, w2) = ((h - 1) / 2 + 1, (w - 1) / 2 + 1);
        let mut along = vec![0f32; h * w2];
        for r in 0..h {
            for j in 0..w2 {
                let j = j as i64;
                along[r * w2 + j as usize] =
                    least([2 * j - 1, 2 * j, 2 * j + 1].map(|c| plane[r * w + mirror(c, w)]));
            }
        }
        let mut next = vec![0f32; h2 * w2];
        for i in 0..h2 {
            for j in 0..w2 {
                let i = i as i64;
                next[i as usize * w2 + j] =
                    least([2 * i - 1, 2 * i, 2 * i + 1].map(|r| along[mirror(r, h) * w2 + j]));
            }
        }
        (plane, h, w) = (next, h2, w2);
        levels.push(plane.clone());
    }
    levels
}

/// The kinds of Slice that are not averaged, and `decimate`: labels keep every second value,
/// range takes the least present value of each 3 × 3 block, the mask Slices are the averaged
/// mask, and `decimate` keeps the value at each grid point for every kind.
fn other_kinds(isas: &[Isa], rows: usize, cols: usize) -> bool {
    println!("## 5. Other kinds of Slice, and `decimate`\n");
    println!("Each is checked against its definition in section 5 of the specification, between 200 random Windows and the whole Image, and against scalar code.\n");
    println!("| Slice | Image | Level method | Instruction set | Values compared | Against the definition | Window vs whole | Against scalar |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    let decimate = ReadOptions {
        level_method: LevelMethod::Decimate,
        ..ReadOptions::default()
    };
    let bilinear = ReadOptions::default();
    let mut ok = true;
    let mut row =
        |name: &str, image: &str, method: &str, isa: Isa, c: (usize, usize, usize), def: usize| {
            ok &= c.1 == 0 && c.2 == 0 && def == 0;
            println!(
                "| `{name}` | {image} | `{method}` | {isa:?} | {} | {def} | {} | {} |",
                c.0, c.1, c.2
            );
        };
    let labels = make("U16", rows, cols, 31);
    let labels_holes = with_holes(make("U16", rows, cols, 31), 32);
    let depth = make("F32", rows, cols, 33);
    let depth_holes = with_holes(make("F32", rows, cols, 33), 34);
    let holes = with_holes(make("U16", rows, cols, 35), 36);
    for isa in isas {
        // Labels keep every second value, with or without missing data.
        for (img, image) in [(&labels, "U16"), (&labels_holes, "U16, with holes")] {
            let whole = rolling_with(img, Func::Labels, *isa, full(img), TOP, bilinear);
            row(
                "class.labels.value",
                image,
                "bilinear",
                *isa,
                independence(img, Func::Labels, *isa, bilinear, 41),
                kept_diffs(&whole),
            );
        }
        // `decimate` keeps the value at each grid point, for every kind of Slice.
        for (img, func, image) in [
            (make("U16", rows, cols, 7), Func::Raw, "U16"),
            (make("ComplexI16", rows, cols, 9), Func::Polar, "ComplexI16"),
            (make("U8x3", rows, cols, 11), Func::Hsv, "U8 color"),
            (
                with_holes(make("U16", rows, cols, 7), 42),
                Func::Raw,
                "U16, with holes",
            ),
            (
                with_holes(make("F32", rows, cols, 33), 43),
                Func::Range,
                "F32, with holes",
            ),
        ] {
            let whole = rolling_with(&img, func, *isa, full(&img), TOP, decimate);
            let names = func
                .slice_names()
                .iter()
                .map(|n| format!("{}.{n}", func.name()))
                .collect::<Vec<_>>()
                .join("`, `");
            row(
                &names,
                image,
                "decimate",
                *isa,
                independence(&img, func, *isa, decimate, 44),
                kept_diffs(&whole),
            );
        }
        // Range: the least present value of each 3 × 3 block, against whole planes.
        for (img, image) in [(&depth, "F32"), (&depth_holes, "F32, with holes")] {
            let whole = rolling_with(img, Func::Range, *isa, full(img), TOP, bilinear);
            let reference = range_reference(img, TOP);
            let def: usize = whole
                .iter()
                .zip(&reference)
                .map(|(l, r)| {
                    l[0].data
                        .iter()
                        .zip(r)
                        .filter(|(a, b)| a.to_bits() != b.to_bits())
                        .count()
                })
                .sum();
            row(
                "depth.sensor.range",
                image,
                "bilinear",
                *isa,
                independence(img, Func::Range, *isa, bilinear, 45),
                def,
            );
        }
        // The mask Slices: `valid_fraction` is the mask averaged as it is, and `valid` is 1.0
        // wherever it is above 0. With no mask, every value is 1.0.
        let mask_as_image = Image {
            rows,
            cols,
            comps: 1,
            samples: bil_exp1::Samples::F32(
                holes
                    .valid
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|&b| f32::from(b))
                    .collect(),
            ),
            valid: None,
        };
        let averaged = rolling_with(&mask_as_image, Func::Raw, *isa, full(&holes), TOP, bilinear);
        let got = rolling_with(&holes, Func::Mask, *isa, full(&holes), TOP, bilinear);
        let mut def = 0;
        for (l, a) in got.iter().zip(&averaged) {
            for (v, f, x) in l[0]
                .data
                .iter()
                .zip(&l[1].data)
                .zip(&a[0].data)
                .map(|((v, f), x)| (v, f, x))
            {
                def += usize::from(f.to_bits() != x.to_bits());
                def += usize::from(*v != if *x > 0.0 { 1.0 } else { 0.0 });
            }
        }
        row(
            "pixel.mask.valid`, `pixel.mask.valid_fraction",
            "U16, with holes",
            "bilinear",
            *isa,
            independence(&holes, Func::Mask, *isa, bilinear, 46),
            def,
        );
        let plain = make("U16", rows, cols, 35);
        let ones = rolling_with(&plain, Func::Mask, *isa, full(&plain), TOP, bilinear);
        let def = ones
            .iter()
            .flatten()
            .flat_map(|s| s.data.iter())
            .filter(|v| **v != 1.0)
            .count();
        row(
            "pixel.mask.valid`, `pixel.mask.valid_fraction",
            "U16, no mask",
            "bilinear",
            *isa,
            independence(&plain, Func::Mask, *isa, bilinear, 47),
            def,
        );
    }
    // Every Window of small Images, where single rows, single columns and Levels of one point
    // meet keeping, the minimum and the mask.
    let (mut reads, mut diffs) = (0usize, 0usize);
    let sizes = [1, 2, 3, 5, 9];
    for &hh in &sizes {
        for &ww in &sizes {
            let mut rng = Rng((hh * 13 + ww) as u64);
            let mask: Vec<u8> = (0..hh * ww).map(|_| u8::from(rng.below(5) >= 2)).collect();
            let cases = [
                (Func::Labels, "U16", bilinear),
                (Func::Range, "F32", bilinear),
                (Func::Mask, "U16", bilinear),
                (Func::Raw, "U16", decimate),
                (Func::Polar, "ComplexF32", decimate),
            ];
            for (func, kind, options) in cases {
                let mut img = make(kind, hh, ww, (hh * 31 + ww) as u64);
                img.valid = Some(mask.clone());
                for isa in isas {
                    for top in 0..=6 {
                        let whole = rolling_with(&img, func, *isa, full(&img), top, options);
                        if func == Func::Labels || options.level_method == LevelMethod::Decimate {
                            diffs += kept_diffs(&whole);
                        }
                        for r0 in 0..hh {
                            for c0 in 0..ww {
                                for rc in 1..=hh - r0 {
                                    for cc in 1..=ww - c0 {
                                        let win = Window {
                                            row_start: r0,
                                            col_start: c0,
                                            row_count: rc,
                                            col_count: cc,
                                        };
                                        for (lw, lwhole) in
                                            rolling_with(&img, func, *isa, win, top, options)
                                                .iter()
                                                .zip(&whole)
                                        {
                                            for (s, w) in lw.iter().zip(lwhole) {
                                                diffs += bit_diffs(s, w);
                                            }
                                        }
                                        reads += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let small_ok = diffs == 0;
    ok &= small_ok;
    println!("\nSmall Images of 1, 2, 3, 5 and 9 rows by 1, 2, 3, 5 and 9 columns, with masks missing two pixels in five, every Window, Levels 0 to 0 up to 0 to 6: labels, range, the mask Slices, and `decimate` for `pixel.raw` and `complex.polar`. {reads} reads, {diffs} values differing from the whole Image or from the definition. Passed: **{small_ok}**\n");
    println!("All checks of the other kinds passed: **{ok}**\n");
    ok
}

fn time<F: FnMut()>(mut f: F) -> f64 {
    f();
    let mut best = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed().as_secs_f64());
    }
    best
}

fn benchmarks(isas: &[Isa]) {
    println!("## 6. Speed of the whole pass\n");
    println!("Best of five runs, one thread, the whole Image as the Window. \"Copy\" is a copy of one float32 plane of the Image's size into memory already in use, the reference section 12 names. The rolling buffer writes its Slices into buffers allocated once and reused. Its working rows, a few rows a Level, are planned once in a `Pass` and reused too, so a repeated read allocates nothing. Experiment 1's whole planes are shown beside it.\n");
    println!("\"Level 0 arithmetic\" is the Level 0 kernel alone over the whole Image, on buffers already in memory: the least the pass could cost for that function. For `pixel.raw` it is the copy.\n");
    println!("| Function | Sample type | Image | Instruction set | Levels | Slices written | Rolling buffer | Times a copy | Times the Level 0 arithmetic | Whole planes | Times a copy |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for (func, kind, n) in [
        (Func::Raw, "U16", 8192),
        (Func::Polar, "ComplexI16", 4096),
        (Func::Hsv, "U8x3", 4096),
    ] {
        let img = make(kind, n, n, 11);
        let src = vec![1f32; n * n];
        let mut dst = vec![0f32; n * n];
        let copy = time(|| {
            dst.copy_from_slice(&src);
            std::hint::black_box(&mut dst);
        });
        for isa in isas {
            let arithmetic = level0_arithmetic(func, *isa, n * n).unwrap_or(copy);
            for top in [0u32, 4] {
                let mut pass = Pass::new(&img, func, full(&img), top);
                let mut out = pass.outputs();
                let t = time(|| {
                    pass.run(&img, *isa, ReadOptions::default(), &mut out);
                    std::hint::black_box(&mut out);
                });
                let planes = time(|| {
                    std::hint::black_box(read_levels(&img, func, *isa, full(&img), top));
                });
                println!(
                    "| `{}` | {kind} | {n} × {n} | {isa:?} | 0 to {top} | {} | {t:.3} s | {:.1} | {:.1} | {planes:.3} s | {:.1} |",
                    func.name(),
                    func.slice_names().len(),
                    t / copy,
                    t / arithmetic,
                    planes / copy
                );
            }
        }
        println!("| Copy | float32 | {n} × {n} | | | 1 | {copy:.3} s | 1.0 | | | |");
    }
    println!();
    missing_speed(isas);
    other_speed(isas);
    binomial_speed(isas);
    bands_speed(isas);
    coarse_speed(isas);
}

/// A read of Levels `first` to `top` alone, into freshly planned buffers, with the pixels read.
fn coarse(
    img: &Image,
    func: Func,
    isa: Isa,
    window: Window,
    first: u32,
    options: ReadOptions,
) -> (Vec<Vec<Slice>>, usize) {
    let mut pass = Pass::for_levels(img, func, window, first, TOP, options.level_method);
    let mut out = pass.outputs();
    for s in out.iter_mut().flatten() {
        s.data.fill(f32::NAN);
    }
    pass.run(img, isa, options, &mut out);
    (out, pass.pixels_read())
}

/// Coarser Levels alone (section 5): a request for Levels `first` to 4 makes the same values as
/// a request for Levels 0 to 4, at those Levels, and none below. Where values are kept, for
/// `decimate` and labels, it reads only the grid points of Level `first`.
fn coarse_alone(isas: &[Isa], cases: &[(Func, &str)], rows: usize, cols: usize) -> bool {
    println!("## 9. Coarser Levels alone\n");
    println!("Requests for Levels 1 to 4, 2 to 4, 3 to 4 and 4 alone, against the same request for Levels 0 to 4, over 100 random Windows for each case. Where values are kept, each is also checked against the Level 0 value at its grid point, and the pixels read against the grid points the Window holds. \"Pixels read\" is the share of the Window's pixels whose samples were converted: every one, and the margins, where values are averaged; only the grid points of the first Level asked for, where they are kept.\n");
    println!("| Function | Sample type | Mask | Level method | Instruction set | Reads | Values compared | Differing bits | Pixels read, first Level 1 | Pixels read, Level 4 alone |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    let mut ok = true;
    let mut all: Vec<(Func, &str)> = cases.to_vec();
    all.extend([
        (Func::Labels, "U16"),
        (Func::Range, "F32"),
        (Func::Mask, "U16"),
    ]);
    for (case_no, (func, kind)) in all.iter().enumerate() {
        let plain = make(kind, rows, cols, 7 + case_no as u64);
        let holes = with_holes(
            make(kind, rows, cols, 7 + case_no as u64),
            300 + case_no as u64,
        );
        for (img, mask) in [(&plain, "none"), (&holes, "holes")] {
            for method in [
                LevelMethod::Decimate,
                LevelMethod::Bilinear,
                LevelMethod::Binomial5,
            ] {
                let options = ReadOptions {
                    level_method: method,
                    ..ReadOptions::default()
                };
                let kept = method == LevelMethod::Decimate || *func == Func::Labels;
                for isa in isas {
                    let mut rng = Rng(400 + case_no as u64);
                    let (mut reads, mut compared, mut diffs) = (0usize, 0usize, 0usize);
                    let (mut window_pixels, mut read1, mut read4) = (0usize, 0usize, 0usize);
                    for t in 0..100 {
                        let win = random_window(&mut rng, rows, cols, t);
                        let all_levels = rolling_with(img, *func, *isa, win, TOP, options);
                        window_pixels += win.row_count * win.col_count;
                        for first in 1..=TOP {
                            let (got, read) = coarse(img, *func, *isa, win, first, options);
                            reads += 1;
                            for (k, (lg, la)) in got.iter().zip(&all_levels).enumerate() {
                                if (k as u32) < first {
                                    diffs += lg.len();
                                    continue;
                                }
                                compared += la.iter().map(|s| s.data.len()).sum::<usize>();
                                diffs += differ(std::slice::from_ref(lg), std::slice::from_ref(la));
                            }
                            // A kept read touches exactly the grid points of its first Level
                            // that the Window holds, and each kept value is the Level 0 value at
                            // its grid point, as the plain conversion of the full read gives it.
                            if kept {
                                for l in got.iter().skip(first as usize) {
                                    for (s, s0) in l.iter().zip(&all_levels[0]) {
                                        let step = 1i64 << s.level;
                                        for i in s.rows.0..=s.rows.1 {
                                            for j in s.cols.0..=s.cols.1 {
                                                diffs += usize::from(
                                                    s.at(i, j).to_bits()
                                                        != s0.at(i * step, j * step).to_bits(),
                                                );
                                            }
                                        }
                                    }
                                }
                                let step = 1usize << first;
                                let grid = |start: usize, count: usize| {
                                    (start + count - 1) / step + 1 - start.div_ceil(step)
                                };
                                let points = grid(win.row_start, win.row_count)
                                    * grid(win.col_start, win.col_count);
                                diffs += usize::from(read != points);
                            }
                            match first {
                                1 => read1 += read,
                                TOP => read4 += read,
                                _ => {}
                            }
                        }
                    }
                    ok &= diffs == 0;
                    println!(
                        "| `{}` | {kind} | {mask} | `{}` | {isa:?} | {reads} | {compared} | {diffs} | {:.3} | {:.4} |",
                        func.name(),
                        method_name(method),
                        read1 as f64 / window_pixels as f64,
                        read4 as f64 / window_pixels as f64
                    );
                }
            }
        }
    }
    println!("\nAll checks of coarser Levels alone passed: **{ok}**\n");
    ok
}

/// What reading only the grid points saves: Level 4 alone with `decimate`, against Levels 0 to 4.
fn coarse_speed(isas: &[Isa]) {
    println!("### Coarser Levels alone\n");
    println!("8,192 × 8,192 pixels, the whole Image, one thread, best of five, on the fastest instruction set.\n");
    println!("| Slice | Level method | Levels | Pixels read | Time | Times a copy |");
    println!("| --- | --- | --- | --- | --- | --- |");
    let isa = *isas.last().unwrap();
    let n = 8192;
    let img = make("U16", n, n, 65);
    let src = vec![1f32; n * n];
    let mut dst = vec![0f32; n * n];
    let copy = time(|| {
        dst.copy_from_slice(&src);
        std::hint::black_box(&mut dst);
    });
    for (method, first) in [
        (LevelMethod::Decimate, 0),
        (LevelMethod::Decimate, 4),
        (LevelMethod::Bilinear, 0),
        (LevelMethod::Bilinear, 4),
    ] {
        let options = ReadOptions {
            level_method: method,
            ..ReadOptions::default()
        };
        let mut pass = Pass::for_levels(&img, Func::Raw, full(&img), first, TOP, method);
        let mut out = pass.outputs();
        let t = time(|| {
            pass.run(&img, isa, options, &mut out);
            std::hint::black_box(&mut out);
        });
        println!(
            "| `pixel.raw.intensity` | `{}` | {first} to {TOP} | {:.4} | {t:.4} s | {:.3} |",
            method_name(method),
            pass.pixels_read() as f64 / (n * n) as f64,
            t / copy
        );
    }
    println!("| Copy | | | | {copy:.4} s | 1.000 |");
    println!();
}

/// A read split into bands of rows, one thread each, into freshly planned buffers.
fn banded(
    img: &Image,
    func: Func,
    isa: Isa,
    window: Window,
    top: u32,
    options: ReadOptions,
    threads: usize,
) -> Vec<Vec<Slice>> {
    banded_counted(img, func, isa, window, top, options, threads).0
}

/// `banded`, with the number of bands the read used. The Slices are filled with NaN first, so a
/// row that no band writes cannot pass for one that holds a value.
fn banded_counted(
    img: &Image,
    func: Func,
    isa: Isa,
    window: Window,
    top: u32,
    options: ReadOptions,
    threads: usize,
) -> (Vec<Vec<Slice>>, usize) {
    let mut pass = BandedPass::new(img, func, window, top, options.level_method, threads);
    let mut out = pass.outputs();
    for s in out.iter_mut().flatten() {
        s.data.fill(f32::NAN);
    }
    pass.run(img, isa, options, &mut out);
    (out, pass.band_count())
}

/// Threads by bands of rows (section 12, rule 7): every value the same, bit for bit, as one
/// thread's, for every function, kind, level method and mask, over random Windows and every
/// Window of small Images.
fn bands(isas: &[Isa], cases: &[(Func, &str)], rows: usize, cols: usize) -> bool {
    println!("## 8. Threads by bands of rows\n");
    println!("Each read is split into bands of whole rows, every band but the first starting on a multiple of 2^4 rows, and each band runs on its own thread into its rows of the same Slices. 100 random Windows for each case, read with 2, 3, 4 and 7 threads into Slices filled with NaN first, compared with one thread.\n");
    println!("| Function | Sample type | Mask | Level method | Instruction set | Reads | Bands used | Values compared | Differing bits |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    let mut ok = true;
    let methods = [
        LevelMethod::Bilinear,
        LevelMethod::Binomial5,
        LevelMethod::Decimate,
    ];
    let mut all: Vec<(Func, &str)> = cases.to_vec();
    all.extend([
        (Func::Labels, "U16"),
        (Func::Range, "F32"),
        (Func::Mask, "U16"),
    ]);
    for (case_no, (func, kind)) in all.iter().enumerate() {
        let plain = make(kind, rows, cols, 7 + case_no as u64);
        let holes = with_holes(
            make(kind, rows, cols, 7 + case_no as u64),
            100 + case_no as u64,
        );
        for (img, mask) in [(&plain, "none"), (&holes, "holes")] {
            for method in methods {
                let options = ReadOptions {
                    level_method: method,
                    missing_weights: if case_no % 2 == 0 {
                        MissingWeights::Equal
                    } else {
                        MissingWeights::PixelCount
                    },
                    missing_value: (case_no % 3 == 0 && img.valid.is_some()).then_some(-1.0),
                };
                for isa in isas {
                    let mut rng = Rng(200 + case_no as u64);
                    let (mut reads, mut compared, mut diffs) = (0usize, 0usize, 0usize);
                    let (mut fewest, mut most) = (usize::MAX, 0);
                    for t in 0..100 {
                        let win = random_window(&mut rng, rows, cols, t);
                        let one = rolling_with(img, *func, *isa, win, TOP, options);
                        compared += one.iter().flatten().map(|s| s.data.len()).sum::<usize>();
                        for threads in [2, 3, 4, 7] {
                            let (got, used) =
                                banded_counted(img, *func, *isa, win, TOP, options, threads);
                            diffs += differ(&got, &one);
                            (fewest, most) = (fewest.min(used), most.max(used));
                            reads += 1;
                        }
                    }
                    ok &= diffs == 0;
                    println!(
                        "| `{}` | {kind} | {mask} | `{}` | {isa:?} | {reads} | {fewest} to {most} | {compared} | {diffs} |",
                        func.name(),
                        method_name(method)
                    );
                }
            }
        }
    }
    // Every Window of small Images, where bands of one row and Levels of one point meet.
    let (mut reads, mut diffs) = (0usize, 0usize);
    let sizes = [1, 2, 3, 5, 9, 17];
    for &hh in &sizes {
        for &ww in &[1, 3, 9] {
            let mut rng = Rng((hh * 19 + ww) as u64);
            let mask: Vec<u8> = (0..hh * ww).map(|_| u8::from(rng.below(5) >= 2)).collect();
            for (func, kind, method) in [
                (Func::Raw, "U16", LevelMethod::Bilinear),
                (Func::Polar, "ComplexF32", LevelMethod::Binomial5),
                (Func::Range, "F32", LevelMethod::Bilinear),
            ] {
                let mut img = make(kind, hh, ww, (hh * 41 + ww) as u64);
                img.valid = Some(mask.clone());
                let options = ReadOptions {
                    level_method: method,
                    ..ReadOptions::default()
                };
                for top in [0, 1, 2, 4] {
                    for r0 in 0..hh {
                        for rc in 1..=hh - r0 {
                            for c0 in 0..ww {
                                let win = Window {
                                    row_start: r0,
                                    col_start: c0,
                                    row_count: rc,
                                    col_count: ww - c0,
                                };
                                let isa = Isa::best();
                                let one = rolling_with(&img, func, isa, win, top, options);
                                for threads in [2, 5] {
                                    diffs += differ(
                                        &banded(&img, func, isa, win, top, options, threads),
                                        &one,
                                    );
                                    reads += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let small_ok = diffs == 0;
    ok &= small_ok;
    println!("\nSmall Images of 1 to 17 rows by 1, 3 and 9 columns, with masks, every Window by its rows and first column, Levels 0 to 0 up to 0 to 4, with 2 and 5 threads: {reads} reads, {diffs} values differing from one thread. Passed: **{small_ok}**\n");
    // One banded read planned once and run again and again, its pool's threads kept between
    // runs, into the same Slices filled with NaN each time; and coarse Levels alone through bands.
    let mut again = 0usize;
    let img = with_holes(make("U16x3", rows, cols, 501), 502);
    let win = Window {
        row_start: 13,
        col_start: 7,
        row_count: 1000,
        col_count: 1200,
    };
    let one = rolling_with(
        &img,
        Func::Hsv,
        Isa::best(),
        win,
        TOP,
        ReadOptions::default(),
    );
    let mut pass = BandedPass::new(&img, Func::Hsv, win, TOP, LevelMethod::Bilinear, 4);
    let mut out = pass.outputs();
    for _ in 0..50 {
        for s in out.iter_mut().flatten() {
            s.data.fill(f32::NAN);
        }
        pass.run(&img, Isa::best(), ReadOptions::default(), &mut out);
        again += differ(&out, &one);
    }
    let mut coarse_diffs = 0usize;
    let labels = with_holes(make("U16", rows, cols, 503), 504);
    for (func, image, method) in [
        (Func::Raw, &labels, LevelMethod::Decimate),
        (Func::Labels, &labels, LevelMethod::Bilinear),
        (Func::Hsv, &img, LevelMethod::Binomial5),
    ] {
        let options = ReadOptions {
            level_method: method,
            ..ReadOptions::default()
        };
        for first in 1..=TOP {
            let mut single = Pass::for_levels(image, func, win, first, TOP, method);
            let mut a = single.outputs();
            single.run(image, Isa::best(), options, &mut a);
            let mut banded = BandedPass::for_levels(image, func, win, first, TOP, method, 3);
            let mut b = banded.outputs();
            for s in b.iter_mut().flatten() {
                s.data.fill(f32::NAN);
            }
            banded.run(image, Isa::best(), options, &mut b);
            coarse_diffs += differ(&a, &b);
            if method == LevelMethod::Decimate || func == Func::Labels {
                coarse_diffs += usize::from(single.pixels_read() != banded.pixels_read());
            }
        }
    }
    let pool_ok = again == 0 && coarse_diffs == 0;
    ok &= pool_ok;
    println!("One banded read of `color.hsv`, planned once with 4 threads and run 50 times on the threads its pool keeps, into the same Slices filled with NaN each time: {again} values differing from one thread. Coarse Levels alone through bands, Levels 1 to 4 up to 4 alone, for `decimate`, labels and `binomial5`: {coarse_diffs} values or counts of pixels read differing from one `Pass`. Passed: **{pool_ok}**\n");
    // Nothing allocated by a read, on one thread or on bands, once the read is planned and has
    // run once.
    let mut allocated = 0;
    let isa = Isa::best();
    let methods = [
        LevelMethod::Bilinear,
        LevelMethod::Binomial5,
        LevelMethod::Decimate,
    ];
    for (func, image, method) in methods
        .iter()
        .flat_map(|&m| [(Func::Hsv, &img, m), (Func::Raw, &labels, m)])
    {
        let options = ReadOptions {
            level_method: method,
            missing_value: Some(0.0),
            ..ReadOptions::default()
        };
        let mut single = Pass::for_method(image, func, win, TOP, method);
        let mut a = single.outputs();
        single.run(image, isa, options, &mut a);
        let mut banded = BandedPass::new(image, func, win, TOP, method, 3);
        let mut b = banded.outputs();
        banded.run(image, isa, options, &mut b);
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        for _ in 0..10 {
            single.run(image, isa, options, &mut a);
            banded.run(image, isa, options, &mut b);
        }
        allocated += ALLOCATIONS.load(Ordering::Relaxed) - before;
    }
    let alloc_ok = allocated == 0;
    ok &= alloc_ok;
    println!("Allocations counted by the program's allocator over 10 more reads each of `color.hsv` and `pixel.raw` with missing data and a `missing_value`, under `bilinear`, `binomial5` and `decimate`, on one thread and on three bands, after a first read of each: {allocated}. Passed: **{alloc_ok}**\n");
    // The pool itself: rounds of every size, each index run once; a panic in a part, raised by
    // the round once every part has ended, with the pool usable after it.
    let mut pool = bil_exp1::pool::Pool::new(3);
    let mut misses = 0usize;
    for round in 0..300 {
        let count = round % (pool.width() + 1);
        let hits: [AtomicUsize; 4] = std::array::from_fn(|_| AtomicUsize::new(0));
        pool.run(count, &|i| {
            hits[i].fetch_add(1, Ordering::Relaxed);
        });
        for (i, h) in hits.iter().enumerate() {
            misses += usize::from(h.load(Ordering::Relaxed) != usize::from(i < count));
        }
    }
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut raised = 0;
    for bad in [0, 2] {
        let ended = AtomicUsize::new(0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool.run(4, &|i| {
                if i == bad {
                    panic!("part {i}");
                }
                ended.fetch_add(1, Ordering::Relaxed);
            })
        }));
        let message = result
            .err()
            .and_then(|e| e.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        raised +=
            usize::from(message == format!("part {bad}") && ended.load(Ordering::Relaxed) == 3);
        let after = AtomicUsize::new(0);
        pool.run(4, &|_| {
            after.fetch_add(1, Ordering::Relaxed);
        });
        misses += usize::from(after.load(Ordering::Relaxed) != 4);
    }
    std::panic::set_hook(hook);
    drop(pool);
    let rounds_ok = misses == 0 && raised == 2;
    ok &= rounds_ok;
    println!("The pool: 300 rounds of 0 to 4 parts, every part run exactly once: {misses} misses. A panic in the caller's part and in a pool thread's part, each raised by the round with its own message after the other parts ended, and the pool used again after each: {raised} of 2. Passed: **{rounds_ok}**\n");
    println!("All checks of threads by bands passed: **{ok}**\n");
    ok
}

/// What bands of rows gain: the whole Image read by 1 to the available number of threads.
fn bands_speed(isas: &[Isa]) {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("### Threads by bands of rows\n");
    println!("The whole Image, Levels 0 to 4, best of five, on the fastest instruction set. This machine offers {cores} threads. The bands run on threads kept between reads.\n");
    println!("| Function | Sample type | Image | Threads | Time | Speed-up | Times a copy |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    let isa = *isas.last().unwrap();
    for (func, kind, n) in [
        (Func::Raw, "U16", 8192),
        (Func::Polar, "ComplexI16", 4096),
        (Func::Hsv, "U8x3", 4096),
    ] {
        let img = make(kind, n, n, 11);
        let src = vec![1f32; n * n];
        let mut dst = vec![0f32; n * n];
        let copy = time(|| {
            dst.copy_from_slice(&src);
            std::hint::black_box(&mut dst);
        });
        let mut one = 0.0;
        for threads in 1..=cores.max(2) {
            let mut pass =
                BandedPass::new(&img, func, full(&img), TOP, LevelMethod::Bilinear, threads);
            let mut out = pass.outputs();
            let t = time(|| {
                pass.run(&img, isa, ReadOptions::default(), &mut out);
                std::hint::black_box(&mut out);
            });
            if threads == 1 {
                one = t;
            }
            println!(
                "| `{}` | {kind} | {n} × {n} | {} | {t:.3} s | {:.2} | {:.1} |",
                func.name(),
                pass.band_count(),
                one / t,
                t / copy
            );
        }
    }
    println!();
    // Small reads, where waking the pool's threads is a larger share of the work.
    println!("Smaller Windows of the 8,192 × 8,192 `pixel.raw` Image, Levels 0 to 4, the mean of 200 reads, best of five. The pool's threads are kept between reads, so a read only wakes them. `band_threads` gives one thread for each {MIN_BAND_PIXELS} pixels of the Window, at most.\n");
    println!("| Window | Threads | Time a read | Speed-up | Threads `band_threads` chooses |");
    println!("| --- | --- | --- | --- | --- |");
    let img = make("U16", 8192, 8192, 11);
    for size in [64usize, 256, 1024] {
        let win = Window {
            row_start: 1000,
            col_start: 2000,
            row_count: size,
            col_count: size,
        };
        let mut one = 0.0;
        for threads in 1..=cores.max(2) {
            let mut pass =
                BandedPass::new(&img, Func::Raw, win, TOP, LevelMethod::Bilinear, threads);
            let mut out = pass.outputs();
            let t = time(|| {
                for _ in 0..200 {
                    pass.run(&img, isa, ReadOptions::default(), &mut out);
                    std::hint::black_box(&mut out);
                }
            }) / 200.0;
            if threads == 1 {
                one = t;
            }
            println!(
                "| {size} × {size} | {} | {:.1} µs | {:.2} | {} |",
                pass.band_count(),
                t * 1e6,
                one / t,
                band_threads(win, threads)
            );
        }
    }
    println!();
}

/// The level method's name in the specification.
fn method_name(method: LevelMethod) -> &'static str {
    match method {
        LevelMethod::Bilinear => "bilinear",
        LevelMethod::Binomial5 => "binomial5",
        LevelMethod::Decimate => "decimate",
    }
}

/// Values whose bits differ between a result and the whole-Image reference at the same grid
/// points.
fn vs_reference(got: &[Vec<Slice>], reference: &[Vec<Slice>]) -> usize {
    got.iter()
        .zip(reference)
        .map(|(l, r)| l.iter().zip(r).map(|(s, w)| bit_diffs(s, w)).sum::<usize>())
        .sum()
}

/// `binomial5`, the second level method of section 5: weights 1 4 6 4 1 at each halving, with a
/// radius of 2, so each Level keeps five rows and the margins are wider. Checked bitwise against
/// the whole-plane reference, with and without missing data, between Windows and the whole
/// Image, between instruction sets, on every Window of small Images, and its arithmetic against
/// float64.
fn binomial(isas: &[Isa], cases: &[(Func, &str)], rows: usize, cols: usize) -> bool {
    println!("## 7. `binomial5`\n");
    println!("The same Images and random Windows as section 1, read with `binomial5`. The reference makes whole planes, one Level after another, in scalar code, with the arithmetic of the kernels but none of the rolling buffer's structure.\n");
    println!("| Function | Sample type | Mask | `missing_weights` | Instruction set | Values compared | Whole vs reference | Window vs whole | Against scalar |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    let mut ok = true;
    for (case_no, (func, kind)) in cases.iter().enumerate() {
        let plain = make(kind, rows, cols, 7 + case_no as u64);
        let holes = with_holes(
            make(kind, rows, cols, 7 + case_no as u64),
            70 + case_no as u64,
        );
        for (img, mask) in [(&plain, "none"), (&holes, "holes")] {
            let weightings: &[MissingWeights] = if img.valid.is_some() {
                &[MissingWeights::Equal, MissingWeights::PixelCount]
            } else {
                &[MissingWeights::Equal]
            };
            for &weights in weightings {
                let options = ReadOptions {
                    level_method: LevelMethod::Binomial5,
                    missing_weights: weights,
                    ..ReadOptions::default()
                };
                let expected = reference(img, *func, TOP, weights, LevelMethod::Binomial5);
                for isa in isas {
                    let whole = rolling_with(img, *func, *isa, full(img), TOP, options);
                    let against = vs_reference(&whole, &expected);
                    let (compared, diffs, vs_scalar) =
                        independence(img, *func, *isa, options, 80 + case_no as u64);
                    ok &= against == 0 && diffs == 0 && vs_scalar == 0;
                    let weights = if img.valid.is_some() {
                        format!("`{weights:?}`")
                    } else {
                        String::new()
                    };
                    println!(
                        "| `{}` | {kind} | {mask} | {weights} | {isa:?} | {compared} | {against} | {diffs} | {vs_scalar} |",
                        func.name()
                    );
                }
            }
        }
    }

    // The arithmetic against float64: Level 1 and Level 2 of `pixel.raw` on float data, each the
    // 1 4 6 4 1 filter applied along rows and down columns of the Level before, mirrored.
    let img = make("F32", rows, cols, 90);
    let got = rolling_with(
        &img,
        Func::Raw,
        Isa::best(),
        full(&img),
        2,
        ReadOptions {
            level_method: LevelMethod::Binomial5,
            ..ReadOptions::default()
        },
    );
    let bil_exp1::Samples::F32(v) = &img.samples else {
        unreachable!()
    };
    let mirror = |i: i64, n: usize| {
        let n = n as i64;
        if n == 1 {
            return 0;
        }
        let p = 2 * (n - 1);
        let m = i.rem_euclid(p);
        (if m >= n { p - m } else { m }) as usize
    };
    let wts = [1.0, 4.0, 6.0, 4.0, 1.0];
    let (mut plane, mut h, mut w): (Vec<f64>, usize, usize) =
        (v.iter().map(|x| *x as f64).collect(), rows, cols);
    let mut worst = 0f64;
    for level in got.iter().skip(1) {
        let (h2, w2) = ((h - 1) / 2 + 1, (w - 1) / 2 + 1);
        let mut along = vec![0f64; h * w2];
        for r in 0..h {
            for j in 0..w2 {
                along[r * w2 + j] = (0..5)
                    .map(|t| wts[t] * plane[r * w + mirror(2 * j as i64 + t as i64 - 2, w)])
                    .sum::<f64>()
                    / 16.0;
            }
        }
        let mut next = vec![0f64; h2 * w2];
        for i in 0..h2 {
            for j in 0..w2 {
                next[i * w2 + j] = (0..5)
                    .map(|t| wts[t] * along[mirror(2 * i as i64 + t as i64 - 2, h) * w2 + j])
                    .sum::<f64>()
                    / 16.0;
            }
        }
        for (x, y) in level[0].data.iter().zip(&next) {
            worst = worst.max(((*x as f64 - y) / y.abs().max(1.0)).abs());
        }
        (plane, h, w) = (next, h2, w2);
    }
    let arithmetic_ok = worst < 1e-6;
    ok &= arithmetic_ok;
    println!("\nThe arithmetic: Levels 1 and 2 of `pixel.raw` on float data against the filter 1 4 6 4 1 by 1 4 6 4 1, divided by 256, computed in float64 from the Level before. Largest relative difference {worst:.1e}. Passed: **{arithmetic_ok}**\n");

    // Aliasing, as section 5 states it: at the highest frequency the halved grid can hold, a
    // period of four pixels, `bilinear` passes half the amplitude and `binomial5` a quarter.
    let (n, m) = (64, 256);
    let wave = Image {
        rows: n,
        cols: m,
        comps: 1,
        samples: bil_exp1::Samples::F32(
            (0..n * m)
                .map(|i| (std::f64::consts::FRAC_PI_2 * (i % m) as f64).cos() as f32)
                .collect(),
        ),
        valid: None,
    };
    let mut passed = [0f32; 2];
    for (slot, method) in [LevelMethod::Bilinear, LevelMethod::Binomial5]
        .into_iter()
        .enumerate()
    {
        let options = ReadOptions {
            level_method: method,
            ..ReadOptions::default()
        };
        let level1 = &rolling_with(&wave, Func::Raw, Isa::best(), full(&wave), 1, options)[1][0];
        // Away from the edges, Level 1 alternates between plus and minus the amplitude passed.
        passed[slot] = level1.at(16, 20).abs();
    }
    let alias_ok = (passed[0] - 0.5).abs() < 1e-6 && (passed[1] - 0.25).abs() < 1e-6;
    ok &= alias_ok;
    println!("Aliasing: a wave with a period of four pixels, the highest frequency Level 1 can hold, comes through `bilinear` at {} of its amplitude and through `binomial5` at {}, as section 5 states. Passed: **{alias_ok}**\n", passed[0], passed[1]);

    // Kinds that are not averaged keep their own reductions, so `binomial5` gives labels and range
    // exactly what `bilinear` gives; the mask Slices are the mask averaged by `binomial5`.
    let binomial = ReadOptions {
        level_method: LevelMethod::Binomial5,
        ..ReadOptions::default()
    };
    let mut kinds = 0;
    let labels = with_holes(make("U16", rows, cols, 91), 92);
    let depth = with_holes(make("F32", rows, cols, 93), 94);
    for isa in isas {
        for (img, func) in [(&labels, Func::Labels), (&depth, Func::Range)] {
            kinds += differ(
                &rolling_with(img, func, *isa, full(img), TOP, binomial),
                &rolling_with(img, func, *isa, full(img), TOP, ReadOptions::default()),
            );
        }
        let mask_as_image = Image {
            rows,
            cols,
            comps: 1,
            samples: bil_exp1::Samples::F32(
                labels
                    .valid
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|&b| f32::from(b))
                    .collect(),
            ),
            valid: None,
        };
        let averaged = rolling_with(
            &mask_as_image,
            Func::Raw,
            *isa,
            full(&labels),
            TOP,
            binomial,
        );
        let got = rolling_with(&labels, Func::Mask, *isa, full(&labels), TOP, binomial);
        for (l, a) in got.iter().zip(&averaged) {
            for ((v, f), x) in l[0].data.iter().zip(&l[1].data).zip(&a[0].data) {
                kinds += usize::from(f.to_bits() != x.to_bits());
                kinds += usize::from(*v != if *x > 0.0 { 1.0 } else { 0.0 });
            }
        }
        let (_, d, s) = independence(&labels, Func::Mask, *isa, binomial, 95);
        kinds += d + s;
    }
    let kinds_ok = kinds == 0;
    ok &= kinds_ok;
    println!("Other kinds: labels and range read with `binomial5` give exactly what `bilinear` gives, since neither is averaged; `pixel.mask` gives the mask averaged by `binomial5`, the same between Windows and the whole Image and on every instruction set. Values differing: {kinds}. Passed: **{kinds_ok}**\n");

    // Every Window of small Images, where the five-row ring and mirroring by two meet single rows,
    // single columns and Levels of one point.
    let (mut reads, mut diffs) = (0usize, 0usize);
    let sizes = [1, 2, 3, 4, 5, 9];
    for &hh in &sizes {
        for &ww in &sizes {
            let mut rng = Rng((hh * 17 + ww) as u64);
            let mask: Vec<u8> = (0..hh * ww).map(|_| u8::from(rng.below(5) >= 2)).collect();
            for (func, kind, masked) in [
                (Func::Raw, "U16", false),
                (Func::Raw, "U16", true),
                (Func::Polar, "ComplexF32", true),
                (Func::Hsv, "U8x3", false),
            ] {
                let mut img = make(kind, hh, ww, (hh * 37 + ww) as u64);
                if masked {
                    img.valid = Some(mask.clone());
                }
                for isa in isas {
                    for top in 0..=6 {
                        let whole = rolling_with(&img, func, *isa, full(&img), top, binomial);
                        diffs += vs_reference(
                            &whole,
                            &reference(
                                &img,
                                func,
                                top,
                                MissingWeights::Equal,
                                LevelMethod::Binomial5,
                            ),
                        );
                        for r0 in 0..hh {
                            for c0 in 0..ww {
                                for rc in 1..=hh - r0 {
                                    for cc in 1..=ww - c0 {
                                        let win = Window {
                                            row_start: r0,
                                            col_start: c0,
                                            row_count: rc,
                                            col_count: cc,
                                        };
                                        let got =
                                            rolling_with(&img, func, *isa, win, top, binomial);
                                        diffs += vs_reference(&got, &whole);
                                        reads += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let small_ok = diffs == 0;
    ok &= small_ok;
    println!("Small Images of 1, 2, 3, 4, 5 and 9 rows by the same columns, every Window, Levels 0 to 0 up to 0 to 6, with and without masks missing two pixels in five: {reads} reads, {diffs} values differing from the whole Image or from the reference. Passed: **{small_ok}**\n");
    println!("All `binomial5` checks passed: **{ok}**\n");
    ok
}

/// What `binomial5` costs beside `bilinear`: about twice the arithmetic at each halving, and two
/// more rows a Level.
fn binomial_speed(isas: &[Isa]) {
    println!("### `binomial5` beside `bilinear`\n");
    println!("The whole Image, one thread, best of five, Levels 0 to 4. Level 0 is the same for both, so the difference is in the halvings.\n");
    println!(
        "| Function | Sample type | Image | Instruction set | `bilinear` | `binomial5` | Ratio |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for (func, kind, n) in [
        (Func::Raw, "U16", 8192),
        (Func::Polar, "ComplexI16", 4096),
        (Func::Hsv, "U8x3", 4096),
    ] {
        let img = make(kind, n, n, 11);
        for isa in isas.iter().filter(|i| **i != Isa::Scalar) {
            let mut t = [0f64; 2];
            for (slot, method) in [LevelMethod::Bilinear, LevelMethod::Binomial5]
                .into_iter()
                .enumerate()
            {
                let options = ReadOptions {
                    level_method: method,
                    ..ReadOptions::default()
                };
                let mut pass = Pass::for_method(&img, func, full(&img), TOP, method);
                let mut out = pass.outputs();
                t[slot] = time(|| {
                    pass.run(&img, *isa, options, &mut out);
                    std::hint::black_box(&mut out);
                });
            }
            println!(
                "| `{}` | {kind} | {n} × {n} | {isa:?} | {:.3} s | {:.3} s | {:.2} |",
                func.name(),
                t[0],
                t[1],
                t[1] / t[0]
            );
        }
    }
    println!();
}

/// The other kinds of Slice and `decimate`, against a copy.
fn other_speed(isas: &[Isa]) {
    println!("### Other kinds of Slice, and `decimate`\n");
    println!("8,192 × 8,192 pixels, the whole Image, one thread, best of five. Each makes one or two Slices; `pixel.mask` makes two.\n");
    println!("| Slice | Image | Level method | Instruction set | Levels | Time | Times a copy |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    let n = 8192;
    let src = vec![1f32; n * n];
    let mut dst = vec![0f32; n * n];
    let copy = time(|| {
        dst.copy_from_slice(&src);
        std::hint::black_box(&mut dst);
    });
    let decimate = ReadOptions {
        level_method: LevelMethod::Decimate,
        ..ReadOptions::default()
    };
    let cases = [
        (
            "class.labels.value",
            Func::Labels,
            make("U16", n, n, 61),
            "U16",
            ReadOptions::default(),
        ),
        (
            "depth.sensor.range",
            Func::Range,
            make("F32", n, n, 62),
            "F32",
            ReadOptions::default(),
        ),
        (
            "pixel.mask",
            Func::Mask,
            with_holes(make("U16", n, n, 63), 64),
            "U16, with holes",
            ReadOptions::default(),
        ),
        (
            "pixel.raw.intensity",
            Func::Raw,
            make("U16", n, n, 65),
            "U16",
            decimate,
        ),
    ];
    for (name, func, img, image, options) in &cases {
        for isa in isas.iter().filter(|i| **i != Isa::Scalar) {
            for top in [0u32, 4] {
                let mut pass = Pass::for_method(img, *func, full(img), top, options.level_method);
                let mut out = pass.outputs();
                let t = time(|| {
                    pass.run(img, *isa, *options, &mut out);
                    std::hint::black_box(&mut out);
                });
                let method = method_name(options.level_method);
                println!(
                    "| `{name}` | {image} | `{method}` | {isa:?} | 0 to {top} | {t:.3} s | {:.1} |",
                    t / copy
                );
            }
        }
    }
    println!("| Copy | float32 | | | | {copy:.3} s | 1.0 |");
    println!();
}

/// What the missing-data path costs: the same reads with a validity mask, against none.
fn missing_speed(isas: &[Isa]) {
    println!("### With missing data\n");
    println!("The same Images with the validity mask of section 4. Each read checks every value and skips the missing ones.\n");
    println!("| Function | Sample type | Image | Instruction set | Levels | No mask | With a mask | Ratio |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for (func, kind, n) in [
        (Func::Raw, "U16", 8192),
        (Func::Polar, "ComplexI16", 4096),
        (Func::Hsv, "U8x3", 4096),
    ] {
        let plain = make(kind, n, n, 11);
        let masked = with_holes(make(kind, n, n, 11), 12);
        for isa in isas.iter().filter(|i| **i != Isa::Scalar) {
            for top in [0u32, 4] {
                let mut t = [0f64; 2];
                for (slot, img) in [&plain, &masked].into_iter().enumerate() {
                    let mut pass = Pass::new(img, func, full(img), top);
                    let mut out = pass.outputs();
                    t[slot] = time(|| {
                        pass.run(img, *isa, ReadOptions::default(), &mut out);
                        std::hint::black_box(&mut out);
                    });
                }
                println!(
                    "| `{}` | {kind} | {n} × {n} | {isa:?} | 0 to {top} | {:.3} s | {:.3} s | {:.2} |",
                    func.name(),
                    t[0],
                    t[1],
                    t[1] / t[0]
                );
            }
        }
    }
    println!();
}

/// The Level 0 kernel of `func` over `n` values already in memory, or `None` for `pixel.raw`,
/// whose Level 0 work is a conversion and a copy.
fn level0_arithmetic(func: Func, isa: Isa, n: usize) -> Option<f64> {
    let mut rng = Rng(5);
    let mut input = || -> Vec<f32> { (0..n).map(|_| (rng.unit() * 255.0) as f32).collect() };
    let (a, b, c) = (input(), input(), input());
    let mut o: Vec<Vec<f32>> = (0..5).map(|_| vec![0f32; n]).collect();
    let mut ptrs = [std::ptr::null_mut(); 5];
    for (p, v) in ptrs.iter_mut().zip(o.iter_mut()) {
        *p = v.as_mut_ptr();
    }
    match func {
        Func::Raw | Func::Mask | Func::Labels | Func::Range => None,
        // Every buffer holds n values.
        Func::Polar => Some(time(|| unsafe {
            polar0(
                isa,
                n,
                a.as_ptr(),
                b.as_ptr(),
                [ptrs[0], ptrs[1], ptrs[2], ptrs[3]],
            )
        })),
        Func::Hsv => Some(time(|| unsafe {
            hsv0(isa, n, a.as_ptr(), b.as_ptr(), c.as_ptr(), ptrs)
        })),
    }
}
