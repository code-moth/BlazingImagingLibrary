// SPDX-License-Identifier: LGPL-2.1-or-later
//! Runs experiment 1 and prints its report.

use bil_exp1::simd::Isa;
use bil_exp1::testing::{bit_diffs, full, make, random_window, Rng};
use bil_exp1::{halve, hsv0, polar0, read_levels, Func, Samples};
use std::time::Instant;

const TOP: u32 = 4;

fn main() {
    let isas = Isa::available();
    println!("# Experiment 1 report\n");
    println!(
        "Instruction sets available: {isas:?}. Levels 0 to {TOP} in one pass. Single thread.\n"
    );

    let cases = [
        (Func::Raw, "U16"),
        (Func::Raw, "F32"),
        (Func::Polar, "ComplexI16"),
        (Func::Polar, "ComplexF32"),
        (Func::Hsv, "U8x3"),
        (Func::Hsv, "U16x3"),
    ];
    let (rows, cols) = (1037, 1291); // neither a multiple of 16
    let windows_per_case = 400;

    println!("## 1. Bitwise agreement\n");
    println!("Image {rows} × {cols}. Each Window's Slices are compared with the same grid points cut from the whole Image's Slices, at every Level.\n");
    println!("| Function | Sample type | Instruction set | Windows | Values compared | Differing bits, Window vs whole | Differing bits, vs scalar |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    let mut all_ok = true;
    for (case_no, (func, kind)) in cases.iter().enumerate() {
        let img = make(kind, rows, cols, 7 + case_no as u64);
        let scalar_whole = read_levels(&img, *func, Isa::Scalar, full(&img), TOP);
        for isa in &isas {
            let whole = read_levels(&img, *func, *isa, full(&img), TOP);
            // Against scalar, whole Image, every Level and Slice.
            let mut vs_scalar = 0;
            for (lw, ls) in whole.iter().zip(&scalar_whole) {
                for (a, b) in lw.iter().zip(ls) {
                    vs_scalar += a
                        .data
                        .iter()
                        .zip(&b.data)
                        .filter(|(x, y)| x.to_bits() != y.to_bits())
                        .count();
                }
            }
            // Random Windows, including ones at the edges and smaller than a Level's spacing.
            let mut rng = Rng(1000 + case_no as u64);
            let mut compared = 0usize;
            let mut diffs = 0usize;
            for t in 0..windows_per_case {
                let win = random_window(&mut rng, rows, cols, t);
                let got = read_levels(&img, *func, *isa, win, TOP);
                for (lw, lwhole) in got.iter().zip(&whole) {
                    for (s, w) in lw.iter().zip(lwhole) {
                        assert_eq!(s.name, w.name);
                        compared += s.data.len();
                        diffs += bit_diffs(s, w);
                    }
                }
            }
            all_ok &= diffs == 0 && vs_scalar == 0;
            println!("| `{}` | {kind} | {isa:?} | {windows_per_case} | {compared} | {diffs} | {vs_scalar} |", func_name(*func));
        }
    }
    println!("\nAll bitwise checks passed: **{all_ok}**\n");

    accuracy();
    // `--check` runs the correctness sections only, for CI, and fails the run on any difference.
    if std::env::args().any(|a| a == "--check") {
        if !all_ok {
            std::process::exit(1);
        }
        return;
    }
    benchmarks(&isas);
}

fn func_name(f: Func) -> &'static str {
    match f {
        Func::Raw => "pixel.raw",
        Func::Polar => "complex.polar",
        Func::Hsv => "color.hsv",
    }
}

/// Level 0 values against float64 references.
fn accuracy() {
    println!("## 2. Accuracy at Level 0, against float64 references\n");
    let isa = Isa::best();
    let img = make("ComplexF32", 512, 512, 3);
    let out = read_levels(&img, Func::Polar, isa, full(&img), 0);
    let Samples::ComplexF32(v) = &img.samples else {
        unreachable!()
    };
    let (mut max_phase, mut max_mag_rel) = (0f64, 0f64);
    for i in 0..512 * 512 {
        let (re, im) = (v[2 * i] as f64, v[2 * i + 1] as f64);
        let ph = im.atan2(re);
        let mut d = (out[0][1].data[i] as f64 - ph).abs();
        if d > std::f64::consts::PI {
            d = std::f64::consts::TAU - d;
        }
        max_phase = max_phase.max(d);
        let m = (re * re + im * im).sqrt();
        if m > 0.0 {
            max_mag_rel = max_mag_rel.max((out[0][0].data[i] as f64 - m).abs() / m);
        }
    }
    let img = make("U8x3", 512, 512, 4);
    let out = read_levels(&img, Func::Hsv, isa, full(&img), 0);
    let Samples::U8(v) = &img.samples else {
        unreachable!()
    };
    let mut max_hue = 0f64;
    for i in 0..512 * 512 {
        let (r, g, b) = (v[3 * i] as f64, v[3 * i + 1] as f64, v[3 * i + 2] as f64);
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        let d = mx - mn;
        let h = if d == 0.0 {
            0.0
        } else if mx == r {
            ((g - b) / d).rem_euclid(6.0) / 6.0
        } else if mx == g {
            ((b - r) / d + 2.0) / 6.0
        } else {
            ((r - g) / d + 4.0) / 6.0
        };
        let mut e = (out[0][0].data[i] as f64 - h).abs();
        if e > 0.5 {
            e = 1.0 - e;
        }
        max_hue = max_hue.max(e);
    }
    println!("| Slice | Largest error | In units |");
    println!("| --- | --- | --- |");
    println!("| `complex.polar.phase` | {max_phase:.2e} | radians |");
    println!("| `complex.polar.magnitude` | {max_mag_rel:.2e} | relative |");
    println!("| `color.hsv.hue` | {max_hue:.2e} | turns |");
    println!();
}

