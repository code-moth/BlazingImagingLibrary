// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The types of specification section 4 that the readers fill: pixel types, Windows, the
//! storage of an Image's blocks, and the native pixels of a Window.

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

/// One block of stored pixels and where it lies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub image: u32,
    pub level: u32,
    pub frame: u64,
    pub block_row: u32,
    pub block_col: u32,
    /// The components it holds, as a range.
    pub components: std::ops::Range<u32>,
    pub offset: u64,
    pub length: u64,
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
