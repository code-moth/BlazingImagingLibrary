// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Tier 1's GeoTIFF reader (specification, sections 1.4 and 10): TIFF and BigTIFF, through the
//! `tiff` crate, with BIL's own reading of GeoTIFF keys.
//!
//! Opening walks the chain of image file directories (IFDs) and reads each one's tags, and
//! nothing else: no pixels. Each full-resolution IFD is an Image, and the reduced-resolution
//! IFDs after it, its overviews, are its stored levels, `stored1` the finest (section 5). The
//! tags of an Image's IFD, and its GeoTIFF keys, are field groups, extracted when first asked
//! for and held (section 14.3).
//!
//! The `tiff` crate reads the directories and their values; BIL reads each chunk's stored
//! bytes itself and decodes them (`chunks`): uncompressed, Deflate, LZW and PackBits, the
//! compression Tier 1 reads, with their predictors, so samples are always as stored, whatever
//! the photometric interpretation (M39, option (b)). Other compression is reported, by codec
//! name, and not decoded. Text is read from its own bytes, as Latin-1. GDAL's no-data tag,
//! GDAL_NODATA, is a fill value: a pixel whose every sample holds it is missing (section 4).
//!
//! Every offset and count is checked against the file before it is used, so a corrupt file is
//! an error, never a crash: a chunk is decoded into memory only when its stored bytes could
//! make that much of it, so what a read takes stays in proportion to the file. The crate's
//! decoder keeps a place in the file, so it is held under a lock, but only to read fields:
//! reads of pixels need nothing shared, and run in parallel (M44).

mod chunks;
mod names;

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use tiff::decoder::ifd::Value as TiffValue;
use tiff::decoder::Decoder;
use tiff::tags::Tag;
use tiff::Directory;

use self::chunks::{Coding, Samples};
use crate::fields::{Field, FieldGroup, FieldName, Origin, Value};
use crate::io::ReadAt;
use crate::types::{
    stored_level_number, Block, ByteRange, ColorModel, Fragment, Interleave, LabEncoding,
    PixelType, Representation, SampleType, Status, Storage, StorageOrder, Window,
};
pub use crate::types::{Scope, StoredLevel};

use crate::cache::{BlockKey, Cache, CacheReport};
pub use crate::error::{Error, Result};
use crate::source::OpenOptions;
use crate::warning::{WarningCode, WarningStore, Warnings};

/// The driver's name, as an `Error::Unrecognized` lists it (M51).
pub const DRIVER: &str = "geotiff";

fn unrecognized() -> Error {
    Error::Unrecognized(vec![DRIVER.into()])
}

fn corrupt(m: impl Into<String>) -> Error {
    Error::Corrupt(m.into())
}

/// A `tiff` crate error, as this reader's: its format errors are corrupt data, the rest as
/// they are.
fn from_tiff(e: tiff::TiffError) -> Error {
    match e {
        tiff::TiffError::IoError(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            corrupt("the file ends inside the data it describes")
        }
        tiff::TiffError::IoError(e) => Error::Io(e),
        tiff::TiffError::UnsupportedError(e) => Error::Unsupported(e.to_string()),
        tiff::TiffError::FormatError(e) => corrupt(e.to_string()),
        // A refusal by a limit, not damage (section 3.3).
        tiff::TiffError::LimitsExceeded => {
            Error::Unsupported("a table or chunk larger than the tiff crate's limits (M41)".into())
        }
        other => corrupt(other.to_string()),
    }
}

// ------------------------------------------------------------------ reading through ReadAt

/// A position in a `ReadAt`, for the `tiff` crate, which reads through `Read` and `Seek`.
///
/// The crate makes an image of the first IFD it is given, and refuses at that point what it
/// cannot make one of, though BIL would list it (M39, M42). So it is given a header of its own,
/// `header`, pointing at a placeholder IFD of one pixel, `extra`, served past the file's end,
/// and BIL walks the file's own chain, asking the crate only to read directories.
struct At<R: ReadAt> {
    source: Arc<R>,
    pos: u64,
    /// The file's size; `extra` follows it.
    size: u64,
    header: Vec<u8>,
    extra: Vec<u8>,
}

impl<R: ReadAt> Read for At<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (pos, header) = (self.pos, self.header.len() as u64);
        let n = if pos < header {
            let n = (header - pos).min(buf.len() as u64) as usize;
            buf[..n].copy_from_slice(&self.header[pos as usize..][..n]);
            n
        } else if pos >= self.size && pos < self.size + GAP {
            // The file's end, as the file has it.
            0
        } else if pos >= self.size {
            let from = ((pos - self.size - GAP).min(self.extra.len() as u64)) as usize;
            let n = (self.extra.len() - from).min(buf.len());
            buf[..n].copy_from_slice(&self.extra[from..][..n]);
            n
        } else {
            let n = (self.size - pos).min(buf.len() as u64) as usize;
            self.source.read_exact_at(&mut buf[..n], pos)?;
            n
        };
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: ReadAt> Seek for At<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => (self.size + GAP + self.extra.len() as u64).checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = pos.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "a seek before the start")
        })?;
        Ok(self.pos)
    }
}

// ------------------------------------------------------------------ what opening finds

/// What opening found about one Image.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageInfo {
    pub rows: u64,
    pub cols: u64,
    pub bands: u32,
    /// `None` where the sample layout is valid TIFF that BIL's types do not hold, such as
    /// 24-bit floats.
    pub pixel_type: Option<PixelType>,
    /// Why reads of the Image fail, where they do though its compression is decoded: a sample
    /// layout or predictor that cannot be read (M42).
    pub unsupported: Option<String>,
    /// The Compression tag's value.
    pub compression: u16,
    /// The codec its chunks need, by name, or `None` when they are not compressed.
    pub codec: Option<String>,
    /// Whether this build decodes its chunks: uncompressed, Deflate, LZW and PackBits.
    pub decodable: bool,
    /// Whether each band is stored apart (PlanarConfiguration 2).
    pub planar: bool,
    /// Whether chunks are tiles rather than strips.
    pub tiled: bool,
    /// Rows and columns of one chunk.
    pub block_size: (u32, u32),
    /// GDAL_NODATA, as the file states it.
    pub nodata: Option<String>,
    pub stored_levels: Vec<StoredLevel>,
    /// Where the Image's IFD lies in the chain, counted from 0.
    pub ifd: usize,
}

/// One IFD's layout, read at open.
#[derive(Clone, Debug)]
struct Layout {
    rows: u64,
    cols: u64,
    samples: u32,
    /// The type delivered, the stored bits of a sample and whether it is signed, or why there is
    /// no type.
    sample: std::result::Result<(SampleType, u32, bool), String>,
    compression: u16,
    predictor: u16,
    /// FillOrder 2.
    reversed_bits: bool,
    /// Why the coding cannot be read, where a tag says so: YCbCr subsampled outside JPEG.
    coding_problem: Option<String>,
    planar: bool,
    tiled: bool,
    chunk: (u32, u32),
    offsets: Vec<u64>,
    counts: Vec<u64>,
    nodata: Option<String>,
    subfile: u32,
    /// The SubIFDs tag's directories, where some of a file's overviews may lie.
    sub_ifds: Vec<u64>,
    /// What the samples are: Photometric, ExtraSamples, InkSet, YCbCrCoefficients, and whether
    /// SampleFormat says complex (section 3, "Representations").
    photometric: Option<u16>,
    extra_samples: Vec<u16>,
    ink_set: Option<u16>,
    ycbcr_coefficients: Option<(f64, f64, f64)>,
    complex: bool,
}

impl Layout {
    /// Chunks across and down, in one plane.
    fn chunks(&self) -> (u64, u64) {
        (
            self.cols.div_ceil(u64::from(self.chunk.1)),
            self.rows.div_ceil(u64::from(self.chunk.0)),
        )
    }

    fn planes(&self) -> u32 {
        if self.planar {
            self.samples
        } else {
            1
        }
    }
}

struct Ifd {
    dir: Directory,
    /// Where each text tag's bytes lie, and how many: read by BIL, as Latin-1.
    text: HashMap<u16, (u64, u64)>,
    layout: std::result::Result<Layout, String>,
}

/// One IFD as BIL reads its chunks: the Image's own, or a stored level's.
struct Coded {
    layout: Layout,
    pixel_type: Option<PixelType>,
    samples: Option<Samples>,
    /// Why its chunks cannot be read, where they cannot.
    unsupported: Option<String>,
}

