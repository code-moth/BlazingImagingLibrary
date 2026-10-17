// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.
//! Slices from real Images (section 5): `read_slice` for `pixel.raw`, running the pass of
//! `bil::pass` on Fragments read through the cache, a band of rows at a time (M75 to M79).

use crate::error::{Error, Result};
use crate::pass::{self, Func, LevelMethod, Pass, ReadOptions, Rows};
use crate::simd::Isa;
use crate::source::Image;
use crate::types::{Fragment, PixelType, SampleType, Slice, Status, Window};

/// The most rows of one band the row source reads at once (M77).
const BAND_ROWS: u64 = 512;

/// The function whose Slices are built so far.
const RAW: &str = "pixel.raw";

/// Reads one Slice at one Level: the body of `Image::read_slice`.
pub(crate) fn read(
    image: &Image<'_>,
    slice_name: &str,
    window: Window,
    level: &str,
) -> Result<Slice> {
    let info = image.info()?;
    let (name, component) = raw_component(slice_name, &info.component_names)?;
    let pixel_type = info.pixel_type.ok_or_else(|| {
        Error::Unsupported(format!(
            "{slice_name}: components of different sample types, which `pixel.raw` does not \
             read yet"
        ))
    })?;
    if matches!(
        pixel_type.sample_type,
        SampleType::ComplexI8
            | SampleType::ComplexI16
            | SampleType::ComplexF32
            | SampleType::ComplexF64
    ) {
        return Err(Error::Unsupported(format!(
            "{slice_name}: complex samples, which the `complex` functions read (M78)"
        )));
    }
    let (base, halvings) = parse_level(level)?;
    // The grid the pass runs on: the Image's, or a stored level's, with the Window on it.
    let (rows, cols, on_grid, origin, scale, block_rows) = if base == "level0" {
        check_window(info.row_count, info.col_count, window)?;
        (
            info.row_count,
            info.col_count,
            window,
            [0.0, 0.0],
            [1.0, 1.0],
            u64::from(info.storage.block_size.0),
        )
    } else {
        let stored = image
            .list_stored_levels()?
            .iter()
            .find(|l| l.name == base)
            .ok_or_else(|| Error::NotFound(format!("level {level} of image {}", image.index())))?;
        check_window(info.row_count, info.col_count, window)?;
        let block_rows = u64::from(image.describe_storage(base)?.block_size.0);
        (
            stored.rows,
            stored.cols,
            stored.covered(window),
            [stored.position.0, stored.position.1],
            [stored.scale.0, stored.scale.1],
            block_rows,
        )
    };
    if halvings >= level_count(rows, cols) {
        return Err(Error::NotFound(format!(
            "level {level} of image {}: it offers {} Levels of {rows} × {cols}",
            image.index(),
            level_count(rows, cols)
        )));
    }
    let step = 1u64 << halvings;
    let empty = Slice {
        name: name.to_string(),
        label: name.to_string(),
        status: Status::Complete,
        level_name: level.to_string(),
        origin,
        scale: [scale[0] * step as f64, scale[1] * step as f64],
        window,
        row_count: 0,
        col_count: 0,
        data: Vec::new(),
        strides: [0, 4],
    };
    if on_grid.row_count == 0 || on_grid.col_count == 0 {
        return Ok(empty);
    }
    let mut source = FragmentRows {
        image: *image,
        level: base,
        rows: rows as usize,
        cols: cols as usize,
        component,
        pixel_type,
        masked: info.may_have_missing,
        block_rows: block_rows.max(1),
        band: None,
        failed: None,
    };
    let w = pass::Window {
        row_start: on_grid.row_start as usize,
        col_start: on_grid.col_start as usize,
        row_count: on_grid.row_count as usize,
        col_count: on_grid.col_count as usize,
    };
    let method = LevelMethod::Bilinear;
    let mut plan = Pass::for_levels(&source, Func::Raw, w, halvings, halvings, method);
    let mut out = plan.outputs();
    let Some(made) = out.get(halvings as usize).and_then(|s| s.first()) else {
        // No grid point of the Level lies in the Window.
        return Ok(empty);
    };
    let ((r0, _), (c0, _)) = (made.rows, made.cols);
    let options = ReadOptions {
        level_method: method,
        ..ReadOptions::default()
    };
    plan.run(&mut source, Isa::best(), options, &mut out);
    if let Some(e) = source.failed {
        return Err(e);
    }
    let made = out.swap_remove(halvings as usize).swap_remove(0);
    let (row_count, col_count) = (made.height() as u32, made.width() as u32);
    let status = if source.masked && made.data.iter().any(|v| v.is_nan()) {
        Status::Partial
    } else {
        Status::Complete
    };
    Ok(Slice {
        status,
        origin: [
            origin[0] + scale[0] * (r0 as u64 * step) as f64,
            origin[1] + scale[1] * (c0 as u64 * step) as f64,
        ],
        row_count,
        col_count,
        strides: [4 * col_count as isize, 4],
        data: made.data,
        ..empty
    })
}

