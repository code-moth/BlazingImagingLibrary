// SPDX-License-Identifier: LGPL-2.1-or-later
//! Runs experiment 1b, the rolling buffer, and prints its report.
//!
//! It passes when the bitwise tests of experiment 1 still pass, now through the rolling buffer,
//! and the whole pass is within a small factor of a copy into warm memory.

use bil_exp1::simd::Isa;
use bil_exp1::testing::{bit_diffs, full, make, random_window, Rng};
use bil_exp1::{
    hsv0, plan_outputs, polar0, read_levels, read_levels_into, Func, Image, Slice, Window,
};
use std::time::Instant;

const TOP: u32 = 4;

fn func_name(f: Func) -> &'static str {
    match f {
        Func::Raw => "pixel.raw",
        Func::Polar => "complex.polar",
        Func::Hsv => "color.hsv",
    }
}

/// Reads through the rolling buffer into freshly planned buffers.
fn rolling(img: &Image, func: Func, isa: Isa, window: Window, top: u32) -> Vec<Vec<Slice>> {
    let mut out = plan_outputs(img, func, window, top);
    read_levels_into(img, func, isa, window, top, &mut out);
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
                func_name(*func)
            );
        }
    }
    println!("\nAll bitwise checks passed: **{all_ok}**\n");

    all_ok &= small_images(&isas);
    all_ok &= reuse(&isas);

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
            read_levels_into(&img, func, *isa, win, TOP, &mut out);
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
            read_levels_into(&img, func, *isa, win, TOP, &mut out);
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
    println!("## 4. Speed of the whole pass\n");
    println!("Best of five runs, one thread, the whole Image as the Window. \"Copy\" is a copy of one float32 plane of the Image's size into memory already in use, the reference section 12 names. The rolling buffer writes its Slices into buffers allocated once and reused. Its own working rows, a few rows a Level, are still allocated on each call. Experiment 1's whole planes are shown beside it.\n");
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
                let mut out = plan_outputs(&img, func, full(&img), top);
                let t = time(|| {
                    read_levels_into(&img, func, *isa, full(&img), top, &mut out);
                    std::hint::black_box(&mut out);
                });
                let planes = time(|| {
                    std::hint::black_box(read_levels(&img, func, *isa, full(&img), top));
                });
                println!(
                    "| `{}` | {kind} | {n} × {n} | {isa:?} | 0 to {top} | {} | {t:.3} s | {:.1} | {:.1} | {planes:.3} s | {:.1} |",
                    func_name(func),
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
        Func::Raw => None,
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
