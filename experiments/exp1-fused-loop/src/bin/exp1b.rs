// SPDX-License-Identifier: LGPL-2.1-or-later
//! Runs experiment 1b, the rolling buffer, and prints its report.
//!
//! `--check` runs the correctness tests: experiment 1's bitwise tests through the rolling buffer,
//! small Images, reused buffers and missing data. The full report adds speed, measured against a
//! copy into warm memory and against each function's own arithmetic, as section 12 asks.

use bil_exp1::simd::Isa;
use bil_exp1::testing::{bit_diffs, full, make, masked_reference, random_window, with_holes, Rng};
use bil_exp1::{
    hsv0, plan_outputs, polar0, read_levels, read_levels_into, Func, Image, LevelMethod,
    MissingWeights, Pass, ReadOptions, Slice, Window,
};
use std::time::Instant;

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
    println!("Instruction sets available: {isas:?}. Levels 0 to {TOP} in one pass, on a rolling buffer of three rows for each Level above Level 0. Single thread.\n");

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
                let mut pass = Pass::new(img, *func, full(img), top);
                let mut out = pass.outputs();
                let t = time(|| {
                    pass.run(img, *isa, *options, &mut out);
                    std::hint::black_box(&mut out);
                });
                let method = match options.level_method {
                    LevelMethod::Bilinear => "bilinear",
                    LevelMethod::Decimate => "decimate",
                };
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
