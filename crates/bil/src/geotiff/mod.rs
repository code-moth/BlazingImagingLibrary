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
//! Pixels are decoded by the `tiff` crate, chunk by chunk, strip or tile: uncompressed and
//! Deflate, the compression Tier 1 reads, with their predictors. Other compression is reported,
//! by codec name, and not decoded. GDAL's no-data tag, GDAL_NODATA, is a fill value: a pixel
//! whose every sample holds it is missing (section 4).
//!
//! Every offset and count is checked against the file before it is used, so a corrupt file is
//! an error, never a crash, and no allocation exceeds what the Window asked for. The decoder
//! keeps a place in the file, so it is held under a lock and reads of one Source take turns.

mod names;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use tiff::decoder::ifd::Value as TiffValue;
use tiff::decoder::Decoder;
use tiff::tags::Tag;
use tiff::Directory;

use crate::fields::{Field, FieldGroup, FieldName, Origin, Value};
use crate::io::ReadAt;
pub use crate::types::Scope;
use crate::types::{
    Block, Fragment, Interleave, PixelType, SampleType, Status, Storage, StorageOrder, Window,
};

/// What can go wrong reading a TIFF file.
#[derive(Debug)]
pub enum Error {
    /// The bytes do not begin as TIFF or BigTIFF do.
    NotTiff,
    /// The file breaks its own structure: an offset, count or tag that cannot be right.
    Corrupt(String),
    /// Valid TIFF that this reader does not read.
    Unsupported(String),
    /// A request outside what the Source holds: an Image, a group, a field or a Window.
    NotFound(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotTiff => f.write_str("not a TIFF or BigTIFF file"),
            Error::Corrupt(m) => write!(f, "corrupt TIFF: {m}"),
            Error::Unsupported(m) => write!(f, "not supported: {m}"),
            Error::NotFound(m) => write!(f, "not found: {m}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

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
        other => corrupt(other.to_string()),
    }
}

// ------------------------------------------------------------------ reading through ReadAt

/// A position in a `ReadAt`, for the `tiff` crate, which reads through `Read` and `Seek`.
struct At<R: ReadAt> {
    source: Arc<R>,
    pos: u64,
    size: u64,
}

impl<R: ReadAt> Read for At<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = (self.size.saturating_sub(self.pos)).min(buf.len() as u64) as usize;
        if n == 0 {
            return Ok(0);
        }
        self.source.read_exact_at(&mut buf[..n], self.pos)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<R: ReadAt> Seek for At<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.size.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = pos.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "a seek before the start")
        })?;
        Ok(self.pos)
    }
}

// ------------------------------------------------------------------ what opening finds

/// A reduced-resolution IFD of an Image: one of its stored levels (section 5).
#[derive(Clone, Debug, PartialEq)]
pub struct StoredLevel {
    /// `stored1`, `stored2` and so on, from the finest.
    pub name: String,
    pub rows: u64,
    pub cols: u64,
    /// Full-resolution rows and columns for each of its own, down and across. TIFF overviews do
    /// not say how they were made, so whether a stored level is aligned is not known.
    pub scale: (f64, f64),
}

/// What opening found about one Image.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageInfo {
    pub rows: u64,
    pub cols: u64,
    pub bands: u32,
    /// `None` where the sample layout is valid TIFF that this reader does not read, such as
    /// 1-bit or complex samples; `unsupported` says why.
    pub pixel_type: Option<PixelType>,
    pub unsupported: Option<String>,
    /// The Compression tag's value.
    pub compression: u16,
    /// The codec its chunks need, by name, or `None` when they are not compressed.
    pub codec: Option<String>,
    /// Whether this build decodes its chunks: uncompressed and Deflate.
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
    sample_type: std::result::Result<SampleType, String>,
    bits: u32,
    compression: u16,
    planar: bool,
    tiled: bool,
    chunk: (u32, u32),
    offsets: Vec<u64>,
    counts: Vec<u64>,
    nodata: Option<String>,
    subfile: u32,
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
    layout: std::result::Result<Layout, String>,
}

struct Image {
    info: ImageInfo,
    layout: Layout,
}

/// The `tiff` decoder, and the IFD it last loaded.
struct DecoderState<R: ReadAt> {
    decoder: Decoder<At<R>>,
    current: Option<usize>,
}

/// An open TIFF Source.
pub struct GeoTiffSource<R: ReadAt> {
    reader: Arc<R>,
    size: u64,
    big_endian: bool,
    ifds: Vec<Ifd>,
    images: Vec<Image>,
    decoder: Mutex<DecoderState<R>>,
    held: Mutex<HashMap<(Scope, String), Arc<FieldGroup>>>,
}