impl Coded {
    fn of(layout: Layout, big_endian: bool) -> Coded {
        let (pixel_type, samples, unsupported) = match &layout.sample {
            Ok((t, bits, signed)) => {
                let samples = Samples {
                    sample_type: *t,
                    bits: *bits,
                    signed: *signed,
                    big_endian,
                    predictor: layout.predictor,
                    reversed_bits: layout.reversed_bits,
                };
                let problem = samples.check().err().or(layout.coding_problem.clone());
                (
                    Some(PixelType {
                        sample_type: *t,
                        component_count: layout.samples,
                        significant_bits: *bits,
                        bit_shift: 0,
                    }),
                    Some(samples),
                    problem,
                )
            }
            Err(why) => (None, None, Some(why.clone())),
        };
        Coded {
            layout,
            pixel_type,
            samples,
            unsupported,
        }
    }
}

struct Image {
    info: ImageInfo,
    full: Coded,
    /// In the order of `info.stored_levels`.
    levels: Vec<Coded>,
}

/// An open TIFF Source.
pub struct GeoTiffSource<R: ReadAt> {
    reader: Arc<R>,
    size: u64,
    big_endian: bool,
    ifds: Vec<Ifd>,
    images: Vec<Image>,
    decoder: Mutex<Decoder<At<R>>>,
    held: Mutex<HashMap<(Scope, String), Arc<FieldGroup>>>,
    /// Chunks decoded, kept for the reads that follow (section 4).
    cache: Cache,
    /// Warnings recorded, drained by `take_warnings` (section 3.3).
    warnings: Arc<WarningStore>,
}

/// The most bytes of text one tag may hold, as the `tiff` crate's limit on a tag's values (M41).
const MAX_TEXT_BYTES: u64 = 256 << 20;

/// Bytes left empty between the file's end and the placeholder IFD (`At`), so that a read past
/// the end finds the end, not the placeholder.
const GAP: u64 = 16;

fn codec(compression: u16) -> Option<String> {
    Some(
        match compression {
            1 => return None,
            8 | 32946 => "deflate",
            5 => "lzw",
            6 | 7 => "jpeg",
            32773 => "packbits",
            34712 => "jpeg2000",
            34887 => "lerc",
            50000 => "zstd",
            50001 => "webp",
            2..=4 => "fax",
            other => return Some(format!("tiff_{other}")),
        }
        .to_string(),
    )
}

/// The sample type for TIFF's SampleFormat and bits, with the stored width and whether it is
/// signed, or why there is none. Integers of other widths widen to the smallest type that holds
/// them (M43); SampleFormat 4, undefined, is read as unsigned, as GDAL reads it.
fn sample_type(format: u16, bits: u32) -> std::result::Result<(SampleType, u32, bool), String> {
    let t = match (format, bits) {
        (1 | 4, 1..=8) => SampleType::U8,
        (1 | 4, 9..=16) => SampleType::U16,
        (1 | 4, 17..=32) => SampleType::U32,
        (1 | 4, 33..=64) => SampleType::U64,
        (2, 1..=8) => SampleType::I8,
        (2, 9..=16) => SampleType::I16,
        (2, 17..=32) => SampleType::I32,
        (2, 33..=64) => SampleType::I64,
        (3, 16) => SampleType::F16,
        (3, 32) => SampleType::F32,
        (3, 64) => SampleType::F64,
        (5, 16) => SampleType::ComplexI8,
        (5, 32) => SampleType::ComplexI16,
        (6, 64) => SampleType::ComplexF32,
        (6, 128) => SampleType::ComplexF64,
        _ => return Err(format!("SampleFormat {format} with {bits} bits a sample")),
    };
    Ok((t, bits, matches!(format, 2 | 5)))
}

/// A placeholder IFD of one uncompressed pixel, in the file's byte order, for the `tiff` crate to
/// make its first image of (see `At`).
fn placeholder(big_endian: bool, bigtiff: bool) -> Vec<u8> {
    // Tag, type (3 SHORT, 4 LONG) and value.
    let entries: [(u16, u16, u32); 9] = [
        (256, 3, 1),
        (257, 3, 1),
        (258, 3, 8),
        (259, 3, 1),
        (262, 3, 1),
        (273, 4, 0),
        (277, 3, 1),
        (278, 4, 1),
        (279, 4, 1),
    ];
    let mut out = Vec::new();
    let put = |out: &mut Vec<u8>, le: &[u8]| {
        if big_endian {
            out.extend(le.iter().rev());
        } else {
            out.extend_from_slice(le);
        }
    };
    if bigtiff {
        put(&mut out, &(entries.len() as u64).to_le_bytes());
    } else {
        put(&mut out, &(entries.len() as u16).to_le_bytes());
    }
    for (tag, kind, v) in entries {
        put(&mut out, &tag.to_le_bytes());
        put(&mut out, &kind.to_le_bytes());
        if bigtiff {
            put(&mut out, &1u64.to_le_bytes());
        } else {
            put(&mut out, &1u32.to_le_bytes());
        }
        // A value is left-justified in its field.
        let start = out.len();
        if kind == 3 {
            put(&mut out, &(v as u16).to_le_bytes());
        } else {
            put(&mut out, &v.to_le_bytes());
        }
        out.resize(start + if bigtiff { 8 } else { 4 }, 0);
    }
    out.extend(std::iter::repeat_n(0, if bigtiff { 8 } else { 4 }));
    out
}

/// Where each text (ASCII) entry of the IFD at `pointer` keeps its bytes, and how many there are,
/// read from the entries themselves, since the `tiff` crate reads text only as UTF-8. The crate
/// has read this IFD, so its entries lie within the file.
fn text_entries<R: ReadAt>(
    reader: &R,
    size: u64,
    big_endian: bool,
    bigtiff: bool,
    pointer: u64,
) -> io::Result<HashMap<u16, (u64, u64)>> {
    let read = |at: u64, n: usize| -> io::Result<u64> {
        let mut b = [0u8; 8];
        reader.read_exact_at(&mut b[..n], at)?;
        Ok(if big_endian {
            b[..n].iter().fold(0, |v, &x| v << 8 | u64::from(x))
        } else {
            b[..n].iter().rev().fold(0, |v, &x| v << 8 | u64::from(x))
        })
    };
    let (count_size, entry_size, field) = if bigtiff { (8, 20, 8) } else { (2, 12, 4) };
    let count = read(pointer, count_size)?;
    let mut text = HashMap::new();
    for k in 0..count.min(size / entry_size) {
        let at = pointer + count_size as u64 + k * entry_size;
        if read(at + 2, 2)? != 2 {
            continue;
        }
        let tag = read(at, 2)? as u16;
        let n = read(at + 4, field)?;
        let value_at = if n <= field as u64 {
            at + entry_size - field as u64
        } else {
            read(at + entry_size - field as u64, field)?
        };
        if value_at.checked_add(n).is_some_and(|e| e <= size) {
            text.insert(tag, (value_at, n));
        }
    }
    Ok(text)
}

/// The stored bytes of a text tag, where the IFD has one.
fn text_bytes<R: ReadAt>(
    reader: &R,
    text: &HashMap<u16, (u64, u64)>,
    tag: u16,
) -> io::Result<Option<Vec<u8>>> {
    let Some(&(at, n)) = text.get(&tag) else {
        return Ok(None);
    };
    if n > MAX_TEXT_BYTES {
        return Err(io::Error::other(format!(
            "{n} bytes of text, more than {MAX_TEXT_BYTES} (M41)"
        )));
    }
    let mut bytes = vec![0u8; n as usize];
    reader.read_exact_at(&mut bytes, at)?;
    Ok(Some(bytes))
}

/// Text from its stored bytes, as Latin-1: one string, or a list where the tag holds several,
/// each ended by NUL.
fn latin1(bytes: &[u8]) -> Value {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let bytes = &bytes[..end];
    let mut strings: Vec<Value> = bytes
        .split(|&b| b == 0)
        .map(|s| Value::Text(s.iter().map(|&b| char::from(b)).collect()))
        .collect();
    if strings.len() == 1 {
        strings.pop().unwrap()
    } else {
        Value::List(strings)
    }
}

impl<R: ReadAt> GeoTiffSource<R> {
    /// Opens a Source with the default `OpenOptions`.
    pub fn open(reader: R) -> Result<GeoTiffSource<R>> {
        Self::open_with(reader, &OpenOptions::default())
    }

