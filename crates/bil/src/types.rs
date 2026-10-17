// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The types of specification section 4 that the readers fill: pixel types, Windows, the
//! storage of an Image's blocks, and the native pixels of a Window.

/// What a field group is attached to (section 14.1): the Source, or one of its Images.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Source,
    Image(usize),
}

/// How a request turned out (section 3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Complete,
    Partial,
    Missing,
    NotConverged,
    Ambiguous,
}

/// A sample's type (section 4). Complex is a sample type, not two components.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SampleType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F16,
    F32,
    F64,
    ComplexI8,
    ComplexI16,
    ComplexF32,
    ComplexF64,
}

impl SampleType {
    /// Whether it holds floating-point numbers, where a stored NaN is missing (section 5).
    pub fn is_float(self) -> bool {
        matches!(
            self,
            SampleType::F16
                | SampleType::F32
                | SampleType::F64
                | SampleType::ComplexF32
                | SampleType::ComplexF64
        )
    }

    /// Bytes in one sample.
    pub fn size(self) -> usize {
        match self {
            SampleType::U8 | SampleType::I8 => 1,
            SampleType::U16 | SampleType::I16 | SampleType::F16 | SampleType::ComplexI8 => 2,
            SampleType::U32 | SampleType::I32 | SampleType::F32 | SampleType::ComplexI16 => 4,
            SampleType::U64 | SampleType::I64 | SampleType::F64 | SampleType::ComplexF32 => 8,
            SampleType::ComplexF64 => 16,
        }
    }

    /// Bytes in one number of the sample: the sample itself, or for a complex sample one of its
    /// two parts. Byte order applies to each number.
    pub fn number_size(self) -> usize {
        match self {
            SampleType::ComplexI8
            | SampleType::ComplexI16
            | SampleType::ComplexF32
            | SampleType::ComplexF64 => self.size() / 2,
            _ => self.size(),
        }
    }
}

/// A pixel's type: `component_count` samples of one sample type, with the bits that carry the
/// value (section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelType {
    pub sample_type: SampleType,
    pub component_count: u32,
    pub significant_bits: u32,
    /// Unused low bits below the value; 0 for most data.
    pub bit_shift: u32,
}

/// A rectangle of an Image at full resolution (section 3). This preview has no `step` and no
/// `frame`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub row_start: u64,
    pub col_start: u64,
    pub row_count: u32,
    pub col_count: u32,
}

/// Reduced-resolution data a file holds of an Image: one of its stored levels (section 5).
#[derive(Clone, Debug, PartialEq)]
pub struct StoredLevel {
    /// `stored1`, `stored2` and so on, from the finest.
    pub name: String,
    pub rows: u64,
    pub cols: u64,
    /// Full-resolution rows and columns for each of its own, down and across.
    pub scale: (f64, f64),
    /// Where its first pixel's center lies, in full-resolution rows and columns, counting each
    /// pixel's center as a whole number (M47).
    pub position: (f64, f64),
    /// Whether its samples sit on the Level grid; `None` where the file does not say.
    pub aligned: Option<bool>,
    /// Pixel i's center down and across, exactly: (a + b·i) / d full-resolution pixels.
    centers: [(i128, i128, i128); 2],
}

impl StoredLevel {
    /// A level covering its Image's extent, as a TIFF overview does (M47): its pixel i spans
    /// full-resolution pixels i·R/r to (i + 1)·R/r, for an Image of R and a level of r.
    pub fn covering_extent(rows: u64, cols: u64, full_rows: u64, full_cols: u64) -> StoredLevel {
        let axis = |r: u64, full: u64| {
            let (r, full) = (i128::from(r.max(1)), i128::from(full));
            // ((2i + 1)·R/r − 1) / 2 = (R − r + 2R·i) / 2r.
            (full - r, 2 * full, 2 * r)
        };
        let (down, across) = (axis(rows, full_rows), axis(cols, full_cols));
        let f = |(a, b, d): (i128, i128, i128)| (b as f64 / d as f64, a as f64 / d as f64);
        StoredLevel {
            name: String::new(),
            rows,
            cols,
            scale: (f(down).0, f(across).0),
            position: (f(down).1, f(across).1),
            aligned: None,
            centers: [down, across],
        }
    }

    /// A level whose pixel i lies on full-resolution pixel i·2^k, as a JPEG 2000 resolution
    /// level does: aligned, by the author's ruling on M48 (version 76).
    pub fn on_grid(name: String, rows: u64, cols: u64, k: u32) -> StoredLevel {
        let s = 1i128 << k.min(62);
        StoredLevel {
            name,
            rows,
            cols,
            scale: (s as f64, s as f64),
            position: (0.0, 0.0),
            aligned: Some(true),
            centers: [(0, s, 1); 2],
        }
    }

