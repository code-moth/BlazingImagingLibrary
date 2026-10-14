// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! JPEG 2000 through OpenJPEG, the codec for every JPEG 2000 file until a Rust codec passes
//! BIL's tests (specification, section 10, "JPEG 2000 codecs in order").
//!
//! A codestream is read straight from the Source through OpenJPEG's stream callbacks, so nothing
//! is copied out first. OpenJPEG reads ahead in chunks of up to 1 MiB (its
//! `OPJ_J2K_STREAM_CHUNK_SIZE`) and skips what it does not need. The rules it is held to here:
//!
//! - **Its messages come back, never printed.** Errors and warnings are caught by handlers; a
//!   warning, corrupt data OpenJPEG went past, is an error (section 3.3).
//! - **The header is checked before pixel memory is taken.** `read_header` gives it to a driver
//!   when a Source is opened, and `decode` shows it to the caller's check again before decoding
//!   (sections 10 and 10.3).
//! - **Nothing is expanded after that check.** A JP2 palette, component mapping and channel
//!   definitions are not applied (section 10.3); `Header::jp2_boxes` says which are present, so
//!   the driver can apply, report or refuse them.
//! - **Decoding runs on the calling thread,** set by OpenJPEG's own call. OpenJPEG still reads
//!   its environment variable `OPJ_NUM_THREADS` when a decoder is made, and where that is set it
//!   starts threads which the call then stops before anything is decoded (M32).
//! - **Nothing unwinds into C.** A panic in a callback, such as one from a `ReadAt`, is caught
//!   and returned as an error.

use std::ffi::{c_char, c_void, CStr};
use std::fmt;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use bil_sys::openjpeg as opj;

use crate::io::ReadAt;

/// What can go wrong decoding JPEG 2000.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The codestream breaks its own rules, or OpenJPEG warned while decoding it.
    Corrupt(String),
    /// The caller's check of the header refused it.
    Refused(String),
    /// The OpenJPEG linked is one these bindings do not describe.
    Unsupported(String),
    /// The Source could not be read, or OpenJPEG could not start: no fault of the data.
    Io(io::ErrorKind, String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Corrupt(m) => write!(f, "corrupt JPEG 2000: {m}"),
            Error::Refused(m) => write!(f, "refused: {m}"),
            Error::Unsupported(m) => write!(f, "not supported: {m}"),
            Error::Io(_, m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for io::Error {
    fn from(e: Error) -> io::Error {
        match e {
            Error::Io(kind, m) => io::Error::new(kind, m),
            other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
        }
    }
}

/// The container a codestream comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// A bare codestream, as in NITF and GeoTIFF.
    Codestream,
    /// A JP2 file.
    Jp2,
}

impl Format {
    /// The format the first bytes of the data show, if either.
    pub fn sniff(first: &[u8]) -> Option<Format> {
        if first.starts_with(&[0, 0, 0, 12, b'j', b'P', b' ', b' ']) {
            Some(Format::Jp2)
        } else if first.starts_with(&[0xFF, 0x4F]) {
            Some(Format::Codestream)
        } else {
            None
        }
    }
}

/// One component as the header declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentInfo {
    /// Subsampling, in columns and rows.
    pub dx: u32,
    pub dy: u32,
    /// Bits of the value.
    pub precision: u32,
    pub signed: bool,
}

/// What a codestream's header declares: the Image's area on the reference grid, and its
/// components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    pub components: Vec<ComponentInfo>,
    /// Of a JP2 file's palette (`pclr`), component mapping (`cmap`) and channel definitions
    /// (`cdef`), those present, none of which is applied. Found by `read_header`; empty from
    /// `decode`.
    pub jp2_boxes: Vec<&'static str>,
}

impl Header {
    pub fn width(&self) -> u64 {
        u64::from(self.x1.saturating_sub(self.x0))
    }
    pub fn height(&self) -> u64 {
        u64::from(self.y1.saturating_sub(self.y0))
    }
}

