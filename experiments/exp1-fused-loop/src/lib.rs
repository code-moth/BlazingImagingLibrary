// SPDX-License-Identifier: LGPL-2.1-or-later
//! BIL experiment 1: one fused pass that makes a slice function's Slices at every Level from
//! 0 to n, with Window independence exact to the bit, on scalar, AVX2 and AVX-512 code.
//!
//! The rules follow section 5 of the specification:
//! - Level n holds the Image pixels whose row and column are multiples of 2^n.
//! - Each halving applies weights 1 2 1, divided by 4, along rows and then columns.
//! - Angles (phase, hue) are reduced as unit vectors.
//! - At an Image edge, values are mirrored about the first and last grid point of each Level.
//!
//! Not covered here, and named in the report: missing data, threads, stored levels and other
//! kinds of Slice. Rolling row buffers are experiment 1b, in `rolling`.

// The experiment keeps kernels short and close to the specification's formulas.
#![allow(
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::excessive_precision
)]

// The vector layer and kernels now live in the `bil` crate.
pub use bil::{kernels, simd};

/// Experiment 1b's pass, the rolling buffer and its threads, now live in the `bil` crate, as
/// `bil::pass`, which `read_slice` runs; this crate keeps the full-plane reference and the checks.
pub use bil::pass::{pool, rolling};

/// Test images and comparisons for the experiment programs.
pub mod testing;

pub use bil::pass::*;
use bil::simd::Isa;

/// A set of planes over one rectangle of a Level's grid.
struct Planes {
    rows: (i64, i64),
    cols: (i64, i64),
    planes: Vec<Vec<f32>>,
}

impl Planes {
    fn new(rows: (i64, i64), cols: (i64, i64), count: usize) -> Planes {
        let size = ((rows.1 - rows.0 + 1) * (cols.1 - cols.0 + 1)) as usize;
        Planes {
            rows,
            cols,
            planes: (0..count).map(|_| vec![0f32; size]).collect(),
        }
    }
    fn width(&self) -> usize {
        (self.cols.1 - self.cols.0 + 1) as usize
    }
    fn row(&self, p: usize, r: i64) -> &[f32] {
        let w = self.width();
        let o = (r - self.rows.0) as usize * w;
        &self.planes[p][o..o + w]
    }
    fn row_ptr(&mut self, p: usize, r: i64) -> *mut f32 {
        let w = self.width();
        let o = (r - self.rows.0) as usize * w;
        self.planes[p][o..o + w].as_mut_ptr()
    }
}

/// Makes every Slice of `func` for `window` at Levels 0 to `top`, in one pass over the native
/// samples the Window and its margins need.
pub fn read_levels(img: &Image, func: Func, isa: Isa, window: Window, top: u32) -> Vec<Vec<Slice>> {
    assert!(isa.is_available(), "{isa:?} is not available");
    assert!(
        matches!(func, Func::Raw | Func::Polar | Func::Hsv),
        "experiment 1's whole planes make only pixel.raw, complex.polar and color.hsv"
    );
    let (row_out, row_need) = axis_plan(
        img.rows,
        window.row_start,
        window.row_start + window.row_count - 1,
        top,
    );
    let (col_out, col_need) = axis_plan(
        img.cols,
        window.col_start,
        window.col_start + window.col_count - 1,
        top,
    );
    let (carried, extra) = func.level0_planes();

    // Level 0, straight from the samples.
    let (r0, r1) = row_need[0].unwrap();
    let (c0, c1) = col_need[0].unwrap();
    let mut level0 = Planes::new((r0, r1), (c0, c1), carried + extra);
    let mut bufs: Vec<Vec<f32>> = vec![Vec::new(); func.inputs()];
    let n = (c1 - c0 + 1) as usize;
    for r in r0..=r1 {
        convert_row(img, r as usize, c0 as usize, c1 as usize, &mut bufs);
        match func {
            Func::Raw => {
                let dst = level0.row_ptr(0, r);
                unsafe { std::ptr::copy_nonoverlapping(bufs[0].as_ptr(), dst, n) };
            }

            Func::Polar => {
                let out = [
                    level0.row_ptr(0, r),
                    level0.row_ptr(1, r),
                    level0.row_ptr(2, r),
                    level0.row_ptr(3, r),
                ];
                // Each pointer covers the n values of this row.
                unsafe { polar0(isa, n, bufs[0].as_ptr(), bufs[1].as_ptr(), out) };
            }
            Func::Hsv => {
                let out = [
                    level0.row_ptr(0, r),
                    level0.row_ptr(1, r),
                    level0.row_ptr(2, r),
                    level0.row_ptr(3, r),
                    level0.row_ptr(4, r),
                ];
                // Each pointer covers the n values of this row.
                unsafe {
                    hsv0(
                        isa,
                        n,
                        bufs[0].as_ptr(),
                        bufs[1].as_ptr(),
                        bufs[2].as_ptr(),
                        out,
                    )
                };
            }
            Func::Mask | Func::Labels | Func::Range => unreachable!("checked above"),
        }
    }

    let mut result = Vec::new();
    let mut prev = level0;
    for k in 0..=top {
        let next = if k < top {
            reduce(
                img,
                isa,
                carried,
                &prev,
                k + 1,
                row_need[k as usize + 1],
                col_need[k as usize + 1],
            )
        } else {
            None
        };
        result.push(slices_at(
            func,
            isa,
            k,
            prev,
            row_out[k as usize],
            col_out[k as usize],
        ));
        match next {
            Some(n) => prev = n,
            None => break,
        }
    }
    result
}

