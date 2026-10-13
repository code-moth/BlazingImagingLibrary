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

/// Experiment 1b: the same pass on a rolling buffer of a few rows, into the caller's buffers.
pub mod rolling;
pub use rolling::{plan_outputs, read_levels_into, LevelMethod, MissingWeights, Pass, ReadOptions};

/// Test images and comparisons for the experiment programs.
pub mod testing;

#[cfg(target_arch = "x86_64")]
use simd::{Avx2, Avx512};
use simd::{Isa, Scalar};

/// Interleaved native samples of a whole Image, as a Fragment would hold them.
pub enum Samples {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
    ComplexI16(Vec<i16>),
    ComplexF32(Vec<f32>),
}

pub struct Image {
    pub rows: usize,
    pub cols: usize,
    /// Components per pixel: 1 for intensity and complex, 3 for color.
    pub comps: usize,
    pub samples: Samples,
    /// The validity mask of a Fragment, one byte a pixel: 0 where the pixel is missing. `None`
    /// means the Image cannot have missing pixels, so no checks are made.
    pub valid: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    /// `pixel.raw`, one component
    Raw,
    /// `complex.polar`
    Polar,
    /// `color.hsv`
    Hsv,
    /// `pixel.mask`: `valid` and `valid_fraction`, made from the validity mask
    Mask,
    /// `class.labels`: a label in each pixel, reduced by keeping every second value
    Labels,
    /// `depth.sensor`: a range in each pixel, reduced by the minimum of each 3 × 3 block
    Range,
}

/// How a kind of Slice is reduced at each halving (section 5 of the specification).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduce {
    /// The level method's weights, 1 2 1, leaving missing values out
    Average,
    /// The value at the grid point, as for labels and for `decimate`
    Keep,
    /// The least present value of the 3 × 3 block centered on the grid point, as for range
    Min3,
}

impl Func {
    pub fn slice_names(self) -> &'static [&'static str] {
        match self {
            Func::Raw => &["intensity"],
            Func::Polar => &["magnitude", "phase"],
            Func::Hsv => &["hue", "saturation", "value"],
            Func::Mask => &["valid", "valid_fraction"],
            Func::Labels => &["value"],
            Func::Range => &["range"],
        }
    }
    /// The function's name in the specification: family and function.
    pub fn name(self) -> &'static str {
        match self {
            Func::Raw => "pixel.raw",
            Func::Polar => "complex.polar",
            Func::Hsv => "color.hsv",
            Func::Mask => "pixel.mask",
            Func::Labels => "class.labels",
            Func::Range => "depth.sensor",
        }
    }
    /// How this function's Slices are reduced under the default level method.
    pub fn reduce(self) -> Reduce {
        match self {
            Func::Raw | Func::Polar | Func::Hsv | Func::Mask => Reduce::Average,
            Func::Labels => Reduce::Keep,
            Func::Range => Reduce::Min3,
        }
    }
    /// Planes made at Level 0: those carried to coarser Levels, then Level 0 extras.
    fn level0_planes(self) -> (usize, usize) {
        match self {
            Func::Raw => (1, 0),
            Func::Polar => (3, 1),
            Func::Hsv => (4, 1),
            Func::Mask | Func::Labels | Func::Range => (1, 0),
        }
    }
    fn inputs(self) -> usize {
        match self {
            Func::Raw | Func::Mask | Func::Labels | Func::Range => 1,
            Func::Polar => 2,
            Func::Hsv => 3,
        }
    }
}

/// A rectangle of the Image in full-resolution rows and columns.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub row_start: usize,
    pub col_start: usize,
    pub row_count: usize,
    pub col_count: usize,
}

/// One Slice at one Level: values at grid indices `rows` × `cols` of that Level.
pub struct Slice {
    pub name: &'static str,
    pub level: u32,
    pub rows: (i64, i64),
    pub cols: (i64, i64),
    pub data: Vec<f32>,
}

impl Slice {
    pub fn width(&self) -> usize {
        (self.cols.1 - self.cols.0 + 1) as usize
    }
    pub fn height(&self) -> usize {
        (self.rows.1 - self.rows.0 + 1) as usize
    }
    /// The value at grid index (r, c) of this Slice's Level.
    pub fn at(&self, r: i64, c: i64) -> f32 {
        self.data[((r - self.rows.0) as usize) * self.width() + (c - self.cols.0) as usize]
    }
}