/// What to decode.
#[derive(Clone, Copy, Debug, Default)]
pub struct Request {
    /// The area to decode, `(x0, y0, x1, y1)`, end exclusive, in full-resolution pixels from the
    /// Image's top-left corner wherever the codestream's grid places it; `None` for the whole
    /// Image.
    pub area: Option<(u64, u64, u64, u64)>,
    /// Resolution levels to discard: each halves the Image.
    pub reduce: u32,
}

/// Decoded components, held in OpenJPEG's own buffers and read in place: no copy is made.
pub struct Decoded {
    image: *mut opj::opj_image_t,
}

// The image is OpenJPEG's allocation, owned here alone, with no tie to the thread that made it.
unsafe impl Send for Decoded {}
unsafe impl Sync for Decoded {}

/// One decoded component.
#[derive(Clone, Copy, Debug)]
pub struct Component<'a> {
    pub width: u32,
    pub height: u32,
    pub precision: u32,
    pub signed: bool,
    /// Row by row, `width` values to a row.
    pub data: &'a [i32],
}

impl Decoded {
    pub fn component_count(&self) -> usize {
        unsafe { (*self.image).numcomps as usize }
    }

    pub fn component(&self, i: usize) -> Option<Component<'_>> {
        if i >= self.component_count() {
            return None;
        }
        let c = unsafe { &*(*self.image).comps.add(i) };
        let n = (c.w as usize).checked_mul(c.h as usize)?;
        if c.data.is_null() {
            return None;
        }
        // OpenJPEG allocates w × h values for each decoded component, and they live as long as
        // the image, which `self` owns.
        let data = unsafe { std::slice::from_raw_parts(c.data, n) };
        Some(Component {
            width: c.w,
            height: c.h,
            precision: c.prec,
            signed: c.sgnd != 0,
            data,
        })
    }
}

impl Drop for Decoded {
    fn drop(&mut self) {
        unsafe { opj::opj_image_destroy(self.image) };
    }
}

/// Fails unless the OpenJPEG linked is a 2.x release from 2.3 on, whose structures these
/// bindings describe.
fn check_version() -> Result<(), Error> {
    static CHECKED: OnceLock<Result<(), Error>> = OnceLock::new();
    CHECKED
        .get_or_init(|| {
            let v = unsafe { CStr::from_ptr(opj::opj_version()) }
                .to_string_lossy()
                .into_owned();
            let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
            match (parts.next(), parts.next()) {
                (Some(2), Some(minor)) if minor >= 3 => Ok(()),
                _ => Err(Error::Unsupported(format!(
                    "OpenJPEG {v}; 2.3 or later is needed"
                ))),
            }
        })
        .clone()
}

// ------------------------------------------------------------------ OpenJPEG's callbacks

/// The messages OpenJPEG sends while one codestream is read.
#[derive(Default)]
struct Messages {
    error: Option<String>,
    warning: Option<String>,
}

fn text(msg: *const c_char) -> String {
    if msg.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(msg) }
        .to_string_lossy()
        .trim_end()
        .to_string()
}

unsafe extern "C" fn on_error(msg: *const c_char, user: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let m = &mut *user.cast::<Messages>();
        if m.error.is_none() {
            m.error = Some(text(msg));
        }
    }));
}

unsafe extern "C" fn on_warning(msg: *const c_char, user: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let m = &mut *user.cast::<Messages>();
        if m.warning.is_none() {
            m.warning = Some(text(msg));
        }
    }));
}

unsafe extern "C" fn on_info(_msg: *const c_char, _user: *mut c_void) {}

/// The codestream's bytes in the Source, read at the place OpenJPEG asks for. `base + len` is
/// checked when the stream is made, so no position here can overflow.
struct Stream<'a> {
    source: &'a dyn ReadAt,
    base: u64,
    len: u64,
    pos: u64,
    failed: Option<(io::ErrorKind, String)>,
}