/// The Slice name's own part and the component it reads, for `pixel.raw` (M75).
fn raw_component<'n>(slice_name: &'n str, names: &[String]) -> Result<(&'n str, usize)> {
    let Some((function, name)) = slice_name.rsplit_once('.') else {
        return Err(Error::NotFound(format!(
            "Slice {slice_name}: a Slice name has three parts, as pixel.raw.intensity"
        )));
    };
    if function != RAW {
        return Err(Error::Unsupported(format!(
            "Slice {slice_name}: only {RAW} is built so far"
        )));
    }
    let component = names.iter().position(|n| n == name).ok_or_else(|| {
        Error::NotFound(format!(
            "Slice {slice_name}: this Image's {RAW} Slices are {}",
            names.join(", ")
        ))
    })?;
    Ok((name, component))
}

/// A level name as the base the pass reads, `level0` or a stored level, and the halvings it makes
/// of it: `level3` is (`level0`, 3), `stored2` is (`stored2`, 0), `stored2.level1` is
/// (`stored2`, 1) (section 5).
fn parse_level(level: &str) -> Result<(&str, u32)> {
    let bad = || Error::NotFound(format!("level {level}"));
    let halvings = |n: &str| -> Result<u32> {
        let k = n.strip_prefix("level").ok_or_else(bad)?;
        if k.is_empty()
            || !k.bytes().all(|b| b.is_ascii_digit())
            || (k.len() > 1 && k.starts_with('0'))
        {
            return Err(bad());
        }
        k.parse().map_err(|_| bad())
    };
    match level.split_once('.') {
        Some((stored, n)) if stored.starts_with("stored") => Ok((stored, halvings(n)?)),
        Some(_) => Err(bad()),
        None if level.starts_with("stored") => Ok((level, 0)),
        None => Ok(("level0", halvings(level)?)),
    }
}

/// Level 0 and every halving until the grid is one point along its shorter side (section 5).
fn level_count(rows: u64, cols: u64) -> u32 {
    let shorter = rows.min(cols).max(1);
    // The halvings k with ⌈shorter / 2^k⌉ > 1, and Level 0.
    (u64::BITS - (shorter - 1).leading_zeros()) + 1
}

fn check_window(rows: u64, cols: u64, window: Window) -> Result<()> {
    let end = |start: u64, count: u32| start.checked_add(u64::from(count));
    if end(window.row_start, window.row_count).is_none_or(|e| e > rows)
        || end(window.col_start, window.col_count).is_none_or(|e| e > cols)
    {
        return Err(Error::NotFound(format!(
            "a Window outside the Image's {rows} × {cols} pixels"
        )));
    }
    Ok(())
}

/// The pass's rows, read from Fragments through the cache, one band of rows at a time (M77).
struct FragmentRows<'a> {
    image: Image<'a>,
    level: &'a str,
    rows: usize,
    cols: usize,
    component: usize,
    pixel_type: PixelType,
    masked: bool,
    /// Rows of one block row of the level's storage: a band starts and ends on them.
    block_rows: u64,
    /// The band read last.
    band: Option<Fragment>,
    /// The first failure; the rows asked after it are NaN, and the read reports it.
    failed: Option<Error>,
}

impl FragmentRows<'_> {
    /// The band holding row `r` and columns `c0` to `c1`, read where the last does not.
    fn band(&mut self, r: usize, c0: usize, c1: usize) -> Option<&Fragment> {
        let holds = |f: &Fragment| {
            let w = f.window;
            (r as u64) >= w.row_start
                && (r as u64) < w.row_start + u64::from(w.row_count)
                && c0 as u64 >= w.col_start
                && (c1 as u64) < w.col_start + u64::from(w.col_count)
        };
        if self.failed.is_some() {
            return None;
        }
        if !self.band.as_ref().is_some_and(holds) {
            // From row r to the end of its block row, at most BAND_ROWS rows: the pass asks for
            // rows in order, so each block row is read once.
            let r = r as u64;
            let end = ((r / self.block_rows + 1) * self.block_rows)
                .min(r + BAND_ROWS)
                .min(self.rows as u64);
            let window = Window {
                row_start: r,
                col_start: c0 as u64,
                row_count: (end - r) as u32,
                col_count: (c1 - c0 + 1) as u32,
            };
            let read = if self.level == "level0" {
                self.image.read_fragment(window, "level0")
            } else {
                self.image.read_stored(window, self.level)
            };
            match read {
                Ok(f) => self.band = Some(f),
                Err(e) => {
                    self.failed = Some(e);
                    self.band = None;
                    return None;
                }
            }
        }
        self.band.as_ref()
    }
}