// ---------------------------------------------------------------- dispatch

macro_rules! dispatch {
    ($name:ident ( $($arg:ident : $ty:ty),* )) => {
        /// Runs the kernel on the chosen instruction set.
        ///
        /// # Safety
        /// Every pointer must be valid for `n` f32 values, inputs for reading and outputs for
        /// writing, and `isa` must be available on this processor.
        #[allow(clippy::too_many_arguments)]
        pub unsafe fn $name(isa: Isa, $($arg: $ty),*) {
            unsafe {
                match isa {
                    Isa::Scalar => kernels::$name::<Scalar>($($arg),*),
                    #[cfg(target_arch = "x86_64")]
                    Isa::Avx2 => {
                        #[target_feature(enable = "avx2")]
                        unsafe fn run($($arg: $ty),*) { kernels::$name::<Avx2>($($arg),*) }
                        run($($arg),*)
                    }
                    #[cfg(target_arch = "x86_64")]
                    Isa::Avx512 => {
                        #[target_feature(enable = "avx512f")]
                        unsafe fn run($($arg: $ty),*) { kernels::$name::<Avx512>($($arg),*) }
                        run($($arg),*)
                    }
                    #[cfg(not(target_arch = "x86_64"))]
                    _ => unreachable!(),
                }
            }
        }
    };
}

dispatch!(halve(n: usize, a: *const f32, b: *const f32, c: *const f32, out: *mut f32));
dispatch!(polar0(n: usize, re: *const f32, im: *const f32, out: [*mut f32; 4]));
dispatch!(phase_from(n: usize, ure: *const f32, uim: *const f32, out: *mut f32));
dispatch!(hsv0(n: usize, r: *const f32, g: *const f32, b: *const f32, out: [*mut f32; 5]));
dispatch!(hue_from(n: usize, hc: *const f32, hs: *const f32, out: *mut f32));
dispatch!(halve_row(n: usize, src: *const f32, out: *mut f32));
dispatch!(halve_weighted(n: usize, x: [*const f32; 3], c: [*const f32; 3], out: *mut f32));
dispatch!(halve_row_weighted(n: usize, x: *const f32, c: *const f32, out: *mut f32));
dispatch!(halve_over(n: usize, s: [*const f32; 3], w: *const f32, out: *mut f32));
dispatch!(present_one(n: usize, w: *const f32, out: *mut f32));
dispatch!(weight_present(n: usize, x: *const f32, c: *const f32, out: *mut f32));
dispatch!(mask_planes1(n: usize, rows: [*mut f32; 2]));
dispatch!(mask_planes4(n: usize, rows: [*mut f32; 5]));
dispatch!(mask_planes5(n: usize, rows: [*mut f32; 6]));
dispatch!(min3_present(n: usize, a: *const f32, b: *const f32, c: *const f32, out: *mut f32));
dispatch!(min3_row(n: usize, src: *const f32, out: *mut f32));
dispatch!(keep_present(n: usize, probe: *const f32, x: *const f32, out: *mut f32));
dispatch!(blur5(n: usize, x: [*const f32; 5], out: *mut f32));
dispatch!(blur5_row(n: usize, src: *const f32, out: *mut f32));
dispatch!(blur5_weighted(n: usize, x: [*const f32; 5], c: [*const f32; 5], out: *mut f32));
dispatch!(blur5_row_weighted(n: usize, x: *const f32, c: *const f32, out: *mut f32));
dispatch!(blur5_over(n: usize, s: [*const f32; 5], w: *const f32, out: *mut f32));

// ---------------------------------------------------------------- grid arithmetic

/// Number of grid points of Level k along an axis of length n.
fn grid_len(n: usize, k: u32) -> i64 {
    (((n - 1) >> k) + 1) as i64
}

/// Mirror index i into [0, j), reflecting about the first and last grid point.
fn reflect(i: i64, j: i64) -> i64 {
    if j == 1 {
        return 0;
    }
    let p = 2 * (j - 1);
    let m = i.rem_euclid(p);
    if m >= j {
        p - m
    } else {
        m
    }
}

/// Grid indices of Level k that fall inside [w0, w1], if any.
fn out_range(w0: usize, w1: usize, k: u32) -> Option<(i64, i64)> {
    let s = 1usize << k;
    let a = w0.div_ceil(s) as i64;
    let b = (w1 / s) as i64;
    (a <= b).then_some((a, b))
}