unsafe extern "C" fn read(buffer: *mut c_void, n: usize, user: *mut c_void) -> usize {
    catch_unwind(AssertUnwindSafe(|| {
        let s = &mut *user.cast::<Stream>();
        if s.pos >= s.len {
            return usize::MAX;
        }
        let k = (s.len - s.pos).min(n as u64) as usize;
        let buf = std::slice::from_raw_parts_mut(buffer.cast::<u8>(), k);
        match s.source.read_exact_at(buf, s.base + s.pos) {
            Ok(()) => {
                s.pos += k as u64;
                k
            }
            Err(e) => {
                s.failed.get_or_insert((e.kind(), e.to_string()));
                usize::MAX
            }
        }
    }))
    .unwrap_or_else(|_| {
        let s = &mut *user.cast::<Stream>();
        s.failed
            .get_or_insert((io::ErrorKind::Other, "reading the Source panicked".into()));
        usize::MAX
    })
}

unsafe extern "C" fn skip(n: i64, user: *mut c_void) -> i64 {
    let s = &mut *user.cast::<Stream>();
    // OpenJPEG skips in a loop until the count is done or -1 comes back, so a skip that cannot
    // move reports -1 rather than 0.
    if (n > 0 && s.pos >= s.len) || (n < 0 && s.pos == 0) {
        return -1;
    }
    let moved = if n < 0 {
        -(n.unsigned_abs().min(s.pos) as i64)
    } else {
        (n as u64).min(s.len - s.pos) as i64
    };
    s.pos = s.pos.wrapping_add_signed(moved);
    moved
}

unsafe extern "C" fn seek(n: i64, user: *mut c_void) -> opj::OPJ_BOOL {
    let s = &mut *user.cast::<Stream>();
    if n < 0 || n as u64 > s.len {
        return 0;
    }
    s.pos = n as u64;
    1
}

// ------------------------------------------------------------------ one reading

/// One codestream being read: the codec, the stream and the image OpenJPEG makes, with the
/// messages and the stream's state they point to.
struct Session<'a> {
    // Declared first, so dropped first: the codec, the stream and the image are destroyed while
    // the messages and the stream's state their callbacks point to still exist.
    handles: Handles,
    messages: Box<Messages>,
    stream: Box<Stream<'a>>,
}

/// Frees the codec, the stream and, unless handed on, the image, whatever happens.
struct Handles {
    codec: *mut opj::opj_codec_t,
    stream: *mut opj::opj_stream_t,
    image: *mut opj::opj_image_t,
}

impl Drop for Handles {
    fn drop(&mut self) {
        unsafe {
            if !self.image.is_null() {
                opj::opj_image_destroy(self.image);
            }
            if !self.stream.is_null() {
                opj::opj_stream_destroy(self.stream);
            }
            if !self.codec.is_null() {
                opj::opj_destroy_codec(self.codec);
            }
        }
    }
}

impl<'a> Session<'a> {
    /// The error for a step that failed: the Source's failure first, then OpenJPEG's message.
    fn failure(&self, what: &str) -> Error {
        if let Some((kind, m)) = &self.stream.failed {
            return Error::Io(*kind, m.clone());
        }
        Error::Corrupt(
            self.messages
                .error
                .clone()
                .unwrap_or_else(|| format!("{what} failed")),
        )
    }

