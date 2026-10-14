// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Tier 1's NITF container reader (specification, sections 1.4 and 10): NITF 2.0, NITF 2.1 and
//! NSIF 1.0.
//!
//! Opening reads the file header and every image subheader, and nothing else: enough to
//! describe each Image (section 14.3, rule 1). Extensions that overflow into data extension
//! segments are read when their header's groups are first asked for. Each image segment is an
//! Image. The headers and the extensions in them are read through the field definitions in
//! [`defs`]; an extension is extracted as a group of fields when first asked for, and held. Pixels that are not
//! compressed are read here; compressed blocks are a codec's work (section 10, "Containers and
//! codecs"), and this reader only says where they lie and which codec they need.
//!
//! Every length and count is checked against the file before it is used, so a corrupt file is
//! an error and never a crash, and no allocation exceeds the file's size or the Window asked
//! for.

pub(crate) mod defs;

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use crate::fields::{Field, FieldGroup, FieldName, Origin};
use crate::io::ReadAt;
use crate::types::{
    Block, Fragment, Interleave, PixelType, SampleType, Status, Storage, StorageOrder, Window,
};
use defs::{walk, WalkError, Walked};

/// What can go wrong reading NITF.
#[derive(Debug)]
pub enum Error {
    /// The bytes do not begin as NITF 2.0, NITF 2.1 or NSIF 1.0 do.
    NotNitf,
    /// The file breaks its own structure: a length, count or field that cannot be right.
    Corrupt(String),
    /// Valid NITF that this reader does not read.
    Unsupported(String),
    /// A request outside what the Source holds: an Image, a group, a field or a Window.
    NotFound(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotNitf => f.write_str("not a NITF 2.0, NITF 2.1 or NSIF 1.0 file"),
            Error::Corrupt(m) => write!(f, "corrupt NITF: {m}"),
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

/// The version of the format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Nitf20,
    Nitf21,
    /// NSIF 1.0, the NATO profile, the same as NITF 2.1 in structure.
    Nsif10,
}

/// The kinds of segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    Image,
    /// NITF 2.1's graphics, and NITF 2.0's symbols, which they replaced.
    Graphic,
    /// NITF 2.0 only.
    Label,
    Text,
    DataExtension,
    ReservedExtension,
}

/// A segment: its subheader and its data, as offsets and lengths in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub kind: SegmentKind,
    pub subheader: (u64, u64),
    pub data: (u64, u64),
}

/// What a field group is attached to (section 14.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Source,
    Image(usize),
}

/// What opening found about one image segment.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageInfo {
    pub rows: u64,
    pub cols: u64,
    pub bands: u32,
    /// `None` where the sample layout is valid NITF that this reader does not read, such as 4
    /// bits a sample; `unsupported` says why.
    pub pixel_type: Option<PixelType>,
    pub unsupported: Option<String>,
    /// The compression code, IC, as the file states it: `NC` and `NM` are uncompressed.
    pub compression: String,
    /// The codec its blocks need, by name, or `None` when they are not compressed.
    pub codec: Option<String>,
    /// Whether a block mask table precedes the data (IC `NM` or `M1` to `M8`).
    pub masked: bool,
    /// IMODE: `B`, `P`, `R` or `S`.
    pub imode: char,
    pub blocks_per_row: u32,
    pub blocks_per_col: u32,
    /// Rows and columns of one block.
    pub block_size: (u32, u32),
    /// Bits a sample is stored in (NBPP).
    pub stored_bits: u32,
    /// Where the image data lies: offset and length.
    pub data: (u64, u64),
}

/// A header's bytes and the fields the walk found in them.
#[derive(Debug)]
struct Header {
    bytes: Vec<u8>,
    fields: Vec<Walked>,
    /// The extensions in the header's own extension areas, found at open: tag, and where their
    /// data lies in `bytes`.
    own: Vec<(String, usize, usize)>,
    /// Why some of those could not be read, where that is so: the header's group is then
    /// `Partial` (section 3.3), and its extensions are those read before the damage.
    own_problem: Option<String>,
    /// The header's fields that can name a segment its extensions overflow into, and those
    /// extensions, read when first asked for.
    overflow_fields: &'static [&'static str],
    overflow: OnceLock<Extensions>,
}

/// Extensions read from segments a header overflows into, with the bytes they lie in.
#[derive(Debug, Default)]
struct Extensions {
    bytes: Vec<u8>,
    found: Vec<(String, usize, usize)>,
    problem: Option<String>,
}

/// Reads the extensions in `bytes[at..end]`: each a tag of 6 bytes, a length of 5 digits and
/// that many bytes of data. At the first that does not fit, it stops and says why.
fn scan(
    bytes: &[u8],
    mut at: usize,
    end: usize,
    place: &str,
) -> (Vec<(String, usize, usize)>, Option<String>) {
    let mut found = Vec::new();
    while at < end {
        if end - at < 11 {
            return (found, Some(format!("an extension in {place} is cut short")));
        }
        let tag: String = bytes[at..at + 6]
            .iter()
            .map(|&c| char::from(c))
            .collect::<String>()
            .trim_end()
            .to_string();
        if tag.is_empty() || !tag.bytes().all(|b| b.is_ascii_graphic()) {
            return (
                found,
                Some(format!("an extension in {place} has no valid tag")),
            );
        }
        let len = std::str::from_utf8(&bytes[at + 6..at + 11])
            .ok()
            .filter(|t| t.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|t| t.parse::<usize>().ok());
        let Some(len) = len else {
            return (
                found,
                Some(format!("extension {tag} in {place} has no valid length")),
            );
        };
        if end - (at + 11) < len {
            return (
                found,
                Some(format!(
                    "extension {tag} claims {len} bytes, more than {place} holds"
                )),
            );
        }
        found.push((tag, at + 11, len));
        at += 11 + len;
    }
    (found, None)
}