impl Rows for FragmentRows<'_> {
    fn row_count(&self) -> usize {
        self.rows
    }
    fn col_count(&self) -> usize {
        self.cols
    }
    fn components(&self) -> usize {
        1
    }
    fn masked(&self) -> bool {
        self.masked
    }
    fn float(&self) -> bool {
        self.pixel_type.sample_type.is_float()
    }
    fn convert(&mut self, r: usize, c0: usize, c1: usize, step: usize, bufs: &mut [Vec<f32>]) {
        let n = (c1 - c0) / step + 1;
        let (component, shift) = (self.component, self.pixel_type.bit_shift);
        let out = &mut bufs[0];
        out.resize(n, 0.0);
        let Some(f) = self.band(r, c0, c1) else {
            out.fill(f32::NAN);
            return;
        };
        let at = (r - f.window.row_start as usize) * f.strides[0]
            + (c0 - f.window.col_start as usize) * f.strides[1]
            + component * f.strides[2];
        convert_samples(
            &f.data[at..],
            f.pixel_type.sample_type,
            shift,
            f.strides[1] * step,
            out,
        );
    }
    fn valid(&mut self, r: usize, c0: usize, c1: usize, step: usize, out: &mut [f32]) {
        let Some(f) = self.band(r, c0, c1) else {
            out.fill(0.0);
            return;
        };
        match &f.validity {
            Some(v) => {
                let width = f.window.col_count as usize;
                let at =
                    (r - f.window.row_start as usize) * width + (c0 - f.window.col_start as usize);
                for (d, &p) in out.iter_mut().zip(v[at..].iter().step_by(step)) {
                    *d = f32::from(u8::from(p));
                }
            }
            None => out.fill(1.0),
        }
    }
}

/// Converts `out.len()` samples, `stride` bytes apart from the first in `data`, to float32 by one
/// correctly rounded conversion: integers after `bit_shift`, an arithmetic shift for signed ones,
/// `F16` exactly, and the wider types rounded to nearest (M79).
fn convert_samples(data: &[u8], t: SampleType, shift: u32, stride: usize, out: &mut [f32]) {
    macro_rules! each {
        ($ty:ty, $n:expr, |$x:ident| $conv:expr) => {
            for (k, d) in out.iter_mut().enumerate() {
                let at = k * stride;
                let $x = <$ty>::from_ne_bytes(data[at..at + $n].try_into().unwrap());
                *d = $conv;
            }
        };
    }
    match t {
        SampleType::U8 => each!(u8, 1, |x| f32::from(x >> shift)),
        SampleType::I8 => each!(i8, 1, |x| f32::from(x >> shift)),
        SampleType::U16 => each!(u16, 2, |x| f32::from(x >> shift)),
        SampleType::I16 => each!(i16, 2, |x| f32::from(x >> shift)),
        SampleType::U32 => each!(u32, 4, |x| (x >> shift) as f32),
        SampleType::I32 => each!(i32, 4, |x| (x >> shift) as f32),
        SampleType::U64 => each!(u64, 8, |x| (x >> shift) as f32),
        SampleType::I64 => each!(i64, 8, |x| (x >> shift) as f32),
        SampleType::F16 => each!(u16, 2, |x| f16_to_f32(x)),
        SampleType::F32 => each!(f32, 4, |x| x),
        SampleType::F64 => each!(f64, 8, |x| x as f32),
        SampleType::ComplexI8
        | SampleType::ComplexI16
        | SampleType::ComplexF32
        | SampleType::ComplexF64 => unreachable!("pixel.raw refuses complex samples (M78)"),
    }
}

/// An IEEE half-precision number as float32, exactly.
fn f16_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1F);
    let man = u32::from(h & 0x3FF);
    let bits = match (exp, man) {
        (0, 0) => sign,
        // Subnormal: m × 2^-24, exact in float32.
        (0, m) => {
            let v = m as f32 * f32::from_bits(0x3380_0000);
            return if sign != 0 { -v } else { v };
        }
        (0x1F, 0) => sign | 0x7F80_0000,
        (0x1F, m) => sign | 0x7FC0_0000 | (m << 13),
        (e, m) => sign | ((e + 112) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_parsed() {
        assert_eq!(parse_level("level0").unwrap(), ("level0", 0));
        assert_eq!(parse_level("level3").unwrap(), ("level0", 3));
        assert_eq!(parse_level("stored2").unwrap(), ("stored2", 0));
        assert_eq!(parse_level("stored2.level1").unwrap(), ("stored2", 1));
        for bad in [
            "",
            "level",
            "level03",
            "levelx",
            "stored2.x",
            "x.level1",
            "level1.level1",
        ] {
            assert!(matches!(parse_level(bad), Err(Error::NotFound(_))), "{bad}");
        }
    }

    #[test]
    fn level_count_halves_to_one_point() {
        assert_eq!(level_count(1, 1), 1);
        assert_eq!(level_count(2, 9), 2);
        assert_eq!(level_count(3, 3), 3);
        assert_eq!(level_count(4, 100), 3);
        assert_eq!(level_count(5, 5), 4);
    }

    #[test]
    fn half_precision_is_exact() {
        for (h, v) in [
            (0x3C00u16, 1.0f32),
            (0xC000, -2.0),
            (0x7BFF, 65504.0),
            (0x0001, 2f32.powi(-24)),
            (0x8001, -(2f32.powi(-24))),
            (0x0400, 2f32.powi(-14)),
            (0x7C00, f32::INFINITY),
            (0x8000, -0.0),
        ] {
            assert_eq!(f16_to_f32(h).to_bits(), v.to_bits(), "{h:#06x}");
        }
        assert!(f16_to_f32(0x7E00).is_nan());
    }
}
