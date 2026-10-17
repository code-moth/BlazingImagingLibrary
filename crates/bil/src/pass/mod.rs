// SPDX-License-Identifier: LGPL-2.1-or-later
//! The pass of experiment 1b, which makes a slice function's Slices at every Level from 0 to n in
//! one sweep of rows, on BIL's own kernels (section 5). Moved here from the experiment so that
//! `read_slice` runs it on Fragments read through the cache; the experiment keeps its full-plane
//! reference, `read_levels`, and its checks, and uses this module.
//!
//! The rules follow section 5 of the specification:
//! - Level n holds the Image pixels whose row and column are multiples of 2^n.
//! - Each halving applies weights 1 2 1, divided by 4, along rows and then columns.
//! - Angles (phase, hue) are reduced as unit vectors.
//! - At an Image edge, values are mirrored about the first and last grid point of each Level.

// The pass keeps kernels short and close to the specification's formulas.
#![allow(
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::excessive_precision
)]

use crate::kernels;
use crate::simd;

/// The rolling pass: rows streamed through every Level on a buffer of a few rows.
pub mod rolling;
pub use rolling::{
    band_threads, plan_outputs, read_levels_into, BandedPass, LevelMethod, MissingWeights, Pass,
    ReadOptions, MIN_BAND_PIXELS,
};

/// Threads kept between reads, for reads split into bands.
pub mod pool;

#[cfg(target_arch = "aarch64")]
use simd::Neon;
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

/// Where the pass reads its rows: the shape of the Image and, row by row, its samples as f32.
/// The pass asks for rows in order, top to bottom, and only for rows and columns inside the
/// Image; it mirrors at the edges itself.
pub trait Rows {
    /// Rows of the whole Image, or of the stored level read.
    fn row_count(&self) -> usize;
    /// Columns of the whole Image, or of the stored level read.
    fn col_count(&self) -> usize;
    /// Values each pixel's samples hold: two for complex samples, else the components.
    fn components(&self) -> usize;
    /// Whether pixels may be missing, so that the pass checks for them.
    fn masked(&self) -> bool;
    /// Whether the samples are floating-point, where a stored NaN is missing.
    fn float(&self) -> bool;
    /// Converts every `step`-th column of row `r`, from column `c0` to column `c1`, which is one
    /// of them, to f32: one row of `bufs` for each value of the pixel, from the first.
    fn convert(&mut self, r: usize, c0: usize, c1: usize, step: usize, bufs: &mut [Vec<f32>]);
    /// The same columns' validity, 1.0 where the pixel is present and 0.0 where it is missing.
    /// Asked only where `masked` is true.
    fn valid(&mut self, r: usize, c0: usize, c1: usize, step: usize, out: &mut [f32]);
}

impl Rows for &Image {
    fn row_count(&self) -> usize {
        self.rows
    }
    fn col_count(&self) -> usize {
        self.cols
    }
    fn components(&self) -> usize {
        match self.samples {
            Samples::ComplexI16(_) | Samples::ComplexF32(_) => 2,
            _ => self.comps,
        }
    }
    fn masked(&self) -> bool {
        self.valid.is_some()
    }
    fn float(&self) -> bool {
        matches!(self.samples, Samples::F32(_) | Samples::ComplexF32(_))
    }
    fn convert(&mut self, r: usize, c0: usize, c1: usize, step: usize, bufs: &mut [Vec<f32>]) {
        convert_row_step(self, r, c0, c1, step, bufs)
    }
    fn valid(&mut self, r: usize, c0: usize, c1: usize, step: usize, out: &mut [f32]) {
        let mask = self
            .valid
            .as_ref()
            .expect("asked only where pixels may be missing");
        let at = r * self.cols;
        let bytes = mask[at + c0..=at + c1].iter().step_by(step);
        for (v, &b) in out.iter_mut().zip(bytes) {
            *v = f32::from(u8::from(b != 0));
        }
    }
}

impl Rows for Image {
    fn row_count(&self) -> usize {
        self.rows
    }
    fn col_count(&self) -> usize {
        self.cols
    }
    fn components(&self) -> usize {
        (&self).components()
    }
    fn masked(&self) -> bool {
        self.valid.is_some()
    }
    fn float(&self) -> bool {
        (&self).float()
    }
    fn convert(&mut self, r: usize, c0: usize, c1: usize, step: usize, bufs: &mut [Vec<f32>]) {
        convert_row_step(self, r, c0, c1, step, bufs)
    }
    fn valid(&mut self, r: usize, c0: usize, c1: usize, step: usize, out: &mut [f32]) {
        (&*self).valid(r, c0, c1, step, out)
    }
}