/// Group names for a list of extension tags: a tag that occurs more than once carries its
/// number.
fn extension_names<'a>(tags: impl Iterator<Item = &'a str> + Clone) -> Vec<String> {
    let mut totals: HashMap<&str, usize> = HashMap::new();
    for t in tags.clone() {
        *totals.entry(t).or_insert(0) += 1;
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    tags.map(|tag| {
        let n = seen.entry(tag).or_insert(0);
        *n += 1;
        if totals[tag] > 1 {
            format!("{tag}[{n}]")
        } else {
            tag.to_string()
        }
    })
    .collect()
}

impl Header {
    fn stored(&self, part: &str) -> Option<&[u8]> {
        self.fields
            .iter()
            .find(|w| w.part == part)
            .map(|w| &self.bytes[w.at..w.at + w.len])
    }

    fn text(&self, part: &str) -> String {
        self.stored(part)
            .map(|b| b.iter().map(|&c| char::from(c)).collect::<String>())
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    /// A field's whole number; blank is 0. Fails where the field is absent or not a number.
    fn number(&self, part: &str) -> Result<u64> {
        let text = self.text(part);
        if text.is_empty() {
            return Ok(0);
        }
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(corrupt(format!("{part} is not a number: {text:?}")));
        }
        text.parse()
            .map_err(|_| corrupt(format!("{part} is out of range")))
    }

    /// Finds the extensions in the named fields, and notes which fields can name overflow.
    fn find_extensions(&mut self, parts: &[&str], overflow: &'static [&'static str]) {
        for part in parts {
            if let Some(w) = self.fields.iter().find(|w| w.part == *part) {
                let (found, problem) = scan(&self.bytes, w.at, w.at + w.len, part);
                self.own.extend(found);
                if let Some(p) = problem {
                    self.own_problem.get_or_insert(p);
                }
            }
        }
        self.overflow_fields = overflow;
    }
}

/// The block mask table of a masked image (MIL-STD-2500C, table A-3(A)).
#[derive(Debug, Clone)]
struct MaskTable {
    /// Where the blocks start, from the start of the image data.
    data_start: u64,
    /// For each stored block unit, its offset from `data_start`, or `None` where it was never
    /// recorded. Empty where the table gives no offsets.
    offsets: Vec<Option<u64>>,
    /// The pad pixel value, where one is stated.
    pad: Option<u64>,
}

struct Image {
    info: ImageInfo,
    header: Header,
    mask: OnceLock<std::result::Result<MaskTable, String>>,
}

/// An open NITF Source.
pub struct NitfSource<R: ReadAt> {
    reader: R,
    size: u64,
    version: Version,
    header: Header,
    segments: Vec<Segment>,
    images: Vec<Image>,
    held: Mutex<HashMap<(Scope, String), Arc<FieldGroup>>>,
}

/// Reads `len` bytes at `offset`, which the caller has checked lie within the file.
fn read<R: ReadAt>(reader: &R, offset: u64, len: u64) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; usize::try_from(len).map_err(|_| corrupt("a length is too large"))?];
    reader.read_exact_at(&mut buf, offset)?;
    Ok(buf)
}

/// Reads a header from `offset` through `layout`, reading more as the walk needs it, at most
/// `limit` bytes. With `stop`, only as far as that field.
fn read_header<R: ReadAt>(
    reader: &R,
    offset: u64,
    limit: u64,
    layout: &[&[defs::Item]],
    stop: Option<&'static str>,
    what: &str,
) -> Result<Header> {
    // Enough for the fields that locate the rest in most headers: HL lies within the first 400
    // bytes of either version. The walk asks for more where a header needs it.
    let mut want = limit.min(512);
    loop {
        let bytes = read(reader, offset, want)?;
        match walk(&bytes, layout, stop) {
            Ok((fields, _)) => {
                return Ok(Header {
                    bytes,
                    fields,
                    own: Vec::new(),
                    own_problem: None,
                    overflow_fields: &[],
                    overflow: OnceLock::new(),
                })
            }
            Err(WalkError::Truncated(needed))
                if (needed as u64) <= limit && needed as u64 > want =>
            {
                want = needed as u64;
            }
            Err(WalkError::Truncated(_)) => {
                return Err(corrupt(format!("the {what} is cut short")))
            }
            Err(WalkError::Corrupt(m)) => return Err(corrupt(format!("in the {what}: {m}"))),
        }
    }
}

/// The sample type for NITF's pixel value type and bits a sample, or why there is none.
fn sample_type(pvtype: &str, nbpp: u32) -> std::result::Result<SampleType, String> {
    Ok(match (pvtype, nbpp) {
        ("INT" | "B", 1) | ("INT", 8) => SampleType::U8,
        ("INT", 12 | 16) => SampleType::U16,
        ("INT", 32) => SampleType::U32,
        ("INT", 64) => SampleType::U64,
        ("SI", 8) => SampleType::I8,
        ("SI", 16) => SampleType::I16,
        ("SI", 32) => SampleType::I32,
        ("SI", 64) => SampleType::I64,
        ("R", 32) => SampleType::F32,
        ("R", 64) => SampleType::F64,
        ("C", 64) => SampleType::ComplexF32,
        _ => {
            return Err(format!(
                "pixel value type {pvtype:?} with {nbpp} bits a sample"
            ))
        }
    })
}