    /// Opens a Source: walks the IFD chain, reads each IFD's tags and checks that every chunk
    /// lies within the file, within `max_directories` and `max_subdirectories` (M41).
    pub fn open_with(reader: R, options: &OpenOptions) -> Result<GeoTiffSource<R>> {
        let reader = Arc::new(reader);
        let size = reader.size()?;
        let mut magic = [0u8; 4];
        if size < 8 {
            return Err(unrecognized());
        }
        reader.read_exact_at(&mut magic, 0)?;
        let (big_endian, bigtiff) = match &magic {
            b"II*\0" => (false, false),
            b"II+\0" => (false, true),
            b"MM\0*" => (true, false),
            b"MM\0+" => (true, true),
            _ => return Err(unrecognized()),
        };
        // The header, with the first IFD's place, which the `tiff` crate is not given (`At`).
        let mut header = vec![0u8; if bigtiff { 16 } else { 8 }];
        if size < header.len() as u64 {
            return Err(unrecognized());
        }
        reader.read_exact_at(&mut header, 0)?;
        let word = |b: &[u8]| -> u64 {
            if big_endian {
                b.iter().fold(0, |v, &x| v << 8 | u64::from(x))
            } else {
                b.iter().rev().fold(0, |v, &x| v << 8 | u64::from(x))
            }
        };
        if bigtiff && (word(&header[4..6]) != 8 || word(&header[6..8]) != 0) {
            return Err(unrecognized());
        }
        let place = if bigtiff { 8..16 } else { 4..8 };
        let first = word(&header[place.clone()]);
        let at_extra = if big_endian {
            (size + GAP).to_be_bytes()
        } else {
            (size + GAP).to_le_bytes()
        };
        if !bigtiff && size + GAP > u64::from(u32::MAX) {
            return Err(Error::Unsupported(
                "a classic TIFF file of more than 4 GiB".into(),
            ));
        }
        if bigtiff {
            header[place].copy_from_slice(&at_extra);
        } else if big_endian {
            header[place].copy_from_slice(&at_extra[4..]);
        } else {
            header[place].copy_from_slice(&at_extra[..4]);
        }
        let at = At {
            source: reader.clone(),
            pos: 0,
            size,
            header,
            extra: placeholder(big_endian, bigtiff),
        };
        let mut decoder = Decoder::new(at).map_err(from_tiff)?;
        if first == 0 {
            return Err(corrupt("no image file directory"));
        }
        // The chain, walked by BIL, so that every IFD is listed however the `tiff` crate would
        // judge it as an image; a chain that loops is corrupt (M39, M42).
        let mut ifds = Vec::new();
        let mut seen = HashSet::new();
        let mut next = Some(tiff::tags::IfdPointer(first));
        while let Some(pointer) = next {
            if !seen.insert(pointer.0) {
                return Err(corrupt("the chain of image file directories loops"));
            }
            if ifds.len() as u64 >= options.max_directories {
                return Err(Error::Unsupported(format!(
                    "more than {} image file directories, max_directories (M41)",
                    options.max_directories
                )));
            }
            if pointer.0 >= size {
                return Err(corrupt(format!(
                    "an IFD at {}, past the end of the file",
                    pointer.0
                )));
            }
            let dir = decoder.read_directory(pointer).map_err(from_tiff)?;
            next = dir.next();
            let text = text_entries(&*reader, size, big_endian, bigtiff, pointer.0)
                .map_err(|e| corrupt(format!("the IFD at {}: {e}", pointer.0)))?;
            let layout = read_layout(&mut decoder, &dir, &text, &*reader, size);
            ifds.push(Ifd { dir, text, layout });
        }
        let mut images: Vec<Image> = Vec::new();
        for (n, ifd) in ifds.iter().enumerate() {
            let layout = match &ifd.layout {
                Ok(l) => l.clone(),
                // An IFD whose layout cannot be read fails the open, whatever it holds: whether it
                // is a mask or an overview is known only from the layout (M42).
                Err(m) => return Err(corrupt(format!("IFD {n}: {m}"))),
            };
            let reduced = layout.subfile & 1 != 0;
            let mask = layout.subfile & 4 != 0;
            if mask {
                // A transparency mask: not read yet.
                continue;
            }
            if reduced {
                if let Some(image) = images.last_mut() {
                    // An overview with bands of its own is not a level of this Image (M49).
                    if layout.samples == image.info.bands {
                        image
                            .info
                            .stored_levels
                            .push(overview(&layout, &image.info));
                        image.levels.push(Coded::of(layout, big_endian));
                    }
                    continue;
                }
            }
            let coded = Coded::of(layout.clone(), big_endian);
            let (pixel_type, unsupported) = (coded.pixel_type, coded.unsupported.clone());
            let codec = codec(layout.compression);
            images.push(Image {
                info: ImageInfo {
                    rows: layout.rows,
                    cols: layout.cols,
                    bands: layout.samples,
                    pixel_type,
                    unsupported,
                    compression: layout.compression,
                    decodable: Coding::of(layout.compression).is_some(),
                    codec,
                    planar: layout.planar,
                    tiled: layout.tiled,
                    block_size: layout.chunk,
                    nodata: layout.nodata.clone(),
                    stored_levels: Vec::new(),
                    ifd: n,
                },
                full: coded,
                levels: Vec::new(),
            });
        }
        if images.is_empty() {
            return Err(corrupt("no full-resolution image"));
        }
        // Overviews in an Image's SubIFDs are stored levels too, as GDAL reads them.
        let warnings = Arc::new(WarningStore::new(options.max_held_warnings));
        for (index, image) in images.iter_mut().enumerate() {
            let pointers = ifds[image.info.ifd]
                .layout
                .as_ref()
                .map(|l| l.sub_ifds.clone())
                .unwrap_or_default();
            if pointers.len() as u64 > options.max_subdirectories {
                return Err(Error::Unsupported(format!(
                    "image {} has more than {} SubIFDs, max_subdirectories (M41)",
                    image.info.ifd, options.max_subdirectories
                )));
            }
            // A SubIFD that cannot be read is skipped, with a warning: it is not in the chain
            // that locates the Images, and holds at most an overview (M42, section 3.3).
            let skipped = |p: u64| {
                warnings.record(
                    WarningCode::SubdirectorySkipped,
                    Scope::Image(index),
                    Some(p),
                    None,
                )
            };
            for p in pointers {
                if !seen.insert(p) {
                    continue;
                }
                if p == 0 || p >= size {
                    skipped(p);
                    continue;
                }
                let Ok(dir) = decoder.read_directory(tiff::tags::IfdPointer(p)) else {
                    skipped(p);
                    continue;
                };
                let Ok(text) = text_entries(&*reader, size, big_endian, bigtiff, p) else {
                    skipped(p);
                    continue;
                };
                let Ok(layout) = read_layout(&mut decoder, &dir, &text, &*reader, size) else {
                    skipped(p);
                    continue;
                };
                if layout.subfile & 1 != 0
                    && layout.subfile & 4 == 0
                    && layout.samples == image.info.bands
                {
                    image
                        .info
                        .stored_levels
                        .push(overview(&layout, &image.info));
                    image.levels.push(Coded::of(layout, big_endian));
                }
            }
        }
        for image in &mut images {
            // Finest first, the level and its chunks together.
            let mut both: Vec<(StoredLevel, Coded)> = std::mem::take(&mut image.info.stored_levels)
                .into_iter()
                .zip(std::mem::take(&mut image.levels))
                .collect();
            both.sort_by_key(|(l, _)| std::cmp::Reverse(u128::from(l.rows) * u128::from(l.cols)));
            for (k, (mut level, coded)) in both.into_iter().enumerate() {
                level.name = format!("stored{}", k + 1);
                image.info.stored_levels.push(level);
                image.levels.push(coded);
            }
        }
        Ok(GeoTiffSource {
            reader,
            size,
            big_endian,
            ifds,
            images,
            decoder: Mutex::new(decoder),
            held: Mutex::new(HashMap::new()),
            cache: Cache::for_source(warnings.clone()),
            warnings,
        })
    }