/// The most IFDs a chain may hold before it is taken as corrupt: far more than any real file,
/// few enough that walking them is quick.
const MAX_IFDS: usize = 100_000;

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

/// The sample type for TIFF's SampleFormat and bits, or why there is none.
fn sample_type(format: u16, bits: u32) -> std::result::Result<SampleType, String> {
    Ok(match (format, bits) {
        (1, 8) => SampleType::U8,
        (1, 16) => SampleType::U16,
        (1, 32) => SampleType::U32,
        (1, 64) => SampleType::U64,
        (2, 8) => SampleType::I8,
        (2, 16) => SampleType::I16,
        (2, 32) => SampleType::I32,
        (2, 64) => SampleType::I64,
        (3, 16) => SampleType::F16,
        (3, 32) => SampleType::F32,
        (3, 64) => SampleType::F64,
        _ => return Err(format!("SampleFormat {format} with {bits} bits a sample")),
    })
}

impl<R: ReadAt> GeoTiffSource<R> {
    /// Opens a Source: walks the IFD chain, reads each IFD's tags and checks that every chunk
    /// lies within the file.
    pub fn open(reader: R) -> Result<GeoTiffSource<R>> {
        let reader = Arc::new(reader);
        let size = reader.size()?;
        let mut magic = [0u8; 4];
        if size < 8 {
            return Err(Error::NotTiff);
        }
        reader.read_exact_at(&mut magic, 0)?;
        let big_endian = match &magic {
            b"II*\0" | b"II+\0" => false,
            b"MM\0*" | b"MM\0+" => true,
            _ => return Err(Error::NotTiff),
        };
        let at = At {
            source: reader.clone(),
            pos: 0,
            size,
        };
        let mut decoder = Decoder::new(at).map_err(from_tiff)?;
        let first = decoder
            .ifd_pointer()
            .ok_or_else(|| corrupt("no image file directory"))?;
        // The chain, walked here so that an IFD the `tiff` crate cannot make an image of, such
        // as a mask, is still listed; a chain that loops is corrupt.
        let mut ifds = Vec::new();
        let mut seen = HashSet::new();
        let mut next = Some(first);
        while let Some(pointer) = next {
            if !seen.insert(pointer.0) || ifds.len() >= MAX_IFDS {
                return Err(corrupt("the chain of image file directories loops"));
            }
            if pointer.0 >= size {
                return Err(corrupt(format!(
                    "an IFD at {}, past the end of the file",
                    pointer.0
                )));
            }
            let dir = decoder.read_directory(pointer).map_err(from_tiff)?;
            next = dir.next();
            let layout = read_layout(&mut decoder, &dir, size);
            ifds.push(Ifd { dir, layout });
        }
        let mut images: Vec<Image> = Vec::new();
        for (n, ifd) in ifds.iter().enumerate() {
            let layout = match &ifd.layout {
                Ok(l) => l.clone(),
                // An IFD whose layout cannot be read is corrupt where it would be an Image.
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
                    let scale = (
                        image.info.rows as f64 / layout.rows as f64,
                        image.info.cols as f64 / layout.cols as f64,
                    );
                    image.info.stored_levels.push(StoredLevel {
                        name: String::new(),
                        rows: layout.rows,
                        cols: layout.cols,
                        scale,
                    });
                    continue;
                }
            }
            let (pixel_type, unsupported) = match &layout.sample_type {
                Ok(t) => (
                    Some(PixelType {
                        sample_type: *t,
                        component_count: layout.samples,
                        significant_bits: layout.bits,
                        bit_shift: 0,
                    }),
                    None,
                ),
                Err(why) => (None, Some(why.clone())),
            };
            let codec = codec(layout.compression);
            images.push(Image {
                info: ImageInfo {
                    rows: layout.rows,
                    cols: layout.cols,
                    bands: layout.samples,
                    pixel_type,
                    unsupported,
                    compression: layout.compression,
                    decodable: matches!(codec.as_deref(), None | Some("deflate")),
                    codec,
                    planar: layout.planar,
                    tiled: layout.tiled,
                    block_size: layout.chunk,
                    nodata: layout.nodata.clone(),
                    stored_levels: Vec::new(),
                    ifd: n,
                },
                layout,
            });
        }
        if images.is_empty() {
            return Err(corrupt("no full-resolution image"));
        }
        for image in &mut images {
            image
                .info
                .stored_levels
                .sort_by_key(|l| std::cmp::Reverse(l.rows * l.cols));
            for (k, level) in image.info.stored_levels.iter_mut().enumerate() {
                level.name = format!("stored{}", k + 1);
            }
        }
        Ok(GeoTiffSource {
            reader,
            size,
            big_endian,
            ifds,
            images,
            decoder: Mutex::new(DecoderState {
                decoder,
                current: None,
            }),
            held: Mutex::new(HashMap::new()),
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

    /// A field group, extracted on first request and held until released.
    pub fn read_field_group(&self, scope: Scope, group: &str) -> Result<Arc<FieldGroup>> {
        let key = (scope, group.to_string());
        if let Some(g) = self.held.lock().unwrap().get(&key) {
            return Ok(g.clone());
        }
        let g = Arc::new(self.extract(scope, group)?);
        self.held
            .lock()
            .unwrap()
            .entry(key)
            .or_insert_with(|| g.clone());
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
        self.held
            .lock()
            .unwrap()
            .remove(&(scope, group.to_string()));
    }

    fn extract(&self, scope: Scope, group: &str) -> Result<FieldGroup> {
        let not_found = || Error::NotFound(format!("group {group}"));
        let Scope::Image(i) = scope else {
            return Err(not_found());
        };
        let ifd = &self.ifds[self.image(i)?.info.ifd];
        let name = FieldName::parse(group);
        let mut state = self.decoder.lock().unwrap();
        let decoder = &mut state.decoder;
        let mut problem = None;
        let mut fields = Vec::new();
        match group {
            "tiff.ifd" => {
                for (tag, _) in ifd.dir.iter() {
                    let code = tag.to_u16();
                    let part = names::tag_name(code)
                        .map(str::to_string)
                        .unwrap_or_else(|| code.to_string());
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
                let keys = read_keys(decoder, &ifd.dir)
                    .map_err(|m| corrupt(format!("GeoTIFF keys: {m}")))?;
                for (code, v, stored) in keys {
                    let part = names::key_name(code)
                        .map(str::to_string)
                        .unwrap_or_else(|| code.to_string());
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

    /// How the Image's chunks lie in the file (section 4, `describe_storage`).
    pub fn describe_storage(&self, image: usize) -> Result<Storage> {
        let l = &self.image(image)?.layout;
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
                    level: 0,
                    frame: 0,
                    block_row: (within / across) as u32,
                    block_col: (within % across) as u32,
                    components,
                    offset,
                    length,
                }
            })
            .collect();
        blocks.sort_by_key(|b| b.offset);
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
    /// order, with pixels holding the no-data value in every sample marked missing.
    pub fn read_fragment(&self, image: usize, window: Window) -> Result<Fragment> {
        let img = self.image(image)?;
        let (info, l) = (&img.info, &img.layout);
        let pixel_type = info
            .pixel_type
            .ok_or_else(|| Error::Unsupported(info.unsupported.clone().unwrap_or_default()))?;
        if !info.decodable {
            return Err(Error::Unsupported(format!(
                "image {image} is compressed with {}, which this build does not decode",
                info.codec.as_deref().unwrap_or("?")
            )));
        }
        let (rows, cols) = (u64::from(window.row_count), u64::from(window.col_count));
        if window
            .row_start
            .checked_add(rows)
            .is_none_or(|e| e > info.rows)
            || window
                .col_start
                .checked_add(cols)
                .is_none_or(|e| e > info.cols)
        {
            return Err(Error::NotFound(format!(
                "a Window outside image {image}'s {} × {} pixels",
                info.rows, info.cols
            )));
        }
        let size = pixel_type.sample_type.size();
        let bands = info.bands as usize;
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
        let mut state = self.decoder.lock().unwrap();
        if state.current != Some(info.ifd) {
            // seek_to_image counts IFDs along the chain, as `open` did.
            state.decoder.seek_to_image(info.ifd).map_err(from_tiff)?;
            state.current = Some(info.ifd);
        }
        let mut chunk_buf = Vec::new();
        for crow in window.row_start / ch..row_end.div_ceil(ch).min(down) {
            for ccol in window.col_start / cw..col_end.div_ceil(cw).min(across) {
                for plane in 0..l.planes() {
                    let index = u64::from(plane) * across * down + crow * across + ccol;
                    let index = u32::try_from(index).map_err(|_| corrupt("too many chunks"))?;
                    // Within the Image's chunks, by the loops' bounds.
                    let (w, h) = state.decoder.chunk_data_dimensions(index);
                    let in_chunk = if l.planar { 1 } else { bands };
                    let row_bytes = w as usize * in_chunk * size;
                    chunk_buf.resize(row_bytes * h as usize, 0);
                    // The chunk's bytes are its own; a fill value read before is not kept.
                    state
                        .decoder
                        .read_chunk_bytes(index, &mut chunk_buf)
                        .map_err(from_tiff)?;
                    let r0 = (crow * ch).max(window.row_start);
                    let r1 = ((crow * ch) + u64::from(h)).min(row_end);
                    let c0 = (ccol * cw).max(window.col_start);
                    let c1 = ((ccol * cw) + u64::from(w)).min(col_end);
                    for r in r0..r1 {
                        let src_row = (r - crow * ch) as usize * row_bytes;
                        let dst_row = (r - window.row_start) as usize * strides[0];
                        if l.planar {
                            for c in c0..c1 {
                                let s = src_row + (c - ccol * cw) as usize * size;
                                let d = dst_row
                                    + (c - window.col_start) as usize * strides[1]
                                    + plane as usize * size;
                                data[d..d + size].copy_from_slice(&chunk_buf[s..s + size]);
                            }
                        } else {
                            let s = src_row + (c0 - ccol * cw) as usize * strides[1];
                            let d = dst_row + (c0 - window.col_start) as usize * strides[1];
                            let n = (c1 - c0) as usize * strides[1];
                            data[d..d + n].copy_from_slice(&chunk_buf[s..s + n]);
                        }
                    }
                }
            }
        }
        drop(state);
        let validity = info
            .nodata
            .as_deref()
            .and_then(|text| fill(text, pixel_type.sample_type))
            .and_then(|fill| {
                let valid: Vec<bool> = data
                    .chunks_exact(strides[1])
                    .map(|pixel| !pixel.chunks_exact(size).all(|s| fill.matches(s)))
                    .collect();
                valid.contains(&false).then_some(valid)
            });
        Ok(Fragment {
            window,
            pixel_type,
            strides,
            data,
            validity,
        })
    }
}

/// Reads one IFD's layout, checking every offset against the file.
fn read_layout<R: ReadAt>(
    decoder: &mut Decoder<At<R>>,
    dir: &Directory,
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
    let expected = rows.div_ceil(chunk.0) * cols.div_ceil(chunk.1) * planes;
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
    let bits0 = bits[0];
    let format0 = formats[0];
    let sample_type = if bits.iter().any(|&b| b != bits0) || formats.iter().any(|&f| f != format0) {
        Err("samples of different bits or formats in one pixel".to_string())
    } else {
        sample_type(format0 as u16, bits0 as u32)
    };
    let mut t = decoder.read_directory_tags(dir);
    let nodata = match t.find_tag(Tag::GdalNodata) {
        Ok(Some(TiffValue::Ascii(s))) => Some(s.trim_end_matches('\0').trim().to_string()),
        _ => None,
    };
    Ok(Layout {
        rows,
        cols,
        samples: samples as u32,
        sample_type,
        bits: bits0 as u32,
        compression: u16::try_from(compression).map_err(|_| "a Compression past 65,535")?,
        planar,
        tiled,
        chunk: (chunk.0 as u32, chunk.1 as u32),
        offsets,
        counts,
        nodata,
        subfile: subfile as u32,
    })
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

/// The no-data value for a sample type, where the text is one it can hold.
fn fill(text: &str, t: SampleType) -> Option<Fill> {
    let float = matches!(t, SampleType::F16 | SampleType::F32 | SampleType::F64);
    if float {
        if text.eq_ignore_ascii_case("nan") {
            return Some(Fill::NaN);
        }
        let v: f64 = text.parse().ok()?;
        // As the sample type holds it.
        // As float32 holds it; a value float16 cannot hold exactly matches no sample.
        return Some(Fill::Float(match t {
            SampleType::F32 => f64::from(v as f32),
            _ => v,
        }));
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
) -> std::result::Result<Vec<(u16, Value, Vec<u8>)>, String> {
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
    let ascii: String = match t
        .find_tag(Tag::GeoAsciiParamsTag)
        .map_err(|e| e.to_string())?
    {
        Some(TiffValue::Ascii(s)) => s,
        Some(_) => return Err("GeoAsciiParamsTag is not text".into()),
        None => String::new(),
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
            0 => (Value::Integer(at as i64), e[3].to_le_bytes().to_vec()),
            34736 => {
                let d = doubles
                    .get(at..at + n)
                    .ok_or_else(|| out_of("GeoDoubleParamsTag"))?;
                let stored = d.iter().flat_map(|x| x.to_le_bytes()).collect();
                let v = if n == 1 {
                    Value::Float(d[0])
                } else {
                    Value::List(d.iter().map(|&x| Value::Float(x)).collect())
                };
                (v, stored)
            }
            34737 => {
                let s = ascii
                    .as_bytes()
                    .get(at..at + n)
                    .ok_or_else(|| out_of("GeoAsciiParamsTag"))?;
                // Each string ends with `|`, which is not part of it.
                let text = String::from_utf8_lossy(s);
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
                (v, s.iter().flat_map(|x| x.to_le_bytes()).collect())
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