/// The codec a compression code needs, by name; `None` for uncompressed data.
fn codec(ic: &str) -> Option<String> {
    Some(
        match ic {
            "NC" | "NM" => return None,
            "C1" | "M1" => "bilevel",
            "C3" | "M3" => "jpeg",
            "C4" | "M4" => "vector_quantization",
            "C5" | "M5" => "lossless_jpeg",
            "C8" | "M8" => "jpeg2000",
            "I1" => "downsampled_jpeg",
            other => return Some(format!("nitf_{}", other.to_ascii_lowercase())),
        }
        .to_string(),
    )
}

impl<R: ReadAt> NitfSource<R> {
    /// Opens a Source: reads the file header and the image subheaders, and checks that every
    /// segment lies within the file.
    pub fn open(reader: R) -> Result<NitfSource<R>> {
        let size = reader.size()?;
        if size < 9 {
            return Err(Error::NotNitf);
        }
        let magic = read(&reader, 0, 9)?;
        let (version, layout): (Version, [&[defs::Item]; 2]) = match &magic[..] {
            b"NITF02.10" => (
                Version::Nitf21,
                [defs::FILE_HEADER_21, &defs::FILE_HEADER_TAIL],
            ),
            b"NSIF01.00" => (
                Version::Nsif10,
                [defs::FILE_HEADER_21, &defs::FILE_HEADER_TAIL],
            ),
            b"NITF02.00" => (
                Version::Nitf20,
                [defs::FILE_HEADER_20, &defs::FILE_HEADER_TAIL],
            ),
            _ => return Err(Error::NotNitf),
        };
        // HL, the header's length, comes after fields some of which are optional, so the walk
        // finds it first, then reads the whole header.
        let head = read_header(&reader, 0, size, &layout, Some("HL"), "file header")?;
        let hl = head.number("HL")?;
        if hl > size {
            return Err(corrupt(format!(
                "the file header claims {hl} bytes, more than the file's {size}"
            )));
        }
        let mut header = read_header(&reader, 0, hl, &layout, None, "file header")?;
        header.find_extensions(&["UDHD", "XHD"], &["UDHOFL", "XHDLOFL"]);

        // The segments lie one after another after the file header, in a fixed order of kinds.
        let kinds: &[(SegmentKind, &str, &str, &str)] = match version {
            Version::Nitf20 => &[
                (SegmentKind::Image, "NUMI", "LISH", "LI"),
                (SegmentKind::Graphic, "NUMS", "LSSH", "LS"),
                (SegmentKind::Label, "NUML", "LLSH", "LL"),
                (SegmentKind::Text, "NUMT", "LTSH", "LT"),
                (SegmentKind::DataExtension, "NUMDES", "LDSH", "LD"),
                (SegmentKind::ReservedExtension, "NUMRES", "LRESH", "LRE"),
            ],
            _ => &[
                (SegmentKind::Image, "NUMI", "LISH", "LI"),
                (SegmentKind::Graphic, "NUMS", "LSSH", "LS"),
                (SegmentKind::Text, "NUMT", "LTSH", "LT"),
                (SegmentKind::DataExtension, "NUMDES", "LDSH", "LD"),
                (SegmentKind::ReservedExtension, "NUMRES", "LRESH", "LRE"),
            ],
        };
        let mut segments = Vec::new();
        let mut at = hl;
        for &(kind, count, sub, data) in kinds {
            for i in 1..=header.number(count)? {
                let s = header.number(&format!("{sub}[{i}]"))?;
                let d = header.number(&format!("{data}[{i}]"))?;
                let end = at.checked_add(s).and_then(|e| e.checked_add(d));
                if end.is_none_or(|e| e > size) {
                    return Err(corrupt(format!(
                        "segment {i} of {count} claims to end past the file's {size} bytes"
                    )));
                }
                segments.push(Segment {
                    kind,
                    subheader: (at, s),
                    data: (at + s, d),
                });
                at += s + d;
            }
        }

        let image_layout: [&[defs::Item]; 1] = match version {
            Version::Nitf20 => [defs::IMAGE_SUBHEADER_20],
            _ => [defs::IMAGE_SUBHEADER_21],
        };
        let mut images = Vec::new();
        for (n, seg) in segments
            .iter()
            .filter(|s| s.kind == SegmentKind::Image)
            .enumerate()
        {
            let what = format!("subheader of image {n}");
            let mut sub = read_header(
                &reader,
                seg.subheader.0,
                seg.subheader.1,
                &image_layout,
                None,
                &what,
            )?;
            sub.find_extensions(&["UDID", "IXSHD"], &["UDOFL", "IXSOFL"]);
            let info = describe(&sub, seg.data).map_err(|e| match e {
                Error::Corrupt(m) => corrupt(format!("image {n}: {m}")),
                other => other,
            })?;
            images.push(Image {
                info,
                header: sub,
                mask: OnceLock::new(),
            });
        }
        Ok(NitfSource {
            reader,
            size,
            version,
            header,
            segments,
            images,
            held: Mutex::new(HashMap::new()),
        })
    }