    /// The bytes the Source reads from.
    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// The file's size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn image_count(&self) -> usize {
        self.images.len()
    }

    pub fn image_info(&self, image: usize) -> Result<&ImageInfo> {
        Ok(&self.image(image)?.info)
    }

    fn image(&self, image: usize) -> Result<&Image> {
        self.images
            .get(image)
            .ok_or_else(|| Error::NotFound(format!("image {image}")))
    }

    // -------------------------------------------------------------- fields

    /// The names of the field groups attached to `scope`: for an Image, its IFD's tags, and its
    /// GeoTIFF keys where it has them. A TIFF file has nothing above its IFDs, so the Source has
    /// none. Extracts nothing.
    pub fn list_field_groups(&self, scope: Scope) -> Result<Vec<String>> {
        let Scope::Image(i) = scope else {
            return Ok(Vec::new());
        };
        let ifd = &self.ifds[self.image(i)?.info.ifd];
        let mut groups = vec!["tiff.ifd".to_string()];
        if ifd.dir.get(Tag::GeoKeyDirectoryTag).is_some() {
            groups.push("geotiff.keys".to_string());
        }
        Ok(groups)
    }

    /// The held groups. A panic while they were locked leaves them whole, since each is
    /// inserted or removed in one step, so a poisoned lock is taken as it is.
    fn held(&self) -> std::sync::MutexGuard<'_, HashMap<(Scope, String), Arc<FieldGroup>>> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The decoder, which reads fields. Each read seeks to its directory first, so a panic
    /// while it was locked leaves nothing to undo.
    fn decoder(&self) -> std::sync::MutexGuard<'_, Decoder<At<R>>> {
        self.decoder
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A field group, extracted on first request and held until released.
    pub fn read_field_group(&self, scope: Scope, group: &str) -> Result<Arc<FieldGroup>> {
        let key = (scope, group.to_string());
        if let Some(g) = self.held().get(&key) {
            return Ok(g.clone());
        }
        let g = Arc::new(self.extract(scope, group)?);
        self.held().entry(key).or_insert_with(|| g.clone());
        Ok(g)
    }

    /// One field, by its dotted name, from the group its first two parts name.
    pub fn read_field(&self, scope: Scope, name: &str) -> Result<Field> {
        let parsed = FieldName::parse(name);
        if parsed.0.len() != 3 {
            return Err(Error::NotFound(format!("field {name}")));
        }
        let group = self.read_field_group(scope, &FieldName(parsed.0[..2].to_vec()).dotted())?;
        group
            .fields
            .iter()
            .find(|f| f.name == parsed)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("field {name}")))
    }

    /// Frees a held group; it is extracted again if asked for.
    pub fn release_metadata(&self, scope: Scope, group: &str) {
        self.held().remove(&(scope, group.to_string()));
    }

    fn extract(&self, scope: Scope, group: &str) -> Result<FieldGroup> {
        let not_found = || Error::NotFound(format!("group {group}"));
        let Scope::Image(i) = scope else {
            return Err(not_found());
        };
        let ifd = &self.ifds[self.image(i)?.info.ifd];
        let name = FieldName::parse(group);
        let mut decoder = self.decoder();
        let decoder = &mut *decoder;
        let mut problem = None;
        let mut fields = Vec::new();
        match group {
            "tiff.ifd" => {
                for (tag, _) in ifd.dir.iter() {
                    let code = tag.to_u16();
                    let part = names::tag_name(code)
                        .map(str::to_string)
                        .unwrap_or_else(|| code.to_string());
                    if ifd.text.contains_key(&code) {
                        // Text is read from its own bytes, every string kept (M36, M39).
                        match text_bytes(&*self.reader, &ifd.text, code)
                            .map(Option::unwrap_or_default)
                        {
                            Ok(bytes) => fields.push(Field::with_value(
                                name.child(part),
                                bytes.clone(),
                                latin1(&bytes),
                                Origin::Specified,
                            )),
                            Err(e) => {
                                problem.get_or_insert(format!("tag {code}: {e}"));
                            }
                        }
                        continue;
                    }
                    match decoder.read_directory_tags(&ifd.dir).find_tag(tag) {
                        Ok(Some(v)) => fields.push(Field::with_value(
                            name.child(part),
                            stored_bytes(&v, self.big_endian),
                            value(v),
                            Origin::Specified,
                        )),
                        Ok(None) => {}
                        Err(e) => {
                            problem.get_or_insert(format!("tag {code}: {e}"));
                        }
                    }
                }
            }
            "geotiff.keys" => {
                let ascii = text_bytes(&*self.reader, &ifd.text, 34737)
                    .map_err(|e| match e.kind() {
                        io::ErrorKind::Other => {
                            Error::Unsupported(format!("GeoAsciiParamsTag: {e}"))
                        }
                        _ => corrupt(format!("GeoAsciiParamsTag: {e}")),
                    })?
                    .unwrap_or_default();
                let keys = read_keys(decoder, &ifd.dir, &ascii, self.big_endian)
                    .map_err(|m| corrupt(format!("GeoTIFF keys: {m}")))?;
                // A key listed more than once carries its place among them (section 14.1).
                let mut totals: HashMap<u16, usize> = HashMap::new();
                for (code, _, _) in &keys {
                    *totals.entry(*code).or_insert(0) += 1;
                }
                let mut seen: HashMap<u16, usize> = HashMap::new();
                for (code, v, stored) in keys {
                    let mut part = names::key_name(code)
                        .map(str::to_string)
                        .unwrap_or_else(|| code.to_string());
                    let n = seen.entry(code).or_insert(0);
                    *n += 1;
                    if totals[&code] > 1 {
                        part = format!("{part}[{n}]");
                    }
                    fields.push(Field::with_value(
                        name.child(part),
                        stored,
                        v,
                        Origin::Specified,
                    ));
                }
            }
            _ => return Err(not_found()),
        }
        // The decoder was moved to another directory's tags; its loaded image is unchanged.
        Ok(FieldGroup {
            name,
            fields,
            status: if problem.is_some() {
                Status::Partial
            } else {
                Status::Complete
            },
            problem,
        })
    }

    // -------------------------------------------------------------- pixels

    /// The Image described as every driver describes one (section 3, `ImageInfo`). It can have
    /// missing pixels where it or a stored level states a no-data value or has a chunk never
    /// written, and where its samples are floats, whose stored NaN is missing (M52).
    pub fn info(&self, image: usize) -> Result<crate::types::ImageInfo> {
        let img = self.image(image)?;
        let (representation, component_names) = representation(&img.full.layout);
        let mut storage = self.describe_storage(image)?;
        storage.blocks = Vec::new();
        let gaps = std::iter::once(&img.full)
            .chain(&img.levels)
            .any(|c| c.layout.nodata.is_some() || c.layout.counts.contains(&0));
        Ok(crate::types::ImageInfo {
            row_count: img.info.rows,
            col_count: img.info.cols,
            frame_count: crate::types::FrameCount::Known(1),
            representation,
            component_names,
            pixel_type: img.info.pixel_type,
            component_count: img.info.bands,
            relations: Vec::new(),
            storage,
            may_have_missing: gaps
                || img
                    .info
                    .pixel_type
                    .is_some_and(|p| p.sample_type.is_float()),
        })
    }

    /// The warnings recorded since the last call, and how many were dropped past
    /// `max_held_warnings`; empties the store (section 3.3).
    pub fn take_warnings(&self) -> Warnings {
        self.warnings.take()
    }

    /// Sets how many decoded chunks this Source keeps; 0 turns the cache off (section 4).
    pub fn set_cache_size(&self, fragment_count: usize) {
        self.cache.set_size(fragment_count);
    }

    /// Sets the most bytes of decoded chunks this Source keeps (section 4).
    pub fn set_cache_bytes(&self, byte_count: u64) {
        self.cache.set_bytes(byte_count);
    }

    /// What the cache holds and has served (section 4).
    pub fn describe_cache(&self) -> CacheReport {
        self.cache.describe()
    }

    /// Frees every cached chunk (section 4).
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// An Image's stored levels, finest first: its overviews and SubIFDs (section 5,
    /// `list_stored_levels`).
    pub fn list_stored_levels(&self, image: usize) -> Result<&[StoredLevel]> {
        Ok(&self.image(image)?.info.stored_levels)
    }

    /// How the Image's chunks lie in the file (section 4, `describe_storage`).
    pub fn describe_storage(&self, image: usize) -> Result<Storage> {
        self.describe_level_storage(image, "level0")
    }

    /// How a level's chunks lie in the file: `level0`'s, or a stored level's, each block
    /// marked with its level (section 10, "Describe storage").
    pub fn describe_level_storage(&self, image: usize, level: &str) -> Result<Storage> {
        let img = self.image(image)?;
        let n = stored_level_number(level)
            .filter(|&n| n <= img.levels.len())
            .ok_or_else(|| Error::NotFound(format!("level {level} of image {image}")))?;
        let l = match n {
            0 => &img.full.layout,
            n => &img.levels[n - 1].layout,
        };
        let (across, _) = l.chunks();
        let per_plane = l.offsets.len() as u64 / u64::from(l.planes());
        let mut blocks: Vec<Block> = l
            .offsets
            .iter()
            .zip(&l.counts)
            .enumerate()
            .filter(|&(_, (_, &count))| count > 0)
            .map(|(k, (&offset, &length))| {
                let k = k as u64;
                let (plane, within) = (k / per_plane, k % per_plane);
                let components = if l.planar {
                    plane as u32..plane as u32 + 1
                } else {
                    0..l.samples
                };
                Block {
                    image: image as u32,
                    level: n as u32,
                    frame: 0,
                    block_row: (within / across) as u32,
                    block_col: (within % across) as u32,
                    components,
                    ranges: vec![ByteRange { offset, length }],
                }
            })
            .collect();
        blocks.sort_by_key(Block::offset);
        Ok(Storage {
            block_size: l.chunk,
            interleave: if l.planar {
                Interleave::Band
            } else {
                Interleave::Pixel
            },
            order: StorageOrder::Known,
            blocks,
        })
    }

    /// The native pixels of a Window at full resolution, pixel-interleaved, in native byte
    /// order and as stored, with pixels holding the no-data value in every sample marked missing.
    /// Reads chunks of its own, so reads of one Source run in parallel (M44).
    pub fn read_fragment(&self, image: usize, window: Window) -> Result<Fragment> {
        let img = self.image(image)?;
        self.read_coded(image, 0, &img.full, img.info.nodata.as_deref(), window)
    }

    /// The native pixels of a Window at a level: `level0`, full resolution, or a stored level
    /// such as `stored2` (section 5, "Read directly"). The Window is in full-resolution rows and
    /// columns; a stored level gives the pixels whose centers fall inside it, and the Fragment's
    /// own Window is in the stored level's rows and columns (M46). A stored level without a
    /// no-data value of its own takes its Image's (M49).
    pub fn read_level_fragment(
        &self,
        image: usize,
        level: &str,
        window: Window,
    ) -> Result<Fragment> {
        let img = self.image(image)?;
        let n = stored_level_number(level)
            .filter(|&n| n <= img.levels.len())
            .ok_or_else(|| Error::NotFound(format!("level {level} of image {image}")))?;
        if n == 0 {
            return self.read_fragment(image, window);
        }
        check_window(image, img.info.rows, img.info.cols, window)?;
        self.read_stored_window(image, level, img.info.stored_levels[n - 1].covered(window))
    }

    /// As `read_level_fragment` for a stored level, with the Window in the level's own rows and
    /// columns, as `read_slice`'s pass reads it (M77).
    pub(crate) fn read_stored_window(
        &self,
        image: usize,
        level: &str,
        window: Window,
    ) -> Result<Fragment> {
        let img = self.image(image)?;
        let n = stored_level_number(level)
            .filter(|&n| n >= 1 && n <= img.levels.len())
            .ok_or_else(|| Error::NotFound(format!("level {level} of image {image}")))?;
        let coded = &img.levels[n - 1];
        let nodata = coded
            .layout
            .nodata
            .as_deref()
            .or(img.info.nodata.as_deref());
        self.read_coded(image, n as u32, coded, nodata, window)
    }

    /// One chunk's rows, `rows` of them from its first, decompressed and turned into native
    /// samples, row after row.
    fn decode_chunk(
        &self,
        coding: Coding,
        samples: &Samples,
        index: usize,
        (offset, count): (u64, u64),
        (row_bytes, rows): (usize, usize),
        (values, in_chunk): (usize, usize),
    ) -> Result<Vec<u8>> {
        // A declared size the stored bytes cannot make is corrupt, and is refused before any
        // memory is taken for it.
        let claimed = (row_bytes as u64).saturating_mul(rows as u64);
        if claimed > count.saturating_mul(coding.most_per_byte()) {
            return Err(corrupt(format!(
                "chunk {index}: {count} bytes cannot hold the {claimed} bytes its rows need"
            )));
        }
        let needed = usize::try_from(claimed)
            .ok()
            .filter(|&n| n <= isize::MAX as usize)
            .ok_or_else(|| Error::Unsupported("a chunk too large to hold".into()))?;
        // Offsets and counts were checked against the file at open.
        let mut stored = vec![0u8; count as usize];
        self.reader.read_exact_at(&mut stored, offset)?;
        if coding == Coding::Lzw && stored.starts_with(&[0, 1]) {
            // LZW as written before TIFF 5.0, with its bits reversed: libtiff reads it, and BIL
            // does not yet.
            return Err(Error::Unsupported(format!(
                "chunk {index}: LZW in the form before TIFF 5.0"
            )));
        }
        let mut decoded = vec![0u8; needed];
        chunks::decompress(coding, &mut stored, samples.reversed_bits, &mut decoded)
            .map_err(|m| corrupt(format!("chunk {index}: {m}")))?;
        let mut native = Vec::new();
        for k in 0..rows {
            chunks::row(
                samples,
                &mut decoded[k * row_bytes..][..row_bytes],
                values,
                in_chunk,
                &mut native,
            );
        }
        Ok(native)
    }

    fn read_coded(
        &self,
        image: usize,
        level: u32,
        coded: &Coded,
        nodata: Option<&str>,
        window: Window,
    ) -> Result<Fragment> {
        let l = &coded.layout;
        let Some(coding) = Coding::of(l.compression) else {
            return Err(Error::Unsupported(format!(
                "image {image} is compressed with {}, which this build does not decode",
                codec(l.compression).unwrap_or_default()
            )));
        };
        if let Some(why) = &coded.unsupported {
            return Err(Error::Unsupported(why.clone()));
        }
        let (Some(pixel_type), Some(samples)) = (coded.pixel_type, coded.samples) else {
            return Err(Error::Unsupported(format!("image {image}")));
        };
        let (rows, cols) = (u64::from(window.row_count), u64::from(window.col_count));
        check_window(image, l.rows, l.cols, window)?;
        let size = pixel_type.sample_type.size();
        let bands = l.samples as usize;
        let total = rows
            .checked_mul(cols)
            .and_then(|p| p.checked_mul(bands as u64 * size as u64))
            .filter(|&n| n <= isize::MAX as u64)
            .ok_or_else(|| Error::Unsupported("a Window too large to hold".into()))?;
        let strides = [cols as usize * bands * size, bands * size, size];
        let mut data = vec![0u8; total as usize];
        let (ch, cw) = (u64::from(l.chunk.0), u64::from(l.chunk.1));
        let (across, down) = l.chunks();
        let row_end = window.row_start + rows;
        let col_end = window.col_start + cols;
        let in_chunk = if l.planar { 1 } else { bands };
        // Pixels in chunks never written (section 4): 0 in the data, and marked.
        let mut unrecorded = vec![false; (rows * cols) as usize];
        for crow in window.row_start / ch..row_end.div_ceil(ch).min(down) {
            // Strips end at the Image. A tile's rows below it are padding, which some writers
            // leave out, so they are neither decoded nor kept.
            let h = ch.min(l.rows - crow * ch);
            let needed_rows = h.min(row_end - crow * ch);
            for ccol in window.col_start / cw..col_end.div_ceil(cw).min(across) {
                let r0 = (crow * ch).max(window.row_start);
                let r1 = (crow * ch + h).min(row_end);
                let c0 = (ccol * cw).max(window.col_start);
                let c1 = (ccol * cw + cw).min(col_end);
                for plane in 0..l.planes() {
                    // Within the Image's chunks, by the loops' bounds.
                    let index = (u64::from(plane) * across * down + crow * across + ccol) as usize;
                    let (offset, count) = (l.offsets[index], l.counts[index]);
                    if count == 0 {
                        // A chunk of no bytes was never written, as in a sparse file: its pixels
                        // were never recorded.
                        for r in r0..r1 {
                            let at =
                                ((r - window.row_start) * cols + c0 - window.col_start) as usize;
                            unrecorded[at..at + (c1 - c0) as usize].fill(true);
                        }
                        continue;
                    }
                    let values = cw * in_chunk as u64;
                    let row_bytes = samples
                        .row_bytes(values)
                        .and_then(|b| usize::try_from(b).ok())
                        .ok_or_else(|| Error::Unsupported("a chunk too large to hold".into()))?;
                    // The whole chunk is decoded and kept (section 4, "One block, one entry"),
                    // unless the cache is off, or the chunk would pass the ceiling and the read
                    // needs fewer of its rows: then only the rows down to the Window's end are
                    // decoded, and nothing is kept (M67).
                    let whole_bytes = (cw * in_chunk as u64 * size as u64).saturating_mul(h);
                    let key = BlockKey {
                        image: image as u32,
                        level,
                        frame: 0,
                        block_row: crow as u32,
                        block_col: ccol as u32,
                        components: if l.planar {
                            (plane, plane + 1)
                        } else {
                            (0, l.samples)
                        },
                    };
                    let native = self
                        .cache
                        .read(key, whole_bytes, needed_rows < h, |whole| {
                            self.decode_chunk(
                                coding,
                                &samples,
                                index,
                                (offset, count),
                                (row_bytes, if whole { h } else { needed_rows } as usize),
                                (values as usize, in_chunk),
                            )
                        })?;
                    let native_row = cw as usize * in_chunk * size;
                    for r in r0..r1 {
                        let k = (r - crow * ch) as usize;
                        let native = &native[k * native_row..][..native_row];
                        let dst_row = (r - window.row_start) as usize * strides[0];
                        if l.planar {
                            for c in c0..c1 {
                                let s = (c - ccol * cw) as usize * size;
                                let d = dst_row
                                    + (c - window.col_start) as usize * strides[1]
                                    + plane as usize * size;
                                data[d..d + size].copy_from_slice(&native[s..s + size]);
                            }
                        } else {
                            let s = (c0 - ccol * cw) as usize * strides[1];
                            let d = dst_row + (c0 - window.col_start) as usize * strides[1];
                            let n = (c1 - c0) as usize * strides[1];
                            data[d..d + n].copy_from_slice(&native[s..s + n]);
                        }
                    }
                }
            }
        }
        let fill = nodata.and_then(|text| fill(text, pixel_type.sample_type));
        let valid: Vec<bool> = data
            .chunks_exact(strides[1])
            .zip(&unrecorded)
            .map(|(pixel, &never)| {
                !never
                    && !fill
                        .as_ref()
                        .is_some_and(|f| pixel.chunks_exact(size).all(|s| f.matches(s)))
            })
            .collect();
        let validity = valid.contains(&false).then_some(valid);
        Ok(Fragment {
            window,
            pixel_type,
            strides,
            data,
            validity,
        })
    }
}