impl<T: Rows + ?Sized> Rows for &mut T {
    fn row_count(&self) -> usize {
        (**self).row_count()
    }
    fn col_count(&self) -> usize {
        (**self).col_count()
    }
    fn components(&self) -> usize {
        (**self).components()
    }
    fn masked(&self) -> bool {
        (**self).masked()
    }
    fn float(&self) -> bool {
        (**self).float()
    }
    fn convert(&mut self, r: usize, c0: usize, c1: usize, step: usize, bufs: &mut [Vec<f32>]) {
        (**self).convert(r, c0, c1, step, bufs)
    }
    fn valid(&mut self, r: usize, c0: usize, c1: usize, step: usize, out: &mut [f32]) {
        (**self).valid(r, c0, c1, step, out)
    }
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
    pub fn level0_planes(self) -> (usize, usize) {
        match self {
            Func::Raw => (1, 0),
            Func::Polar => (3, 1),
            Func::Hsv => (4, 1),
            Func::Mask | Func::Labels | Func::Range => (1, 0),
        }
    }
    pub fn inputs(self) -> usize {
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
                    #[cfg(target_arch = "aarch64")]
                    Isa::Neon => {
                        #[target_feature(enable = "neon")]
                        unsafe fn run($($arg: $ty),*) { kernels::$name::<Neon>($($arg),*) }
                        run($($arg),*)
                    }
                    #[cfg(not(target_arch = "x86_64"))]
                    Isa::Avx2 | Isa::Avx512 => unreachable!("not an x86 processor"),
                    #[cfg(not(target_arch = "aarch64"))]
                    Isa::Neon => unreachable!("not an ARM processor"),
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
pub fn grid_len(n: usize, k: u32) -> i64 {
    (((n - 1) >> k) + 1) as i64
}

/// Mirror index i into [0, j), reflecting about the first and last grid point.
pub fn reflect(i: i64, j: i64) -> i64 {
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
pub fn out_range(w0: usize, w1: usize, k: u32) -> Option<(i64, i64)> {
    let s = 1usize << k;
    let a = w0.div_ceil(s) as i64;
    let b = (w1 / s) as i64;
    (a <= b).then_some((a, b))
}

/// Grid indices at each Level, where there are any.
pub type LevelRanges = Vec<Option<(i64, i64)>>;

/// For one axis: the output range at each Level, and the range of valid grid indices that must
/// be computed at each Level so that every output can be made by a halving of radius 1.
pub fn axis_plan(n: usize, w0: usize, w1: usize, top: u32) -> (LevelRanges, LevelRanges) {
    axis_plan_radius(n, w0, w1, top, 1)
}

/// `axis_plan` for a halving that reaches `radius` grid points of the Level before on each side.
pub fn axis_plan_radius(
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

// ---------------------------------------------------------------- the pass

/// Converts every `step`-th column of row `r`, from column `c0` to column `c1`, which is one of
/// them, to f32 component rows. With a step of 1 it is `convert_row`.
pub fn convert_row_step(
    img: &Image,
    r: usize,
    c0: usize,
    c1: usize,
    step: usize,
    bufs: &mut [Vec<f32>],
) {
    if step == 1 {
        return convert_row(img, r, c0, c1, bufs);
    }
    let n = (c1 - c0) / step + 1;
    let per = match img.samples {
        Samples::ComplexI16(_) | Samples::ComplexF32(_) => 2,
        _ => img.comps,
    };
    for b in bufs.iter_mut() {
        b.resize(n, 0.0);
    }
    #[inline(always)]
    fn gather<T: Copy>(
        src: &[T],
        at: usize,
        step: usize,
        per: usize,
        bufs: &mut [Vec<f32>],
        conv: impl Fn(T) -> f32,
    ) {
        for (k, b) in bufs.iter_mut().enumerate() {
            for (j, d) in b.iter_mut().enumerate() {
                *d = conv(src[at + j * step * per + k]);
            }
        }
    }
    let at = (r * img.cols + c0) * per;
    match &img.samples {
        Samples::U8(v) => gather(v, at, step, per, bufs, |x| x as f32),
        Samples::U16(v) => gather(v, at, step, per, bufs, |x| x as f32),
        Samples::F32(v) => gather(v, at, step, per, bufs, |x| x),
        Samples::ComplexI16(v) => gather(v, at, step, per, bufs, |x| x as f32),
        Samples::ComplexF32(v) => gather(v, at, step, per, bufs, |x| x),
    }
}

/// Converts one row's columns [c0, c1] of native samples to f32 component rows. Exact for every
/// sample type here.
pub fn convert_row(img: &Image, r: usize, c0: usize, c1: usize, bufs: &mut [Vec<f32>]) {
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