    /// The bytes the Source reads from.
    pub fn reader(&self) -> &R {
        &self.reader
    }

    pub fn version(&self) -> Version {
        self.version
    }

    /// The file's size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
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

    fn header(&self, scope: Scope) -> Result<(&Header, &'static str)> {
        Ok(match scope {
            Scope::Source => (&self.header, "file_header"),
            Scope::Image(i) => (&self.image(i)?.header, "image_subheader"),
        })
    }

    // -------------------------------------------------------------- fields

    /// The names of the field groups attached to `scope`: its header, then each extension.
    /// Extracts nothing.
    pub fn list_field_groups(&self, scope: Scope) -> Result<Vec<String>> {
        let (header, name) = self.header(scope)?;
        let (extensions, _) = self.extensions(header);
        Ok(std::iter::once(format!("nitf.{name}"))
            .chain(
                extension_names(extensions.iter().map(|(t, _)| *t))
                    .into_iter()
                    .map(|t| format!("nitf.{t}")),
            )
            .collect())
    }

    /// Every extension of a header, its own and those in segments it overflows into, which are
    /// read here on first use; and why some could not be read, where that is so.
    fn extensions<'a>(&'a self, header: &'a Header) -> (Vec<(&'a str, &'a [u8])>, Option<&'a str>) {
        let over = header.overflow.get_or_init(|| {
            let des: Vec<Segment> = self
                .segments
                .iter()
                .filter(|s| s.kind == SegmentKind::DataExtension)
                .copied()
                .collect();
            read_overflow(&self.reader, self.version, &des, header)
        });
        let own = header
            .own
            .iter()
            .map(|(t, at, len)| (t.as_str(), &header.bytes[*at..*at + len]));
        let more = over
            .found
            .iter()
            .map(|(t, at, len)| (t.as_str(), &over.bytes[*at..*at + len]));
        (
            own.chain(more).collect(),
            header.own_problem.as_deref().or(over.problem.as_deref()),
        )
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
        if parsed.0.len() < 3 {
            return Err(Error::NotFound(format!("field {name}")));
        }
        let group = self.read_field_group(scope, &FieldName(parsed.0[..2].to_vec()).dotted())?;
        if let Some(f) = group.fields.iter().find(|f| f.name == parsed) {
            return Ok(f.clone());
        }
        // A field the definitions name, which this file leaves out, such as IGEOLO where ICORDS
        // is blank, is delivered as Missing; only a name nothing defines is an error (section
        // 3.3).
        let native = parsed.0[2].split('[').next().unwrap_or("");
        let layouts: Vec<&[defs::Item]> = match (scope, parsed.0[1].as_str()) {
            (Scope::Source, "file_header") => match self.version {
                Version::Nitf20 => vec![defs::FILE_HEADER_20, &defs::FILE_HEADER_TAIL],
                _ => vec![defs::FILE_HEADER_21, &defs::FILE_HEADER_TAIL],
            },
            (Scope::Image(_), "image_subheader") => match self.version {
                Version::Nitf20 => vec![defs::IMAGE_SUBHEADER_20],
                _ => vec![defs::IMAGE_SUBHEADER_21],
            },
            (_, group) => defs::extension_layout(group.split('[').next().unwrap_or(""))
                .into_iter()
                .collect(),
        };
        // The name must carry one well-formed number for each run the field lies inside.
        let numbers = parsed.0[2][native.len()..]
            .split_inclusive(']')
            .map(|n| {
                n.strip_prefix('[')
                    .and_then(|n| n.strip_suffix(']'))
                    .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            })
            .collect::<Vec<_>>();
        match defs::definition(&layouts, native) {
            Some((def, depth)) if numbers.len() == depth && numbers.iter().all(|&ok| ok) => {
                Ok(Field::missing(parsed, Some(def), Origin::Specified))
            }
            _ => Err(Error::NotFound(format!("field {name}"))),
        }
    }

    /// Frees a held group; it is extracted again if asked for.
    pub fn release_metadata(&self, scope: Scope, group: &str) {
        self.held
            .lock()
            .unwrap()
            .remove(&(scope, group.to_string()));
    }

    fn extract(&self, scope: Scope, group: &str) -> Result<FieldGroup> {
        let (header, header_name) = self.header(scope)?;
        let name = FieldName::parse(group);
        let not_found = || Error::NotFound(format!("group {group}"));
        if name.0.len() != 2 || name.0[0] != "nitf" {
            return Err(not_found());
        }
        let field = |part: &str, bytes: &[u8], def| {
            Field::new(name.child(part), bytes.to_vec(), def, Origin::Specified)
        };
        let (extensions, problem) = self.extensions(header);
        if name.0[1] == header_name {
            let fields = header
                .fields
                .iter()
                .map(|w| field(&w.part, &header.bytes[w.at..w.at + w.len], Some(w.def)))
                .collect();
            return Ok(FieldGroup {
                name,
                fields,
                status: if problem.is_some() {
                    Status::Partial
                } else {
                    Status::Complete
                },
                problem: problem.map(str::to_string),
            });
        }
        let names = extension_names(extensions.iter().map(|(t, _)| *t));
        let i = names
            .iter()
            .position(|n| *n == name.0[1])
            .ok_or_else(not_found)?;
        let (tag, data) = extensions[i];
        // Through the extension's definitions where BIL has them and they fit its length
        // exactly; otherwise, its data as bytes, under the standard's name for it.
        let fields = match defs::extension_layout(tag).map(|l| walk(data, &[l], None)) {
            Some(Ok((found, end))) if end == data.len() => found
                .iter()
                .map(|w| field(&w.part, &data[w.at..w.at + w.len], Some(w.def)))
                .collect(),
            _ => vec![field("CEDATA", data, None)],
        };
        Ok(FieldGroup {
            name,
            fields,
            status: Status::Complete,
            problem: None,
        })
    }

    // -------------------------------------------------------------- pixels

    /// The block mask table of a masked image, read on first use.
    fn mask(&self, image: usize) -> Result<Option<&MaskTable>> {
        let img = self.image(image)?;
        if !img.info.masked {
            return Ok(None);
        }
        img.mask
            .get_or_init(|| read_mask(&self.reader, &img.info).map_err(|e| e.to_string()))
            .as_ref()
            .map(Some)
            .map_err(|m| corrupt(format!("image {image}'s block mask: {m}")))
    }

    /// Where stored unit `k` of an Image's blocks lies, from the start of the file, or `None`
    /// where it was never recorded. Uncompressed units lie one after another unless a mask
    /// table gives their offsets.
    fn unit_start(
        info: &ImageInfo,
        mask: Option<&MaskTable>,
        unit_bytes: u64,
        k: u64,
    ) -> Option<u64> {
        match mask {
            Some(m) if !m.offsets.is_empty() => {
                m.offsets[k as usize].map(|v| info.data.0 + m.data_start + v)
            }
            Some(m) => Some(info.data.0 + m.data_start + k * unit_bytes),
            None => Some(info.data.0 + k * unit_bytes),
        }
    }

    /// How the Image's blocks lie in the file (section 4, `describe_storage`).
    pub fn describe_storage(&self, image: usize) -> Result<Storage> {
        let info = &self.image(image)?.info;
        let g = Geometry::of(info);
        let interleave = match info.imode {
            'P' => Interleave::Pixel,
            'R' => Interleave::Line,
            _ => Interleave::Band,
        };
        let mut storage = Storage {
            block_size: info.block_size,
            interleave,
            order: StorageOrder::Unknown,
            blocks: Vec::new(),
        };
        let mask = self.mask(image)?;
        let data_end = info.data.0 + info.data.1;
        if info.codec.is_some() && mask.is_none_or(|m| m.offsets.is_empty()) {
            // Compressed data with no block offsets lies in one run; its blocks' places are
            // inside the compressed stream, which is the codec's to read. With one block, the
            // block is the run.
            if g.units == 1 {
                let offset = info.data.0 + mask.map_or(0, |m| m.data_start);
                storage.blocks.push(Block {
                    image: image as u32,
                    level: 0,
                    frame: 0,
                    block_row: 0,
                    block_col: 0,
                    components: 0..info.bands,
                    offset,
                    length: data_end - offset,
                });
                storage.order = StorageOrder::Known;
            }
            return Ok(storage);
        }
        let fixed = info.codec.is_none().then_some(g.unit_bytes);
        let mut placed: Vec<(u64, u64)> = (0..g.units)
            .filter_map(|k| Self::unit_start(info, mask, g.unit_bytes, k).map(|s| (s, k)))
            .collect();
        placed.sort_unstable();
        for (n, &(offset, k)) in placed.iter().enumerate() {
            let length = match fixed {
                Some(len) => len,
                // Compressed blocks run to the next block, or to the end of the data.
                None => placed.get(n + 1).map_or(data_end, |&(next, _)| next) - offset,
            };
            let (block, components) = g.unit_place(k);
            storage.blocks.push(Block {
                image: image as u32,
                level: 0,
                frame: 0,
                block_row: (block / u64::from(info.blocks_per_row)) as u32,
                block_col: (block % u64::from(info.blocks_per_row)) as u32,
                components,
                offset,
                length,
            });
        }
        storage.order = StorageOrder::Known;
        Ok(storage)
    }

    /// The native pixels of a Window of an uncompressed Image, pixel-interleaved, in native byte
    /// order, with the pixels never recorded marked: those in blocks the mask table leaves out,
    /// and pad pixels.
    pub fn read_fragment(&self, image: usize, window: Window) -> Result<Fragment> {
        let info = &self.image(image)?.info;
        let pixel_type = info
            .pixel_type
            .ok_or_else(|| Error::Unsupported(info.unsupported.clone().unwrap_or_default()))?;
        if let Some(c) = &info.codec {
            return Err(Error::Unsupported(format!(
                "image {image} is compressed; its blocks go to the {c} codec"
            )));
        }
        let rows = u64::from(window.row_count);
        let cols = u64::from(window.col_count);
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
        let g = Geometry::of(info);
        let bands = info.bands as usize;
        let size = pixel_type.sample_type.size();
        // The Fragment's bytes, checked before anything is taken for them.
        rows.checked_mul(cols)
            .and_then(|p| p.checked_mul(bands as u64))
            .and_then(|p| p.checked_mul(size as u64))
            .filter(|&n| n <= isize::MAX as u64)
            .ok_or_else(|| {
                Error::Unsupported(format!(
                    "a Window of {rows} × {cols} pixels of {bands} bands, more than can be held"
                ))
            })?;
        let strides = [cols as usize * bands * size, bands * size, size];
        let mut data = vec![0u8; rows as usize * strides[0]];
        let mut valid = vec![true; (rows * cols) as usize];
        let mut any_missing = false;
        let mask = self.mask(image)?;
        let pad = mask.and_then(|m| m.pad);
        let (bh, bw) = (u64::from(info.block_size.0), u64::from(info.block_size.1));
        let row_end = window.row_start + rows;
        let col_end = window.col_start + cols;
        for brow in window.row_start / bh..row_end.div_ceil(bh) {
            for bcol in window.col_start / bw..col_end.div_ceil(bw) {
                let block = brow * u64::from(info.blocks_per_row) + bcol;
                // The part of this block inside the Window, in Image rows and columns.
                let r0 = (brow * bh).max(window.row_start);
                let r1 = ((brow + 1) * bh).min(row_end);
                let c0 = (bcol * bw).max(window.col_start);
                let c1 = ((bcol + 1) * bw).min(col_end);
                for (unit, bands_in_unit) in g.units_of_block(block) {
                    let Some(start) = Self::unit_start(info, mask, g.unit_bytes, unit) else {
                        // Never recorded: the pixels stay 0 and are marked.
                        any_missing = true;
                        for r in r0..r1 {
                            for c in c0..c1 {
                                valid[((r - window.row_start) * cols + c - window.col_start)
                                    as usize] = false;
                            }
                        }
                        continue;
                    };
                    if start + g.unit_bytes > info.data.0 + info.data.1 {
                        return Err(corrupt(format!(
                            "image {image}: block {block} lies past the end of its data"
                        )));
                    }
                    let bytes = read(&self.reader, start, g.unit_bytes)?;
                    for b in bands_in_unit {
                        for r in r0..r1 {
                            for c in c0..c1 {
                                let bit = g.bit_offset(r - brow * bh, c - bcol * bw, b);
                                let out = ((r - window.row_start) as usize * strides[0])
                                    + ((c - window.col_start) as usize * strides[1])
                                    + b as usize * size;
                                g.copy_sample(&bytes, bit, &mut data[out..out + size]);
                            }
                        }
                    }
                }
            }
        }
        if let Some(pad) = pad {
            // A pad pixel is one whose every sample holds the pad value.
            for p in 0..(rows * cols) as usize {
                let pixel = &data[p * strides[1]..(p + 1) * strides[1]];
                if pixel.chunks_exact(size).all(|s| native_value(s) == pad) {
                    valid[p] = false;
                    any_missing = true;
                }
            }
        }
        Ok(Fragment {
            window,
            pixel_type,
            strides,
            data,
            validity: any_missing.then_some(valid),
        })
    }
}