    /// Starts a decoder on the bytes at `offset`, `length` long, and reads the header.
    fn open(
        source: &'a dyn ReadAt,
        offset: u64,
        length: u64,
        format: Format,
        reduce: u32,
        jp2_boxes: bool,
    ) -> Result<(Session<'a>, Header), Error> {
        check_version()?;
        if offset.checked_add(length).is_none() || i64::try_from(length).is_err() {
            return Err(Error::Refused(format!(
                "{length} bytes at {offset}, past what can be counted"
            )));
        }
        let mut s = Session {
            handles: Handles {
                codec: unsafe {
                    opj::opj_create_decompress(match format {
                        Format::Codestream => opj::OPJ_CODEC_J2K,
                        Format::Jp2 => opj::OPJ_CODEC_JP2,
                    })
                },
                stream: unsafe { opj::opj_stream_create(opj::OPJ_J2K_STREAM_CHUNK_SIZE, 1) },
                image: std::ptr::null_mut(),
            },
            messages: Box::default(),
            stream: Box::new(Stream {
                source,
                base: offset,
                len: length,
                pos: 0,
                failed: None,
            }),
        };
        let (codec, stream) = (s.handles.codec, s.handles.stream);
        if codec.is_null() || stream.is_null() {
            return Err(Error::Io(
                io::ErrorKind::OutOfMemory,
                "OpenJPEG could not start a decoder".into(),
            ));
        }
        unsafe {
            // The boxes do not move while the session lives, so these pointers stay good.
            let user = (&mut *s.messages as *mut Messages).cast::<c_void>();
            opj::opj_set_error_handler(codec, Some(on_error), user);
            opj::opj_set_warning_handler(codec, Some(on_warning), user);
            opj::opj_set_info_handler(codec, Some(on_info), user);
            let state = (&mut *s.stream as *mut Stream).cast::<c_void>();
            opj::opj_stream_set_user_data(stream, state, None);
            opj::opj_stream_set_user_data_length(stream, length);
            opj::opj_stream_set_read_function(stream, Some(read));
            opj::opj_stream_set_skip_function(stream, Some(skip));
            opj::opj_stream_set_seek_function(stream, Some(seek));
            let mut params = std::mem::MaybeUninit::<opj::opj_dparameters_t>::zeroed();
            opj::opj_set_default_decoder_parameters(params.as_mut_ptr());
            let params = params.as_mut_ptr();
            (*params).flags |= opj::OPJ_DPARAMETERS_IGNORE_PCLR_CMAP_CDEF_FLAG;
            (*params).cp_reduce = reduce;
            if opj::opj_setup_decoder(codec, params) == 0 {
                return Err(s.failure("setting up the decoder"));
            }
            // 0: the calling thread only, whatever OPJ_NUM_THREADS says. Only a build of
            // OpenJPEG without threads refuses, and it has only the calling thread anyway.
            let _ = opj::opj_codec_set_threads(codec, 0);
            if opj::opj_read_header(stream, codec, &mut s.handles.image) == 0
                || s.handles.image.is_null()
            {
                return Err(s.failure("reading the header"));
            }
        }
        let img = unsafe { &*s.handles.image };
        let comps = if img.numcomps == 0 || img.comps.is_null() {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(img.comps, img.numcomps as usize) }
        };
        let mut header = Header {
            x0: img.x0,
            y0: img.y0,
            x1: img.x1,
            y1: img.y1,
            components: comps
                .iter()
                .map(|c| ComponentInfo {
                    dx: c.dx,
                    dy: c.dy,
                    precision: c.prec,
                    signed: c.sgnd != 0,
                })
                .collect(),
            jp2_boxes: Vec::new(),
        };
        if header.components.is_empty() || header.width() == 0 || header.height() == 0 {
            return Err(Error::Corrupt(
                "a header with no components or no area".into(),
            ));
        }
        if let Some(w) = &s.messages.warning {
            return Err(Error::Corrupt(format!("OpenJPEG warned: {w}")));
        }
        if format == Format::Jp2 && jp2_boxes {
            header.jp2_boxes = jp2_header_boxes(source, offset, length)?;
        }
        Ok((s, header))
    }
}