    /// The stored pixels whose centers fall inside a full-resolution Window (section 5, "Read
    /// directly"), as a Window in the stored level's own rows and columns (M46). Exact: a center
    /// on the Window's first edge is inside it.
    pub fn covered(&self, window: Window) -> Window {
        // The Window holds centers from start − ½ up to, not including, start + count − ½, so
        // pixel i is inside where (2·start − 1)·d ≤ 2a + 2b·i < (2·end − 1)·d.
        let first = |edge: u64, (a, b, d): (i128, i128, i128), n: u64| -> u64 {
            let need = (2 * i128::from(edge) - 1) * d - 2 * a;
            let i = need.div_euclid(2 * b) + i128::from(need.rem_euclid(2 * b) != 0);
            i.clamp(0, i128::from(n)) as u64
        };
        let span = |start: u64, count: u32, centers, n| -> (u64, u32) {
            let a = first(start, centers, n);
            let b = first(start + u64::from(count), centers, n);
            (a, (b - a).min(u64::from(u32::MAX)) as u32)
        };
        let (row_start, row_count) = span(
            window.row_start,
            window.row_count,
            self.centers[0],
            self.rows,
        );
        let (col_start, col_count) = span(
            window.col_start,
            window.col_count,
            self.centers[1],
            self.cols,
        );
        Window {
            row_start,
            col_start,
            row_count,
            col_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(start: u64, count: u32) -> Window {
        Window {
            row_start: start,
            col_start: start,
            row_count: count,
            col_count: count,
        }
    }

    /// Against exact arithmetic, every size and Window: a center on the first edge is inside.
    #[test]
    fn covered_is_exact() {
        for full in 2..120u64 {
            for r in [full / 2, full / 3, full / 4, full / 7]
                .into_iter()
                .filter(|&r| r > 0)
            {
                let l = StoredLevel::covering_extent(r, r, full, full);
                for start in 0..full {
                    for count in 0..=(full - start) as u32 {
                        let got = l.covered(w(start, count));
                        // Pixel i's center, doubled and times r: (2i + 1)·full − r. Inside where
                        // (2·start − 1)·r ≤ that < (2·end − 1)·r.
                        let inside: Vec<u64> = (0..r)
                            .filter(|&i| {
                                let c = (2 * i as i64 + 1) * full as i64 - r as i64;
                                let lo = (2 * start as i64 - 1) * r as i64;
                                let hi = (2 * (start + u64::from(count)) as i64 - 1) * r as i64;
                                lo <= c && c < hi
                            })
                            .collect();
                        let want = (
                            inside.first().copied().unwrap_or(got.row_start),
                            inside.len() as u32,
                        );
                        assert_eq!(
                            (got.row_start, got.row_count),
                            want,
                            "{full} {r} {start} {count}"
                        );
                    }
                }
            }
        }
        // The case the checking agent found: Image 396, level 50, Window from 99.
        let l = StoredLevel::covering_extent(50, 50, 396, 396);
        assert_eq!(l.covered(w(99, 10)).row_start, 12);
        // On a grid of 4: samples at 0, 4, 8, ...; a Window from 4 to 8 holds 4 only.
        let g = StoredLevel::on_grid("stored2".into(), 8, 8, 2);
        assert_eq!(g.aligned, Some(true));
        assert_eq!(l.aligned, None);
        assert_eq!(
            g.covered(w(4, 4)),
            Window {
                row_start: 1,
                col_start: 1,
                row_count: 1,
                col_count: 1
            }
        );
    }
}

/// The level a name gives: 0 for `level0`, full resolution, and n for `stored`n.
pub fn stored_level_number(name: &str) -> Option<usize> {
    if name == "level0" {
        return Some(0);
    }
    let n = name.strip_prefix("stored")?;
    if n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok().filter(|&n| n > 0)
}

/// How components lie within a block (sections 4 and 10.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interleave {
    Pixel,
    Line,
    Band,
}

/// Whether the blocks' places in the file are known (section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageOrder {
    Known,
    Unknown,
}

/// One run of a block's bytes in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

/// One block of stored pixels and where it lies: one byte range, or several where its data lies
/// in parts, as a JPEG 2000 tile's tile-parts may (section 4, M74).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub image: u32,
    pub level: u32,
    pub frame: u64,
    pub block_row: u32,
    pub block_col: u32,
    /// The components it holds, as a range.
    pub components: std::ops::Range<u32>,
    /// Its byte ranges, in the order the block's data is read.
    pub ranges: Vec<ByteRange>,
}

impl Block {
    /// Where its first byte range starts, by which blocks are listed in file order.
    pub fn offset(&self) -> u64 {
        self.ranges.first().map_or(0, |r| r.offset)
    }