/// Reads the extensions a header overflows into data extension segments: where one of the
/// header's overflow fields names a segment, counted from 1, that segment's data holds more
/// extensions of the same header. Problems are recorded, as for the header's own extensions.
fn read_overflow<R: ReadAt>(
    reader: &R,
    version: Version,
    des: &[Segment],
    header: &Header,
) -> Extensions {
    let (layout, tag_field, tags) = match version {
        Version::Nitf20 => (defs::DES_SUBHEADER_20, "DESTAG", defs::OVERFLOW_20),
        _ => (defs::DES_SUBHEADER_21, "DESID", defs::OVERFLOW_21),
    };
    let mut out = Extensions::default();
    for field in header.overflow_fields {
        let n = match header.number(field) {
            Ok(0) => continue,
            Ok(n) => n,
            Err(e) => {
                out.problem.get_or_insert(e.to_string());
                continue;
            }
        };
        let place = format!("data extension segment {n}, named by {field}");
        let data = (|| {
            let seg = des
                .get(n as usize - 1)
                .ok_or_else(|| format!("{place} does not exist"))?;
            let sub = read_header(
                reader,
                seg.subheader.0,
                seg.subheader.1,
                &[layout],
                None,
                &place,
            )
            .map_err(|e| e.to_string())?;
            if !sub.stored(tag_field).is_some_and(|t| tags.contains(&t)) {
                return Err(format!("{place} does not hold extensions"));
            }
            read(reader, seg.data.0, seg.data.1).map_err(|e| e.to_string())
        })();
        match data {
            Ok(data) => {
                let at = out.bytes.len();
                out.bytes.extend_from_slice(&data);
                let (found, problem) = scan(&out.bytes, at, out.bytes.len(), &place);
                out.found.extend(found);
                if let Some(p) = problem {
                    out.problem.get_or_insert(p);
                }
            }
            Err(why) => {
                out.problem.get_or_insert(why);
            }
        }
    }
    out
}