/// Which of `pclr`, `cmap` and `cdef` a JP2 file's header box holds. OpenJPEG has read the file
/// already, so its boxes are known to be whole; any that are not are left to OpenJPEG.
fn jp2_header_boxes(
    source: &dyn ReadAt,
    offset: u64,
    length: u64,
) -> Result<Vec<&'static str>, Error> {
    let read_box = |at: u64, end: u64| -> Result<Option<([u8; 4], u64, u64)>, Error> {
        if end.saturating_sub(at) < 8 {
            return Ok(None);
        }
        let mut b = [0u8; 16];
        let k = if end - at >= 16 { 16 } else { 8 };
        source
            .read_exact_at(&mut b[..k], offset + at)
            .map_err(|e| Error::Io(e.kind(), e.to_string()))?;
        let size = u64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        let kind = [b[4], b[5], b[6], b[7]];
        let (body, len) = match size {
            0 => (at + 8, end - at),
            1 if k == 16 => (at + 16, u64::from_be_bytes(b[8..16].try_into().unwrap())),
            n => (at + 8, n),
        };
        if len < body - at || at.checked_add(len).is_none_or(|e| e > end) {
            return Ok(None);
        }
        Ok(Some((kind, body, at + len)))
    };
    let mut found = Vec::new();
    let mut at = 0;
    while let Some((kind, body, next)) = read_box(at, length)? {
        if &kind == b"jp2h" {
            let mut inner = body;
            while let Some((kind, _, next)) = read_box(inner, next)? {
                for name in ["pclr", "cmap", "cdef"] {
                    if kind == *name.as_bytes() && !found.contains(&name) {
                        found.push(name);
                    }
                }
                inner = next;
            }
            break;
        }
        at = next;
    }
    Ok(found)
}

/// Reads the header of the codestream or JP2 file at `offset`, `length` bytes long, in
/// `source`, without decoding: what a driver checks when a Source is opened.
pub fn read_header(
    source: &dyn ReadAt,
    offset: u64,
    length: u64,
    format: Format,
) -> Result<Header, Error> {
    Session::open(source, offset, length, format, 0, true).map(|(_, header)| header)
}

/// Decodes the codestream at `offset`, `length` bytes long, in `source`. `check` sees the header
/// before any pixel memory is taken, and may refuse it with a reason.
pub fn decode(
    source: &dyn ReadAt,
    offset: u64,
    length: u64,
    format: Format,
    request: Request,
    check: &dyn Fn(&Header) -> Result<(), String>,
) -> Result<(Header, Decoded), Error> {
    // The JP2 boxes were found by `read_header` when the Source was opened, and are not sought
    // again for every read.
    let (mut s, header) = Session::open(source, offset, length, format, request.reduce, false)?;
    check(&header).map_err(Error::Refused)?;
    let (codec, stream) = (s.handles.codec, s.handles.stream);
    unsafe {
        if let Some((x0, y0, x1, y1)) = request.area {
            if x0 >= x1 || y0 >= y1 || x1 > header.width() || y1 > header.height() {
                return Err(Error::Refused(format!(
                    "an area {x0},{y0} to {x1},{y1} outside the Image's {} × {} pixels",
                    header.width(),
                    header.height()
                )));
            }
            // On the reference grid; OpenJPEG takes it as signed 32-bit numbers.
            let grid = |v: u64, o: u32| i32::try_from(v + u64::from(o)).ok();
            let (Some(gx0), Some(gy0), Some(gx1), Some(gy1)) = (
                grid(x0, header.x0),
                grid(y0, header.y0),
                grid(x1, header.x0),
                grid(y1, header.y0),
            ) else {
                return Err(Error::Unsupported(
                    "an area past 2^31 on the reference grid".into(),
                ));
            };
            if opj::opj_set_decode_area(codec, s.handles.image, gx0, gy0, gx1, gy1) == 0 {
                return Err(s.failure("setting the area"));
            }
        }
        if opj::opj_decode(codec, stream, s.handles.image) == 0
            || opj::opj_end_decompress(codec, stream) == 0
        {
            return Err(s.failure("decoding"));
        }
    }
    if let Some((kind, m)) = &s.stream.failed {
        return Err(Error::Io(*kind, m.clone()));
    }
    if let Some(w) = &s.messages.warning {
        return Err(Error::Corrupt(format!("OpenJPEG warned: {w}")));
    }
    if let Some(e) = &s.messages.error {
        return Err(Error::Corrupt(e.clone()));
    }
    let image = std::mem::replace(&mut s.handles.image, std::ptr::null_mut());
    let decoded = Decoded { image };
    for i in 0..decoded.component_count() {
        if decoded.component(i).is_none() {
            return Err(Error::Corrupt(format!("component {i} was not decoded")));
        }
    }
    Ok((header, decoded))
}