    /// Its bytes in the file over all its ranges.
    pub fn length(&self) -> u64 {
        self.ranges.iter().map(|r| r.length).sum()
    }
}

/// How an Image's pixels lie in the file (section 4, "Access patterns").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    /// Rows and columns of one block.
    pub block_size: (u32, u32),
    pub interleave: Interleave,
    pub order: StorageOrder,
    /// Where the order is known, the blocks in the order they lie in the file. A block that
    /// was never recorded lies nowhere and is not listed.
    pub blocks: Vec<Block>,
}

/// How many frames an Image holds, and how sure that is (section 3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameCount {
    /// Exactly this many.
    Known(u64),
    /// At least this many, as in a stream not yet read to its end.
    AtLeast(u64),
    /// Not known without reading the whole stream.
    Unknown,
}

/// How one Image relates to another (section 3, "Related Images").
#[derive(Clone, Debug, PartialEq)]
pub enum RelationKind {
    /// The Images line up pixel for pixel.
    SameGrid,
    /// The other Image covers the same scene at a whole-number reduction, down and across,
    /// its first sample's center at `position` in this Image's pixels.
    ScaledGrid {
        reduction: (u32, u32),
        position: (f64, f64),
    },
}

/// A related Image, by its index.
#[derive(Clone, Debug, PartialEq)]
pub struct Relation {
    pub image: usize,
    pub kind: RelationKind,
}

/// What an Image's samples are (section 3, "Representations").
#[derive(Clone, Debug, PartialEq)]
pub enum Representation {
    /// `white_is_zero` where the samples, as stored, count down from white (TIFF Photometric 0).
    Intensity { white_is_zero: bool },
    /// Color, in its model.
    Color(ColorModel),
    /// Indices into a color table.
    Palette,
    /// Complex samples, whatever the file declares.
    Complex,
    /// Bands of one scene, such as multispectral data.
    Spectral,
    /// Samples whose meaning the file does not state.
    Vector,
}

/// A color representation's model.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorModel {
    Rgb,
    /// Luma weights of red, green and blue: BT.601's (0.299, 0.587, 0.114) unless stated.
    YCbCr {
        coefficients: (f64, f64, f64),
    },
    Lab {
        encoding: LabEncoding,
    },
    Cmyk,
}

/// How Lab samples are encoded, as TIFF's Photometric 8, 9 and 10 state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabEncoding {
    /// CIELab (Photometric 8).
    Cie,
    /// ICCLab (Photometric 9).
    Icc,
    /// ITULab (Photometric 10).
    Itu,
}

/// BT.601's luma weights.
pub const BT601: (f64, f64, f64) = (0.299, 0.587, 0.114);

impl Representation {
    /// Its snake_case name.
    pub fn name(&self) -> &'static str {
        match self {
            Representation::Intensity { .. } => "intensity",
            Representation::Color(_) => "color",
            Representation::Palette => "palette",
            Representation::Complex => "complex",
            Representation::Spectral => "spectral",
            Representation::Vector => "vector",
        }
    }
}

/// Component names as section 3 gives them: `None` becomes `component`k, and a name that would
/// repeat takes its place number, counted from 1.
pub fn component_names(names: impl IntoIterator<Item = Option<&'static str>>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names
        .into_iter()
        .enumerate()
        .map(|(i, n)| match n {
            Some(n) if seen.insert(n) => n.to_string(),
            Some(n) => format!("{n}{}", i + 1),
            None => format!("component{}", i + 1),
        })
        .collect()
}

/// One Image, described the same way by every driver (section 3, `ImageInfo`).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageInfo {
    pub row_count: u64,
    pub col_count: u64,
    pub frame_count: FrameCount,
    pub representation: Representation,
    pub component_names: Vec<String>,
    /// `None` where BIL's types do not hold the samples; reads then fail, and the reader's own
    /// description says why.
    pub pixel_type: Option<PixelType>,
    /// The number of components, given even where `pixel_type` is `None`.
    pub component_count: u32,
    pub relations: Vec<Relation>,
    /// The block size, the interleave and whether the block order is known: the first three
    /// parts of `describe_storage("level0")`, with no blocks listed.
    pub storage: Storage,
    /// Whether this Image can have missing pixels (section 5, "No cost where nothing is
    /// missing").
    pub may_have_missing: bool,
}

/// The native pixels of a Window, owned (section 4). This preview fills the parts of a Fragment
/// that a container reader can: the samples, in native byte order, pixel-interleaved, and the
/// pixels never recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment {
    pub window: Window,
    pub pixel_type: PixelType,
    /// Bytes from one row, one column and one component to the next.
    pub strides: [usize; 3],
    pub data: Vec<u8>,
    /// One entry for each pixel, row by row: `false` where the pixel was never recorded.
    /// `None` when every pixel was.
    pub validity: Option<Vec<bool>>,
}