/// A sample's value from its native bytes, as bits, for comparison with a pad value.
fn native_value(sample: &[u8]) -> u64 {
    let mut v = [0u8; 8];
    let n = sample.len().min(8);
    if cfg!(target_endian = "little") {
        v[..n].copy_from_slice(&sample[..n]);
        u64::from_le_bytes(v)
    } else {
        v[8 - n..].copy_from_slice(&sample[..n]);
        u64::from_be_bytes(v)
    }
}

/// Checks an image subheader's numbers and works out what they describe.
fn describe(sub: &Header, data: (u64, u64)) -> Result<ImageInfo> {
    let rows = sub.number("NROWS")?;
    let cols = sub.number("NCOLS")?;
    let bands = match sub.number("NBANDS")? {
        0 => sub.number("XBANDS")?,
        n => n,
    };
    let nbpr = sub.number("NBPR")?;
    let nbpc = sub.number("NBPC")?;
    let mut nppbh = sub.number("NPPBH")?;
    let mut nppbv = sub.number("NPPBV")?;
    // A block size of 0 with one block per row or column means the whole Image, as MIL-STD-2500C
    // allows for Images of more than 8192 pixels.
    if nbpr == 1 && nppbh == 0 {
        nppbh = cols;
    }
    if nbpc == 1 && nppbv == 0 {
        nppbv = rows;
    }
    let nbpp = sub.number("NBPP")?;
    let abpp = sub.number("ABPP")?;
    if rows == 0 || cols == 0 || bands == 0 || nbpp == 0 {
        return Err(corrupt("an Image with no rows, columns, bands or bits"));
    }
    if nbpr == 0 || nbpc == 0 || nppbh == 0 || nppbv == 0 {
        return Err(corrupt("blocks with no size, or no blocks"));
    }
    if nbpr.checked_mul(nppbh).is_none_or(|w| w < cols)
        || nbpc.checked_mul(nppbv).is_none_or(|h| h < rows)
    {
        return Err(corrupt(format!(
            "{nbpr} × {nbpc} blocks of {nppbh} × {nppbv} pixels do not cover {cols} × {rows}"
        )));
    }
    // Every later size is a part of one block's bits over every band, so this product bounds
    // them all: a header that claims blocks too large to count is corrupt, not an overflow.
    if nppbv
        .checked_mul(nppbh)
        .and_then(|p| p.checked_mul(nbpp))
        .and_then(|p| p.checked_mul(bands))
        .is_none_or(|bits| bits > u64::MAX / 8)
    {
        return Err(corrupt(format!(
            "blocks of {nppbh} × {nppbv} pixels, {bands} bands and {nbpp} bits are too large to count"
        )));
    }
    let ic = sub.text("IC");
    let imode = sub.text("IMODE").chars().next().unwrap_or(' ');
    if !matches!(imode, 'B' | 'P' | 'R' | 'S') {
        return Err(corrupt(format!("IMODE {imode:?}")));
    }
    let pvtype = sub.text("PVTYPE");
    let (pixel_type, unsupported) = match sample_type(&pvtype, nbpp as u32) {
        Ok(t) => {
            let whole = matches!(
                t,
                SampleType::F32 | SampleType::F64 | SampleType::ComplexF32
            );
            let significant = if whole || abpp == 0 || abpp > nbpp {
                nbpp
            } else {
                abpp
            };
            let shift = if sub.text("PJUST") == "L" {
                nbpp - significant
            } else {
                0
            };
            (
                Some(PixelType {
                    sample_type: t,
                    component_count: bands as u32,
                    significant_bits: significant as u32,
                    bit_shift: shift as u32,
                }),
                None,
            )
        }
        Err(why) => (None, Some(why)),
    };
    let info = ImageInfo {
        rows,
        cols,
        bands: u32::try_from(bands).map_err(|_| corrupt("too many bands"))?,
        pixel_type,
        unsupported,
        codec: codec(&ic),
        masked: ic == "NM" || ic.starts_with('M'),
        compression: ic,
        imode,
        blocks_per_row: u32::try_from(nbpr).map_err(|_| corrupt("too many blocks"))?,
        blocks_per_col: u32::try_from(nbpc).map_err(|_| corrupt("too many blocks"))?,
        block_size: (
            u32::try_from(nppbv).map_err(|_| corrupt("blocks too tall"))?,
            u32::try_from(nppbh).map_err(|_| corrupt("blocks too wide"))?,
        ),
        stored_bits: nbpp as u32,
        data,
    };
    // The one check always made (section 10.3): uncompressed data that claims more pixels than
    // the file holds is refused.
    if info.codec.is_none() && !info.masked {
        let g = Geometry::of(&info);
        if g.units.checked_mul(g.unit_bytes).is_none_or(|n| n > data.1) {
            return Err(corrupt(format!(
                "its blocks need more bytes than the {} its data holds",
                data.1
            )));
        }
    }
    Ok(info)
}