/// Grid indices at each Level, where there are any.
type LevelRanges = Vec<Option<(i64, i64)>>;

/// For one axis: the output range at each Level, and the range of valid grid indices that must
/// be computed at each Level so that every output can be made by a halving of radius 1.
fn axis_plan(n: usize, w0: usize, w1: usize, top: u32) -> (LevelRanges, LevelRanges) {
    axis_plan_radius(n, w0, w1, top, 1)
}

/// `axis_plan` for a halving that reaches `radius` grid points of the Level before on each side.
fn axis_plan_radius(
    n: usize,
    w0: usize,
    w1: usize,
    top: u32,
    radius: i64,
) -> (LevelRanges, LevelRanges) {
    let outs: Vec<_> = (0..=top).map(|k| out_range(w0, w1, k)).collect();
    let mut need = vec![None; top as usize + 1];
    for k in (0..=top as usize).rev() {
        let mut lo = i64::MAX;
        let mut hi = i64::MIN;
        if let Some((a, b)) = outs[k] {
            lo = lo.min(a);
            hi = hi.max(b);
        }
        if k < top as usize {
            if let Some((a, b)) = need[k + 1] {
                let j = grid_len(n, k as u32);
                for i in (2 * a - radius)..=(2 * b + radius) {
                    let r = reflect(i, j);
                    lo = lo.min(r);
                    hi = hi.max(r);
                }
            }
        }
        need[k] = (lo <= hi).then_some((lo, hi));
    }
    (outs, need)
}

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

// ---------------------------------------------------------------- the pass

/// Converts one row's columns [c0, c1] of native samples to f32 component rows. Exact for every
/// sample type here.
fn convert_row(img: &Image, r: usize, c0: usize, c1: usize, bufs: &mut [Vec<f32>]) {
    let n = c1 - c0 + 1;
    for b in bufs.iter_mut() {
        b.resize(n, 0.0);
    }
    #[inline(always)]
    fn spread<T: Copy>(src: &[T], comps: usize, bufs: &mut [Vec<f32>], conv: impl Fn(T) -> f32) {
        match comps {
            1 => {
                for (d, s) in bufs[0].iter_mut().zip(src) {
                    *d = conv(*s);
                }
            }
            2 => {
                let (a, b) = bufs.split_at_mut(1);
                for ((x, y), s) in a[0]
                    .iter_mut()
                    .zip(b[0].iter_mut())
                    .zip(src.as_chunks::<2>().0)
                {
                    *x = conv(s[0]);
                    *y = conv(s[1]);
                }
            }
            3 => {
                let (a, rest) = bufs.split_at_mut(1);
                let (b, c) = rest.split_at_mut(1);
                for (((x, y), z), s) in a[0]
                    .iter_mut()
                    .zip(b[0].iter_mut())
                    .zip(c[0].iter_mut())
                    .zip(src.as_chunks::<3>().0)
                {
                    *x = conv(s[0]);
                    *y = conv(s[1]);
                    *z = conv(s[2]);
                }
            }
            _ => unreachable!(),
        }
    }
    let per = match img.samples {
        Samples::ComplexI16(_) | Samples::ComplexF32(_) => 2,
        _ => img.comps,
    };
    let a = (r * img.cols + c0) * per;
    let b = (r * img.cols + c1 + 1) * per;
    match &img.samples {
        Samples::U8(v) => spread(&v[a..b], per, bufs, |x| x as f32),
        Samples::U16(v) => spread(&v[a..b], per, bufs, |x| x as f32),
        Samples::F32(v) => spread(&v[a..b], per, bufs, |x| x),
        Samples::ComplexI16(v) => spread(&v[a..b], per, bufs, |x| x as f32),
        Samples::ComplexF32(v) => spread(&v[a..b], per, bufs, |x| x),
    }
}

/// Makes every Slice of `func` for `window` at Levels 0 to `top`, in one pass over the native
/// samples the Window and its margins need.
pub fn read_levels(img: &Image, func: Func, isa: Isa, window: Window, top: u32) -> Vec<Vec<Slice>> {
    assert!(
        isa == Isa::Scalar || Isa::available().contains(&isa),
        "{isa:?} is not available"
    );
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