/// One halving: Level k from Level k - 1, over the grid indices Level k needs.
fn reduce(
    img: &Image,
    isa: Isa,
    carried: usize,
    prev: &Planes,
    k: u32,
    rn: Option<(i64, i64)>,
    cn: Option<(i64, i64)>,
) -> Option<Planes> {
    let (rn, cn) = (rn?, cn?);
    let jr = grid_len(img.rows, k - 1);
    let jc = grid_len(img.cols, k - 1);
    let w = (cn.1 - cn.0 + 1) as usize;
    // Source columns for each output column, mirrored only where the edge needs it.
    let idx = |i: i64| (reflect(i, jc) - prev.cols.0) as usize;
    let ia: Vec<usize> = (cn.0..=cn.1).map(|j| idx(2 * j - 1)).collect();
    let ib: Vec<usize> = (cn.0..=cn.1)
        .map(|j| (2 * j - prev.cols.0) as usize)
        .collect();
    let ic: Vec<usize> = (cn.0..=cn.1).map(|j| idx(2 * j + 1)).collect();
    // Along rows.
    let mut horiz = Planes::new(prev.rows, cn, carried);
    let (mut a, mut b, mut c) = (vec![0f32; w], vec![0f32; w], vec![0f32; w]);
    for p in 0..carried {
        for r in prev.rows.0..=prev.rows.1 {
            let src = prev.row(p, r);
            for i in 0..w {
                a[i] = src[ia[i]];
                b[i] = src[ib[i]];
                c[i] = src[ic[i]];
            }
            // a, b, c and the output row each hold w values.
            unsafe {
                halve(
                    isa,
                    w,
                    a.as_ptr(),
                    b.as_ptr(),
                    c.as_ptr(),
                    horiz.row_ptr(p, r),
                )
            };
        }
    }
    // Down columns.
    let mut next = Planes::new(rn, cn, carried);
    for p in 0..carried {
        for i in rn.0..=rn.1 {
            let ra = horiz.row(p, reflect(2 * i - 1, jr)).as_ptr();
            let rb = horiz.row(p, 2 * i).as_ptr();
            let rc = horiz.row(p, reflect(2 * i + 1, jr)).as_ptr();
            // Every row of these planes holds w values.
            unsafe { halve(isa, w, ra, rb, rc, next.row_ptr(p, i)) };
        }
    }
    Some(next)
}

/// A kernel that makes one plane from two.
type DeriveFn = unsafe fn(Isa, usize, *const f32, *const f32, *mut f32);

/// Cuts the output Slices of one Level from its planes, moving a plane when it is exactly the
/// output, and copying only when the Level computed more than the Window shows.
fn slices_at(
    func: Func,
    isa: Isa,
    k: u32,
    mut planes: Planes,
    rows: Option<(i64, i64)>,
    cols: Option<(i64, i64)>,
) -> Vec<Slice> {
    let (Some(rows), Some(cols)) = (rows, cols) else {
        return Vec::new();
    };
    let w = (cols.1 - cols.0 + 1) as usize;
    let h = (rows.1 - rows.0 + 1) as usize;
    let exact = rows == planes.rows && cols == planes.cols;
    let mut cut = |p: usize| -> Vec<f32> {
        if exact {
            return std::mem::take(&mut planes.planes[p]);
        }
        let mut out = Vec::with_capacity(w * h);
        let o = (cols.0 - planes.cols.0) as usize;
        for r in rows.0..=rows.1 {
            out.extend_from_slice(&planes.row(p, r)[o..o + w]);
        }
        out
    };
    let derive = |a: &[f32], b: &[f32], f: DeriveFn| -> Vec<f32> {
        let mut out = vec![0f32; w * h];
        assert!(a.len() == w * h && b.len() == w * h);
        // a, b and out each hold w × h values.
        unsafe { f(isa, w * h, a.as_ptr(), b.as_ptr(), out.as_mut_ptr()) };
        out
    };
    let make = |name, data| Slice {
        name,
        level: k,
        rows,
        cols,
        data,
    };
    match func {
        Func::Raw => vec![make("intensity", cut(0))],
        Func::Polar => {
            let magnitude = cut(0);
            let phase = if k == 0 {
                cut(3)
            } else {
                let (ure, uim) = (cut(1), cut(2));
                derive(&ure, &uim, phase_from)
            };
            vec![make("magnitude", magnitude), make("phase", phase)]
        }
        Func::Hsv => {
            let (saturation, value) = (cut(0), cut(1));
            let hue = if k == 0 {
                cut(4)
            } else {
                let (hc, hs) = (cut(2), cut(3));
                derive(&hc, &hs, hue_from)
            };
            vec![
                make("hue", hue),
                make("saturation", saturation),
                make("value", value),
            ]
        }
        Func::Mask | Func::Labels | Func::Range => unreachable!("checked above"),
    }
}