/// A TIFF overview as a stored level, its name given once the levels are sorted. TIFF does not
/// say how an overview was made, so its alignment is unknown; it is taken to cover its Image's
/// extent, as GDAL takes it (M38, M47).
fn overview(layout: &Layout, image: &ImageInfo) -> StoredLevel {
    StoredLevel::covering_extent(layout.rows, layout.cols, image.rows, image.cols)
}

/// A Window within an Image or level of `rows` × `cols` pixels, or an error.
fn check_window(image: usize, rows: u64, cols: u64, window: Window) -> Result<()> {
    if window
        .row_start
        .checked_add(u64::from(window.row_count))
        .is_none_or(|e| e > rows)
        || window
            .col_start
            .checked_add(u64::from(window.col_count))
            .is_none_or(|e| e > cols)
    {
        return Err(Error::NotFound(format!(
            "a Window outside image {image}'s {rows} × {cols} pixels"
        )));
    }
    Ok(())
}

/// Reads one IFD's layout, checking every offset against the file.
fn read_layout<R: ReadAt>(
    decoder: &mut Decoder<At<R>>,
    dir: &Directory,
    text: &HashMap<u16, (u64, u64)>,
    reader: &R,
    size: u64,
) -> std::result::Result<Layout, String> {
    let mut t = decoder.read_directory_tags(dir);
    let mut one = |tag: Tag| -> std::result::Result<Option<u64>, String> {
        t.find_tag_unsigned::<u64>(tag)
            .map_err(|e| format!("{tag:?}: {e}"))
    };
    let cols = one(Tag::ImageWidth)?.ok_or("no ImageWidth")?;
    let rows = one(Tag::ImageLength)?.ok_or("no ImageLength")?;
    let samples = one(Tag::SamplesPerPixel)?.unwrap_or(1);
    let compression = one(Tag::Compression)?.unwrap_or(1);
    let predictor = one(Tag::Predictor)?.unwrap_or(1);
    let fill_order = one(Tag::FillOrder)?.unwrap_or(1);
    let planar = one(Tag::PlanarConfiguration)?.unwrap_or(1) == 2;
    let subfile = one(Tag::NewSubfileType)?.unwrap_or(0)
        | if one(Tag::SubfileType)? == Some(2) {
            1
        } else {
            0
        };
    let tile = (one(Tag::TileLength)?, one(Tag::TileWidth)?);
    let mut t = decoder.read_directory_tags(dir);
    let mut many = |tag: Tag| -> std::result::Result<Option<Vec<u64>>, String> {
        t.find_tag_unsigned_vec::<u64>(tag)
            .map_err(|e| format!("{tag:?}: {e}"))
    };
    let bits = many(Tag::BitsPerSample)?.unwrap_or_else(|| vec![1]);
    let formats = many(Tag::SampleFormat)?.unwrap_or_else(|| vec![1]);
    if rows == 0 || cols == 0 || samples == 0 || samples > 65_535 {
        return Err(format!("{cols} × {rows} pixels of {samples} samples"));
    }
    let (tiled, chunk) = match tile {
        (Some(h), Some(w)) => (true, (h, w)),
        (None, None) => {
            let mut t = decoder.read_directory_tags(dir);
            let per = t
                .find_tag_unsigned::<u64>(Tag::RowsPerStrip)
                .map_err(|e| format!("RowsPerStrip: {e}"))?
                .unwrap_or(rows)
                .min(rows);
            (false, (per, cols))
        }
        _ => return Err("TileWidth without TileLength, or the reverse".into()),
    };
    if chunk.0 == 0
        || chunk.1 == 0
        || chunk.0 > u64::from(u32::MAX)
        || chunk.1 > u64::from(u32::MAX)
    {
        return Err(format!("chunks of {} × {} pixels", chunk.1, chunk.0));
    }
    let mut t = decoder.read_directory_tags(dir);
    let (offsets, counts) = if tiled {
        (
            t.find_tag_unsigned_vec::<u64>(Tag::TileOffsets),
            t.find_tag_unsigned_vec::<u64>(Tag::TileByteCounts),
        )
    } else {
        (
            t.find_tag_unsigned_vec::<u64>(Tag::StripOffsets),
            t.find_tag_unsigned_vec::<u64>(Tag::StripByteCounts),
        )
    };
    let offsets = offsets
        .map_err(|e| e.to_string())?
        .ok_or("no chunk offsets")?;
    let counts = counts
        .map_err(|e| e.to_string())?
        .ok_or("no chunk byte counts")?;
    let planes = if planar { samples } else { 1 };
    let expected = rows
        .div_ceil(chunk.0)
        .checked_mul(cols.div_ceil(chunk.1))
        .and_then(|n| n.checked_mul(planes))
        .ok_or("more chunks than can be counted")?;
    if offsets.len() as u64 != expected || counts.len() as u64 != expected {
        return Err(format!(
            "{} chunk offsets and {} byte counts, for {expected} chunks",
            offsets.len(),
            counts.len()
        ));
    }
    for (&o, &c) in offsets.iter().zip(&counts) {
        if o.checked_add(c).is_none_or(|e| e > size) {
            return Err(format!(
                "a chunk at {o} of {c} bytes runs past the end of the file"
            ));
        }
    }
    let bits0 = *bits.first().ok_or("BitsPerSample with no values")?;
    let format0 = *formats.first().ok_or("SampleFormat with no values")?;
    let bits0 = u32::try_from(bits0).map_err(|_| format!("{bits0} bits a sample"))?;
    let format0 = u16::try_from(format0).map_err(|_| format!("SampleFormat {format0}"))?;
    let sample = if bits.iter().any(|&b| b != u64::from(bits0))
        || formats.iter().any(|&f| f != u64::from(format0))
    {
        Err("samples of different bits or formats in one pixel".to_string())
    } else {
        sample_type(format0, bits0)
    };
    let mut t = decoder.read_directory_tags(dir);
    let photometric = t
        .find_tag_unsigned::<u16>(Tag::PhotometricInterpretation)
        .ok()
        .flatten();
    // YCbCr data outside JPEG is stored in units of subsampled pixels, 2 × 2 unless a tag says
    // otherwise (TIFF 6.0, section 21); BIL does not regroup them.
    let subsampling = t
        .find_tag_unsigned_vec::<u16>(Tag::ChromaSubsampling)
        .ok()
        .flatten()
        .unwrap_or_else(|| vec![2, 2]);
    let extra_samples = t
        .find_tag_unsigned_vec::<u16>(Tag::ExtraSamples)
        .ok()
        .flatten()
        .unwrap_or_default();
    let ink_set = t
        .find_tag_unsigned::<u16>(Tag::from_u16_exhaustive(332))
        .ok()
        .flatten();
    // YCbCrCoefficients: three rationals, the luma weights of red, green and blue.
    let ycbcr_coefficients = match t.find_tag(Tag::from_u16_exhaustive(529)) {
        Ok(Some(TiffValue::List(v))) if v.len() == 3 => {
            let r = |v: &TiffValue| match *v {
                TiffValue::Rational(n, d) if d != 0 => Some(f64::from(n) / f64::from(d)),
                _ => None,
            };
            match (r(&v[0]), r(&v[1]), r(&v[2])) {
                (Some(a), Some(b), Some(c)) => Some((a, b, c)),
                _ => None,
            }
        }
        _ => None,
    };
    let coding_problem = (photometric == Some(6)
        && !matches!(compression, 6 | 7)
        && subsampling.iter().any(|&f| f != 1))
    .then(|| "YCbCr samples subsampled outside JPEG".to_string());
    let sub_ifds = match t.find_tag(Tag::SubIfd) {
        Ok(Some(v)) => {
            let one = |v: &TiffValue| match v {
                TiffValue::Ifd(x) | TiffValue::Unsigned(x) => Some(u64::from(*x)),
                TiffValue::IfdBig(x) | TiffValue::UnsignedBig(x) => Some(*x),
                _ => None,
            };
            match &v {
                TiffValue::List(items) => items.iter().filter_map(one).collect(),
                single => one(single).into_iter().collect(),
            }
        }
        _ => Vec::new(),
    };
    let nodata = text_bytes(reader, text, Tag::GdalNodata.to_u16())
        .ok()
        .flatten()
        .and_then(|b| match latin1(&b) {
            Value::Text(s) => Some(s.trim().to_string()),
            _ => None,
        });
    Ok(Layout {
        rows,
        cols,
        samples: samples as u32,
        sample,
        compression: u16::try_from(compression).map_err(|_| "a Compression past 65,535")?,
        predictor: u16::try_from(predictor).unwrap_or(u16::MAX),
        reversed_bits: fill_order == 2,
        coding_problem,
        planar,
        tiled,
        chunk: (chunk.0 as u32, chunk.1 as u32),
        offsets,
        counts,
        nodata,
        subfile: subfile as u32,
        sub_ifds,
        photometric,
        extra_samples,
        ink_set,
        ycbcr_coefficients,
        complex: formats.iter().any(|&f| f == 5 || f == 6),
    })
}