/// Reads a block mask table, checking every number against the image data's length.
fn read_mask<R: ReadAt>(reader: &R, info: &ImageInfo) -> Result<MaskTable> {
    let (data, len) = info.data;
    if len < 10 {
        return Err(corrupt("its data is too short for a mask table"));
    }
    let head = read(reader, data, 10)?;
    let imdatoff = u64::from(u32::from_be_bytes(head[0..4].try_into().unwrap()));
    let bmrlnth = u16::from_be_bytes([head[4], head[5]]);
    let tmrlnth = u16::from_be_bytes([head[6], head[7]]);
    let tpxcdlnth = u16::from_be_bytes([head[8], head[9]]);
    if !matches!(bmrlnth, 0 | 4) || !matches!(tmrlnth, 0 | 4) || tpxcdlnth > 64 {
        return Err(corrupt(format!(
            "record lengths {bmrlnth}, {tmrlnth} and {tpxcdlnth}, against 0 or 4, 0 or 4, and at most 64 bits"
        )));
    }
    let pad_bytes = u64::from(tpxcdlnth).div_ceil(8);
    let units = Geometry::of(info).units;
    let table = 10 + pad_bytes + units * (u64::from(bmrlnth) + u64::from(tmrlnth));
    if imdatoff < table || imdatoff > len {
        return Err(corrupt(format!(
            "the blocks start at {imdatoff}, against a table of {table} bytes and data of {len}"
        )));
    }
    let geometry = Geometry::of(info);
    // Without offsets, uncompressed blocks lie one after another, and must fit in the data.
    if bmrlnth == 0
        && info.codec.is_none()
        && units
            .checked_mul(geometry.unit_bytes)
            .is_none_or(|n| n > len - imdatoff)
    {
        return Err(corrupt(format!(
            "its blocks need more bytes than the {} after its mask table",
            len - imdatoff
        )));
    }
    let bytes = read(reader, data + 10, table - 10)?;
    let pad = (tpxcdlnth > 0).then(|| {
        bytes[..pad_bytes as usize]
            .iter()
            .fold(0u64, |v, &b| (v << 8) | u64::from(b))
    });
    let offsets = if bmrlnth == 4 {
        bytes[pad_bytes as usize..(pad_bytes + units * 4) as usize]
            .chunks_exact(4)
            .map(|c| match u32::from_be_bytes(c.try_into().unwrap()) {
                0xFFFF_FFFF => Ok(None),
                v if u64::from(v) >= len - imdatoff => Err(corrupt(format!(
                    "a block at {v}, past the {} bytes of blocks",
                    len - imdatoff
                ))),
                v => Ok(Some(u64::from(v))),
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    Ok(MaskTable {
        data_start: imdatoff,
        offsets,
        pad,
    })
}

/// Where each sample lies within an Image's stored blocks.
struct Geometry {
    imode: char,
    bands: u64,
    nbpp: u64,
    width: u64,
    sample_type: Option<SampleType>,
    /// Blocks in the Image.
    blocks: u64,
    /// Stored units: one for each block, or for IMODE S one for each block of each band.
    units: u64,
    /// Bytes in one unit.
    unit_bytes: u64,
    /// Bytes in one band's part of a block under IMODE B.
    band_bytes: u64,
}

impl Geometry {
    fn of(info: &ImageInfo) -> Geometry {
        let (h, w) = (u64::from(info.block_size.0), u64::from(info.block_size.1));
        let bands = u64::from(info.bands);
        let nbpp = u64::from(info.stored_bits);
        let blocks = u64::from(info.blocks_per_row) * u64::from(info.blocks_per_col);
        // Each band's part of a block, and each block, starts on a byte.
        let band_bytes = (h * w * nbpp).div_ceil(8);
        let (units, unit_bytes) = match info.imode {
            'S' => (blocks * bands, band_bytes),
            'B' => (blocks, band_bytes * bands),
            _ => (blocks, (h * w * nbpp * bands).div_ceil(8)),
        };
        Geometry {
            imode: info.imode,
            bands,
            nbpp,
            width: w,
            sample_type: info.pixel_type.map(|p| p.sample_type),
            blocks,
            units,
            unit_bytes,
            band_bytes,
        }
    }

    /// The units that hold a block, each with the bands it holds.
    fn units_of_block(&self, block: u64) -> Vec<(u64, std::ops::Range<u64>)> {
        if self.imode == 'S' {
            (0..self.bands)
                .map(|b| (b * self.blocks + block, b..b + 1))
                .collect()
        } else {
            vec![(block, 0..self.bands)]
        }
    }

    /// The block and the components a unit holds.
    fn unit_place(&self, unit: u64) -> (u64, std::ops::Range<u32>) {
        if self.imode == 'S' {
            let b = (unit / self.blocks) as u32;
            (unit % self.blocks, b..b + 1)
        } else {
            (unit, 0..self.bands as u32)
        }
    }

    /// Where, in bits from the start of its unit, the sample of band `b` at row `r` and column
    /// `c` within its block lies.
    fn bit_offset(&self, r: u64, c: u64, b: u64) -> u64 {
        let pixel = r * self.width + c;
        match self.imode {
            'S' => pixel * self.nbpp,
            'B' => b * self.band_bytes * 8 + pixel * self.nbpp,
            'P' => (pixel * self.bands + b) * self.nbpp,
            // 'R': each row holds every band's row in turn.
            _ => ((r * self.bands + b) * self.width + c) * self.nbpp,
        }
    }

    /// Copies one sample from its big-endian bits at `bit` into `out`, in native byte order.
    fn copy_sample(&self, unit: &[u8], bit: u64, out: &mut [u8]) {
        if self.nbpp.is_multiple_of(8) {
            let at = (bit / 8) as usize;
            let stored = &unit[at..at + out.len()];
            let number = self.sample_type.map_or(out.len(), SampleType::number_size);
            for (o, s) in out
                .chunks_exact_mut(number)
                .zip(stored.chunks_exact(number))
            {
                o.copy_from_slice(s);
                if cfg!(target_endian = "little") {
                    o.reverse();
                }
            }
        } else {
            // Packed samples, 1 or 12 bits, most significant bit first.
            let mut v = 0u64;
            for k in bit..bit + self.nbpp {
                let byte = unit[(k / 8) as usize];
                v = (v << 1) | u64::from((byte >> (7 - k % 8)) & 1);
            }
            let bytes = v.to_ne_bytes();
            let n = out.len();
            if cfg!(target_endian = "little") {
                out.copy_from_slice(&bytes[..n]);
            } else {
                out.copy_from_slice(&bytes[8 - n..]);
            }
        }
    }
}