fn time<F: FnMut()>(mut f: F) -> f64 {
    f();
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed().as_secs_f64());
    }
    best
}

fn benchmarks(isas: &[Isa]) {
    println!("## 3. Speed\n");
    println!("Best of three runs, one thread. \"Copy\" is a plain copy of the Level 0 output's bytes, float32, the reference section 12 names.\n");
    println!("| Function | Sample type | Image | Instruction set | Levels | Time | Megapixels a second | Times a copy |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    let cases = [
        (Func::Raw, "U16", 8192),
        (Func::Polar, "ComplexI16", 4096),
        (Func::Hsv, "U8x3", 4096),
    ];
    for (func, kind, n) in cases {
        let img = make(kind, n, n, 11);
        let src = vec![1f32; n * n];
        let mut dst = vec![0f32; n * n];
        let copy = time(|| dst.copy_from_slice(&src));
        for isa in isas {
            for top in [0u32, 4] {
                let t = time(|| {
                    let out = read_levels(&img, func, *isa, full(&img), top);
                    std::hint::black_box(out);
                });
                println!(
                    "| `{}` | {kind} | {n} × {n} | {isa:?} | 0 to {top} | {:.3} s | {:.0} | {:.1} |",
                    func_name(func),
                    t,
                    (n * n) as f64 / t / 1e6,
                    t / copy
                );
            }
        }
        println!(
            "| Copy | float32 | {n} × {n} | | | {copy:.3} s | {:.0} | 1.0 |",
            (n * n) as f64 / copy / 1e6
        );
        let fresh = time(|| {
            let mut d = vec![0f32; n * n];
            d.copy_from_slice(&src);
            std::hint::black_box(d);
        });
        println!(
            "| Copy into fresh memory | float32 | {n} × {n} | | | {fresh:.3} s | {:.0} | {:.1} |",
            (n * n) as f64 / fresh / 1e6,
            fresh / copy
        );
    }
    println!();
    kernel_speed(isas);
}

/// The kernels alone, over warm buffers, so the instruction set is all that varies.
fn kernel_speed(isas: &[Isa]) {
    println!("## 4. Kernel speed on warm buffers\n");
    println!("16 million values, already in memory, best of three. This isolates the arithmetic from allocation.\n");
    println!("| Kernel | Instruction set | Nanoseconds a value | Speed-up over scalar |");
    println!("| --- | --- | --- | --- |");
    let n = 16 * 1024 * 1024;
    let mut rng = Rng(5);
    let a: Vec<f32> = (0..n)
        .map(|_| (rng.unit() * 200.0 - 100.0) as f32)
        .collect();
    let b: Vec<f32> = (0..n)
        .map(|_| (rng.unit() * 200.0 - 100.0) as f32)
        .collect();
    let c: Vec<f32> = (0..n).map(|_| (rng.unit() * 200.0) as f32).collect();
    let mut o: Vec<Vec<f32>> = (0..5).map(|_| vec![0f32; n]).collect();
    let names = [
        "halve (one 1 2 1 step)",
        "complex.polar at Level 0",
        "color.hsv at Level 0",
    ];
    for (k, name) in names.iter().enumerate() {
        let mut scalar_t = 0.0;
        for isa in isas {
            let t = time(|| {
                let (o0, rest) = o.split_at_mut(1);
                let (o1, rest) = rest.split_at_mut(1);
                let (o2, rest) = rest.split_at_mut(1);
                let (o3, o4) = rest.split_at_mut(1);
                let outs4 = [
                    o0[0].as_mut_ptr(),
                    o1[0].as_mut_ptr(),
                    o2[0].as_mut_ptr(),
                    o3[0].as_mut_ptr(),
                ];
                // Every buffer holds n values.
                unsafe {
                    match k {
                        0 => halve(
                            *isa,
                            n,
                            a.as_ptr(),
                            b.as_ptr(),
                            c.as_ptr(),
                            o0[0].as_mut_ptr(),
                        ),
                        1 => polar0(*isa, n, a.as_ptr(), b.as_ptr(), outs4),
                        _ => hsv0(
                            *isa,
                            n,
                            c.as_ptr(),
                            a.as_ptr(),
                            b.as_ptr(),
                            [outs4[0], outs4[1], outs4[2], outs4[3], o4[0].as_mut_ptr()],
                        ),
                    }
                }
            });
            if *isa == Isa::Scalar {
                scalar_t = t;
            }
            println!(
                "| {name} | {isa:?} | {:.2} | {:.1} |",
                t / n as f64 * 1e9,
                scalar_t / t
            );
        }
    }
    println!();
}