/// An Image's representation and component names from its Photometric and the tags beside it
/// (section 3, "Representations").
fn representation(l: &Layout) -> (Representation, Vec<String>) {
    let n = l.samples as usize;
    let extras = l.extra_samples.len().min(n);
    let base = n - extras;
    let named = |names: &[&'static str]| -> Vec<Option<&'static str>> {
        (0..base).map(|i| names.get(i).copied()).collect()
    };
    let (r, mut names): (Representation, Vec<Option<&'static str>>) = if l.complex {
        (Representation::Complex, vec![Some("complex"); base])
    } else {
        match l.photometric {
            // More than one sample beside any extra ones: spectral (section 3's table).
            Some(0) | Some(1) | None if base > 1 => (Representation::Spectral, vec![None; base]),
            p @ (Some(0) | Some(1) | None) => (
                Representation::Intensity {
                    white_is_zero: p == Some(0),
                },
                named(&["intensity"]),
            ),
            Some(2) => (
                Representation::Color(ColorModel::Rgb),
                named(&["red", "green", "blue"]),
            ),
            Some(3) => (Representation::Palette, named(&["index"])),
            // InkSet 1, CMYK, is TIFF's default.
            Some(5) if l.ink_set.unwrap_or(1) == 1 && base == 4 => (
                Representation::Color(ColorModel::Cmyk),
                named(&["cyan", "magenta", "yellow", "black"]),
            ),
            Some(6) => (
                Representation::Color(ColorModel::YCbCr {
                    coefficients: l.ycbcr_coefficients.unwrap_or(crate::types::BT601),
                }),
                named(&["luma", "chroma_blue", "chroma_red"]),
            ),
            Some(p @ 8..=10) => (
                Representation::Color(ColorModel::Lab {
                    encoding: match p {
                        8 => LabEncoding::Cie,
                        9 => LabEncoding::Icc,
                        _ => LabEncoding::Itu,
                    },
                }),
                named(&["lightness", "green_red", "blue_yellow"]),
            ),
            _ => (Representation::Vector, vec![None; base]),
        }
    };
    names.extend(
        l.extra_samples[..extras]
            .iter()
            .map(|&e| matches!(e, 1 | 2).then_some("alpha")),
    );
    (r, crate::types::component_names(names))
}

/// A no-data value in a sample type's own terms.
enum Fill {
    Int { value: i128, signed: bool },
    Float(f64),
    NaN,
}

impl Fill {
    /// Whether a sample, in native byte order, holds the value.
    fn matches(&self, s: &[u8]) -> bool {
        let int = |signed: bool| -> i128 {
            let mut b = [0u8; 16];
            let n = s.len();
            if cfg!(target_endian = "little") {
                b[..n].copy_from_slice(s);
                if signed && s[n - 1] & 0x80 != 0 {
                    b[n..].fill(0xFF);
                }
                i128::from_le_bytes(b)
            } else {
                b[16 - n..].copy_from_slice(s);
                if signed && s[0] & 0x80 != 0 {
                    b[..16 - n].fill(0xFF);
                }
                i128::from_be_bytes(b)
            }
        };
        let float = || -> f64 {
            match s.len() {
                2 => {
                    let bits = u16::from_ne_bytes([s[0], s[1]]);
                    f64::from(half_to_f32(bits))
                }
                4 => f64::from(f32::from_ne_bytes(s.try_into().unwrap())),
                _ => f64::from_ne_bytes(s.try_into().unwrap()),
            }
        };
        match self {
            Fill::Int { value, signed } => int(*signed) == *value,
            Fill::Float(v) => float() == *v,
            Fill::NaN => float().is_nan(),
        }
    }
}

/// A half-precision float's value (IEEE 754 binary16), exactly.
fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((h >> 10) & 0x1F);
    let fraction = f32::from(h & 0x3FF);
    match exponent {
        0 => sign * fraction * 2f32.powi(-24),
        31 if fraction == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        e => sign * (1.0 + fraction / 1024.0) * 2f32.powi(e - 15),
    }
}

/// A value rounded to the nearest float16, ties to even, or `None` beyond float16's range.
fn round_to_half(v: f64) -> Option<f64> {
    if !v.is_finite() {
        return Some(v);
    }
    let a = v.abs();
    // Below 2^-14 the spacing is that of the subnormals, 2^-24.
    let exponent = if a < 2f64.powi(-14) {
        -14
    } else {
        ((a.to_bits() >> 52) & 0x7FF) as i32 - 1023
    };
    let quantum = 2f64.powi(exponent - 10);
    let r = (a / quantum).round_ties_even() * quantum;
    (r <= 65_504.0).then_some(r.copysign(v))
}

/// The no-data value for a sample type, where the text is one it can hold.
fn fill(text: &str, t: SampleType) -> Option<Fill> {
    let float = matches!(t, SampleType::F16 | SampleType::F32 | SampleType::F64);
    if float {
        if text.eq_ignore_ascii_case("nan") {
            return Some(Fill::NaN);
        }
        let v: f64 = text.parse().ok()?;
        // Rounded to float32, as GDAL does, and to float16 likewise; a value beyond the type's
        // range marks nothing (M37).
        return match t {
            SampleType::F32 if v.is_finite() && (v as f32).is_infinite() => None,
            SampleType::F32 => Some(Fill::Float(f64::from(v as f32))),
            SampleType::F16 => round_to_half(v).map(Fill::Float),
            _ => Some(Fill::Float(v)),
        };
    }
    let v: i128 = match text.parse::<i128>() {
        Ok(v) => v,
        Err(_) => {
            let f: f64 = text.parse().ok()?;
            if f.fract() != 0.0 || !f.is_finite() {
                return None;
            }
            f as i128
        }
    };
    let (lo, hi): (i128, i128) = match t {
        SampleType::U8 => (0, 255),
        SampleType::I8 => (-128, 127),
        SampleType::U16 => (0, 65_535),
        SampleType::I16 => (-32_768, 32_767),
        SampleType::U32 => (0, u32::MAX.into()),
        SampleType::I32 => (i32::MIN.into(), i32::MAX.into()),
        SampleType::U64 => (0, u64::MAX.into()),
        SampleType::I64 => (i64::MIN.into(), i64::MAX.into()),
        _ => return None,
    };
    let signed = matches!(
        t,
        SampleType::I8 | SampleType::I16 | SampleType::I32 | SampleType::I64
    );
    (lo..=hi)
        .contains(&v)
        .then_some(Fill::Int { value: v, signed })
}

/// A tag's value as a field's.
fn value(v: TiffValue) -> Value {
    #[allow(deprecated)]
    match v {
        TiffValue::Byte(x) => Value::Integer(x.into()),
        TiffValue::Short(x) => Value::Integer(x.into()),
        TiffValue::SignedByte(x) => Value::Integer(x.into()),
        TiffValue::SignedShort(x) => Value::Integer(x.into()),
        TiffValue::Signed(x) => Value::Integer(x.into()),
        TiffValue::SignedBig(x) => Value::Integer(x),
        TiffValue::Unsigned(x) => Value::Integer(x.into()),
        TiffValue::UnsignedBig(x) => {
            i64::try_from(x).map_or(Value::Float(x as f64), Value::Integer)
        }
        TiffValue::Float(x) => Value::Float(x.into()),
        TiffValue::Double(x) => Value::Float(x),
        TiffValue::Rational(n, d) => Value::Float(f64::from(n) / f64::from(d)),
        TiffValue::SRational(n, d) => Value::Float(f64::from(n) / f64::from(d)),
        TiffValue::RationalBig(n, d) => Value::Float(n as f64 / d as f64),
        TiffValue::SRationalBig(n, d) => Value::Float(n as f64 / d as f64),
        TiffValue::Ascii(s) => Value::Text(s.trim_end_matches('\0').to_string()),
        TiffValue::Ifd(x) => Value::Integer(x.into()),
        TiffValue::IfdBig(x) => i64::try_from(x).map_or(Value::Float(x as f64), Value::Integer),
        TiffValue::List(items) => Value::List(items.into_iter().map(value).collect()),
        // A kind of value a later `tiff` release adds: shown, not hidden (section 14.1).
        other => Value::Text(format!("{other:?}")),
    }
}

/// A tag's value as the file stores it: each number at its type's size, in the file's byte
/// order, and text as its bytes.
fn stored_bytes(v: &TiffValue, big_endian: bool) -> Vec<u8> {
    fn put(out: &mut Vec<u8>, le: &[u8], big_endian: bool) {
        if big_endian {
            out.extend(le.iter().rev());
        } else {
            out.extend_from_slice(le);
        }
    }
    let mut out = Vec::new();
    #[allow(deprecated)]
    match v {
        TiffValue::Byte(x) => out.push(*x),
        TiffValue::SignedByte(x) => out.push(*x as u8),
        TiffValue::Short(x) => put(&mut out, &x.to_le_bytes(), big_endian),
        TiffValue::SignedShort(x) => put(&mut out, &x.to_le_bytes(), big_endian),
        TiffValue::Unsigned(x) | TiffValue::Ifd(x) => put(&mut out, &x.to_le_bytes(), big_endian),
        TiffValue::Signed(x) => put(&mut out, &x.to_le_bytes(), big_endian),
        TiffValue::UnsignedBig(x) | TiffValue::IfdBig(x) => {
            put(&mut out, &x.to_le_bytes(), big_endian)
        }
        TiffValue::SignedBig(x) => put(&mut out, &x.to_le_bytes(), big_endian),
        TiffValue::Float(x) => put(&mut out, &x.to_le_bytes(), big_endian),
        TiffValue::Double(x) => put(&mut out, &x.to_le_bytes(), big_endian),
        TiffValue::Rational(n, d) => {
            put(&mut out, &n.to_le_bytes(), big_endian);
            put(&mut out, &d.to_le_bytes(), big_endian);
        }
        TiffValue::SRational(n, d) => {
            put(&mut out, &n.to_le_bytes(), big_endian);
            put(&mut out, &d.to_le_bytes(), big_endian);
        }
        TiffValue::RationalBig(n, d) => {
            put(&mut out, &n.to_le_bytes(), big_endian);
            put(&mut out, &d.to_le_bytes(), big_endian);
        }
        TiffValue::SRationalBig(n, d) => {
            put(&mut out, &n.to_le_bytes(), big_endian);
            put(&mut out, &d.to_le_bytes(), big_endian);
        }
        TiffValue::Ascii(s) => {
            out.extend_from_slice(s.as_bytes());
            if !s.ends_with('\0') {
                out.push(0);
            }
        }
        TiffValue::List(items) => {
            for item in items {
                out.extend(stored_bytes(item, big_endian));
            }
        }
        _ => {}
    }
    out
}

/// The GeoTIFF keys of a directory: each key's code, value and stored bytes, in the order the
/// key directory lists them (GeoTIFF 1.0, section 2.4).
fn read_keys<R: ReadAt>(
    decoder: &mut Decoder<At<R>>,
    dir: &Directory,
    ascii: &[u8],
    big_endian: bool,
) -> std::result::Result<Vec<(u16, Value, Vec<u8>)>, String> {
    // Little-endian bytes, as the file stores them.
    let order = |le: Vec<u8>, width: usize| -> Vec<u8> {
        if big_endian {
            le.chunks(width)
                .flat_map(|c| c.iter().rev().copied())
                .collect()
        } else {
            le
        }
    };
    let mut t = decoder.read_directory_tags(dir);
    let directory: Vec<u16> = t
        .find_tag_unsigned_vec::<u16>(Tag::GeoKeyDirectoryTag)
        .map_err(|e| e.to_string())?
        .ok_or("no GeoKeyDirectoryTag")?;
    let doubles: Vec<f64> = match t
        .find_tag(Tag::GeoDoubleParamsTag)
        .map_err(|e| e.to_string())?
    {
        Some(v) => v.into_f64_vec().map_err(|e| e.to_string())?,
        None => Vec::new(),
    };
    if directory.len() < 4 {
        return Err("the key directory is shorter than its header".into());
    }
    let count = usize::from(directory[3]);
    if directory.len() < 4 + 4 * count {
        return Err(format!(
            "the key directory lists {count} keys but holds fewer"
        ));
    }
    let mut keys = Vec::with_capacity(count);
    for k in 0..count {
        let e = &directory[4 + 4 * k..8 + 4 * k];
        let (code, location, n, at) = (e[0], e[1], usize::from(e[2]), usize::from(e[3]));
        let out_of = |what: &str| format!("key {code} points past the end of {what}");
        let (v, stored) = match location {
            0 => (
                Value::Integer(at as i64),
                order(e[3].to_le_bytes().to_vec(), 2),
            ),
            34736 => {
                let d = doubles
                    .get(at..at + n)
                    .ok_or_else(|| out_of("GeoDoubleParamsTag"))?;
                let stored = order(d.iter().flat_map(|x| x.to_le_bytes()).collect(), 8);
                let v = if n == 1 {
                    Value::Float(d[0])
                } else {
                    Value::List(d.iter().map(|&x| Value::Float(x)).collect())
                };
                (v, stored)
            }
            34737 => {
                let s = ascii
                    .get(at..at + n)
                    .ok_or_else(|| out_of("GeoAsciiParamsTag"))?;
                // Each string ends with `|`, which is not part of it; the text is Latin-1.
                let text: String = s.iter().map(|&b| char::from(b)).collect();
                (
                    Value::Text(text.trim_end_matches(['|', '\0']).to_string()),
                    s.to_vec(),
                )
            }
            34735 => {
                let s = directory
                    .get(at..at + n)
                    .ok_or_else(|| out_of("the key directory"))?;
                let v = Value::List(s.iter().map(|&x| Value::Integer(x.into())).collect());
                (
                    v,
                    order(s.iter().flat_map(|x| x.to_le_bytes()).collect(), 2),
                )
            }
            other => {
                return Err(format!(
                    "key {code} is stored in tag {other}, not a GeoTIFF tag"
                ))
            }
        };
        keys.push((code, v, stored));
    }
    Ok(keys)
}
