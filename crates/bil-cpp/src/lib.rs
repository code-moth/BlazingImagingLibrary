// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The binary layer beneath the C++ header `bil/bil.hpp` (specification, section 2): plain
//! functions on opaque handles, which the shared library exports and the header wraps. It is
//! BIL's own, versioned by `bil_abi_version`, and not an interface for callers.
//!
//! Every function that can fail returns an error handle, null on success, and writes its value
//! through its last argument. No panic crosses the boundary: in a function that can fail it is
//! caught and returned as an error, and in one that cannot, caught and dropped. The header's
//! text sinks do not throw, so no C++ exception crosses it either.

#![allow(clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, RwLock};

use bil_core::types::{
    ColorModel, FrameCount, Interleave, LabEncoding, RelationKind, Representation, SampleType,
    Scope, Status, StorageOrder, Window,
};
use bil_core::{Error, OpenOptions, Source};

/// The version of this layer, which the header checks when it first calls the library: the major
/// version of the plugin interface it carries (specification, sections 2 and 10), 1 until
/// external plugins come. It goes up with every incompatible change.
pub const ABI_VERSION: u32 = 1;

#[no_mangle]
pub extern "C" fn bil_abi_version() -> u32 {
    ABI_VERSION
}

/// Where text goes: the header passes a function that appends to its string.
pub type Sink = extern "C" fn(ctx: *mut c_void, text: *const u8, len: usize);

fn put(sink: Sink, ctx: *mut c_void, text: &str) {
    sink(ctx, text.as_ptr(), text.len());
}

/// Text the header passes: a path or a level name, which must be UTF-8.
unsafe fn text<'a>(ptr: *const u8, len: usize, what: &str) -> Result<&'a str, AbiError> {
    if len == 0 {
        return Ok("");
    }
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len))
        .map_err(|_| Error::Unsupported(format!("{what} that is not UTF-8")).into())
}

/// Runs `f`, which cannot fail but for a bug, so that not even a panic crosses the boundary.
fn quiet(f: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}

// ------------------------------------------------------------------ errors

/// A failure, boxed: one of BIL's errors, or a call on a closed Source, which is the caller's
/// misuse and no kind of BIL's (specification, section 3.3); the header throws its own
/// `std::logic_error` for it.
pub enum AbiError {
    Bil(Error),
    Closed,
}

impl From<Error> for AbiError {
    fn from(e: Error) -> AbiError {
        AbiError::Bil(e)
    }
}

fn fail(e: AbiError) -> *mut AbiError {
    Box::into_raw(Box::new(e))
}

/// Runs `f`, turning its error or a panic into an error handle.
fn guard(f: impl FnOnce() -> Result<(), AbiError>) -> *mut AbiError {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => std::ptr::null_mut(),
        Ok(Err(e)) => fail(e),
        Err(panic) => {
            let why = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            fail(AbiError::Bil(Error::Internal(format!("a panic: {why}"))))
        }
    }
}

/// The error's kind: 0 a closed Source, 1 `Unrecognized`, 2 `Corrupt`, 3 `Unsupported`,
/// 4 `NotFound`, 5 `Io`, 6 `Internal`.
#[no_mangle]
pub unsafe extern "C" fn bil_abi_error_kind(e: *const AbiError) -> u32 {
    match &*e {
        AbiError::Closed => 0,
        AbiError::Bil(Error::Unrecognized(_)) => 1,
        AbiError::Bil(Error::Corrupt(_)) => 2,
        AbiError::Bil(Error::Unsupported(_)) => 3,
        AbiError::Bil(Error::NotFound(_)) => 4,
        AbiError::Bil(Error::Io(_)) => 5,
        AbiError::Bil(Error::Internal(_)) => 6,
    }
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_error_message(e: *const AbiError, sink: Sink, ctx: *mut c_void) {
    quiet(|| {
        put(
            sink,
            ctx,
            &match &*e {
                AbiError::Bil(e) => e.to_string(),
                AbiError::Closed => "the Source is closed".to_string(),
            },
        );
    });
}

/// The drivers an `Unrecognized` error tried, one call of `sink` each.
#[no_mangle]
pub unsafe extern "C" fn bil_abi_error_tried(e: *const AbiError, sink: Sink, ctx: *mut c_void) {
    quiet(|| {
        if let AbiError::Bil(Error::Unrecognized(tried)) = &*e {
            for t in tried {
                put(sink, ctx, t);
            }
        }
    });
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_error_free(e: *mut AbiError) {
    if !e.is_null() {
        drop(Box::from_raw(e));
    }
}

// ------------------------------------------------------------------ options and opening

/// `OpenOptions`, size first, so a later version can add fields.
#[repr(C)]
pub struct AbiOpenOptions {
    pub size: usize,
    pub max_directories: u64,
    pub max_subdirectories: u64,
    pub max_held_warnings: u64,
}

/// Fills `out` with the defaults, which the library alone states.
#[no_mangle]
pub unsafe extern "C" fn bil_abi_open_options_default(out: *mut AbiOpenOptions) {
    quiet(|| {
        let d = OpenOptions::default();
        (*out).max_directories = d.max_directories;
        (*out).max_subdirectories = d.max_subdirectories;
        (*out).max_held_warnings = d.max_held_warnings as u64;
    });
}

unsafe fn options(o: *const AbiOpenOptions) -> Result<OpenOptions, AbiError> {
    if (*o).size < std::mem::size_of::<AbiOpenOptions>() {
        return Err(Error::Unsupported(
            "OpenOptions from an older header than this library's".into(),
        )
        .into());
    }
    Ok(OpenOptions {
        max_directories: (*o).max_directories,
        max_subdirectories: (*o).max_subdirectories,
        max_held_warnings: usize::try_from((*o).max_held_warnings).unwrap_or(usize::MAX),
    })
}

/// A Source, shared by every copy the header holds; `close` empties it for all of them.
pub struct AbiSource(RwLock<Option<Arc<Source>>>);

fn handle(source: Source) -> *const AbiSource {
    Arc::into_raw(Arc::new(AbiSource(RwLock::new(Some(Arc::new(source))))))
}

unsafe fn source(h: *const AbiSource) -> Result<Arc<Source>, AbiError> {
    let lock = (*h).0.read().unwrap_or_else(|p| p.into_inner());
    lock.clone().ok_or(AbiError::Closed)
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_open_path(
    o: *const AbiOpenOptions,
    path: *const u8,
    len: usize,
    out: *mut *const AbiSource,
) -> *mut AbiError {
    guard(|| {
        let s = options(o)?.open(text(path, len, "a path")?)?;
        *out = handle(s);
        Ok(())
    })
}

/// Opens a copy of the bytes given.
#[no_mangle]
pub unsafe extern "C" fn bil_abi_open_bytes(
    o: *const AbiOpenOptions,
    bytes: *const u8,
    len: usize,
    out: *mut *const AbiSource,
) -> *mut AbiError {
    guard(|| {
        let copy = if len == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(bytes, len).to_vec()
        };
        let s = options(o)?.open_from(copy)?;
        *out = handle(s);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_source_release(h: *const AbiSource) {
    Arc::decrement_strong_count(h);
}

/// Closes the Source for every copy: what it holds is freed once no read is using it.
#[no_mangle]
pub unsafe extern "C" fn bil_abi_source_close(h: *const AbiSource) {
    let mut lock = (*h).0.write().unwrap_or_else(|p| p.into_inner());
    *lock = None;
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_source_driver(
    h: *const AbiSource,
    sink: Sink,
    ctx: *mut c_void,
) -> *mut AbiError {
    guard(|| {
        put(sink, ctx, source(h)?.driver());
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_source_image_count(
    h: *const AbiSource,
    out: *mut usize,
) -> *mut AbiError {
    guard(|| {
        *out = source(h)?.image_count();
        Ok(())
    })
}

// ------------------------------------------------------------------ the cache

#[repr(C)]
pub struct AbiCacheReport {
    pub fragment_count: usize,
    pub bytes: u64,
    pub max_fragments: usize,
    pub max_bytes: u64,
    pub hits: u64,
    pub misses: u64,
    pub served_from_read_ahead: u64,
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_set_cache_size(h: *const AbiSource, n: usize) -> *mut AbiError {
    guard(|| {
        source(h)?.set_cache_size(n);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_set_cache_bytes(h: *const AbiSource, n: u64) -> *mut AbiError {
    guard(|| {
        source(h)?.set_cache_bytes(n);
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_clear_cache(h: *const AbiSource) -> *mut AbiError {
    guard(|| {
        source(h)?.clear_cache();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_describe_cache(
    h: *const AbiSource,
    out: *mut AbiCacheReport,
) -> *mut AbiError {
    guard(|| {
        let r = source(h)?.describe_cache();
        *out = AbiCacheReport {
            fragment_count: r.fragment_count,
            bytes: r.bytes,
            max_fragments: r.max_fragments,
            max_bytes: r.max_bytes,
            hits: r.hits,
            misses: r.misses,
            served_from_read_ahead: r.served_from_read_ahead,
        };
        Ok(())
    })
}

// ------------------------------------------------------------------ warnings

/// A drained set of warnings, read by index and freed once.
pub struct AbiWarnings(bil_core::warning::Warnings);

/// One warning's fields, but its message and code name, which come through a `Sink`.
#[repr(C)]
pub struct AbiWarning {
    /// 1 `cache_ceiling_exceeded`, 2 `subdirectory_skipped`, 3 `corrupt_block_missing`,
    /// 4 `badly_packed`.
    pub code: u32,
    /// The Image, or -1 for the Source.
    pub image: i64,
    pub has_detail: u8,
    pub detail: u64,
    pub count: u64,
    pub block_count: usize,
}

#[repr(C)]
pub struct AbiBlockKey {
    pub image: u32,
    pub level: u32,
    pub frame: u64,
    pub block_row: u32,
    pub block_col: u32,
    pub components: [u32; 2],
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_take_warnings(
    h: *const AbiSource,
    out: *mut *mut AbiWarnings,
) -> *mut AbiError {
    guard(|| {
        *out = Box::into_raw(Box::new(AbiWarnings(source(h)?.take_warnings())));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_warnings_count(w: *const AbiWarnings) -> usize {
    (&*w).0.warnings.len()
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_warnings_dropped(w: *const AbiWarnings) -> u64 {
    (&*w).0.dropped
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_warning_get(
    w: *const AbiWarnings,
    i: usize,
    out: *mut AbiWarning,
) {
    quiet(|| {
        use bil_core::warning::WarningCode as C;
        let x = &(&*w).0.warnings[i];
        *out = AbiWarning {
            code: match x.code {
                C::CacheCeilingExceeded => 1,
                C::SubdirectorySkipped => 2,
                C::CorruptBlockMissing => 3,
                C::BadlyPacked => 4,
            },
            image: match x.scope {
                Scope::Source => -1,
                Scope::Image(i) => i as i64,
            },
            has_detail: u8::from(x.detail.is_some()),
            detail: x.detail.unwrap_or(0),
            count: x.count,
            block_count: x.blocks.len(),
        };
    });
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_warning_block(
    w: *const AbiWarnings,
    i: usize,
    k: usize,
    out: *mut AbiBlockKey,
) {
    quiet(|| {
        let b = &(&*w).0.warnings[i].blocks[k];
        *out = AbiBlockKey {
            image: b.image,
            level: b.level,
            frame: b.frame,
            block_row: b.block_row,
            block_col: b.block_col,
            components: [b.components.0, b.components.1],
        };
    });
}

/// The warning's message, built now (specification, section 3.3).
#[no_mangle]
pub unsafe extern "C" fn bil_abi_warning_message(
    w: *const AbiWarnings,
    i: usize,
    sink: Sink,
    ctx: *mut c_void,
) {
    quiet(|| {
        put(sink, ctx, &(&*w).0.warnings[i].to_string());
    });
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_warnings_free(w: *mut AbiWarnings) {
    if !w.is_null() {
        drop(Box::from_raw(w));
    }
}

// ------------------------------------------------------------------ Images

fn sample_code(t: SampleType) -> u32 {
    match t {
        SampleType::U8 => 0,
        SampleType::I8 => 1,
        SampleType::U16 => 2,
        SampleType::I16 => 3,
        SampleType::U32 => 4,
        SampleType::I32 => 5,
        SampleType::U64 => 6,
        SampleType::I64 => 7,
        SampleType::F16 => 8,
        SampleType::F32 => 9,
        SampleType::F64 => 10,
        SampleType::ComplexI8 => 11,
        SampleType::ComplexI16 => 12,
        SampleType::ComplexF32 => 13,
        SampleType::ComplexF64 => 14,
    }
}

#[repr(C)]
pub struct AbiPixelType {
    pub sample_type: u32,
    pub component_count: u32,
    pub significant_bits: u32,
    pub bit_shift: u32,
}

/// `ImageInfo`, with component names and relations read by index.
#[repr(C)]
pub struct AbiImageInfo {
    pub row_count: u64,
    pub col_count: u64,
    /// 0 `Known`, 1 `AtLeast`, 2 `Unknown`.
    pub frame_kind: u32,
    pub frame_count: u64,
    /// 0 `Intensity`, 1 `Color`, 2 `Palette`, 3 `Complex`, 4 `Spectral`, 5 `Vector`.
    pub representation: u32,
    pub white_is_zero: u8,
    /// 0 `Rgb`, 1 `YCbCr`, 2 `Lab`, 3 `Cmyk`, for `Color`.
    pub color_model: u32,
    pub coefficients: [f64; 3],
    /// 0 `Cie`, 1 `Icc`, 2 `Itu`, for `Lab`.
    pub lab_encoding: u32,
    pub has_pixel_type: u8,
    pub pixel_type: AbiPixelType,
    pub component_count: u32,
    pub name_count: usize,
    pub relation_count: usize,
    pub block_size: [u32; 2],
    /// 0 `Pixel`, 1 `Line`, 2 `Band`.
    pub interleave: u32,
    /// 0 `Known`, 1 `Unknown`.
    pub order: u32,
    pub may_have_missing: u8,
}

fn interleave(i: Interleave) -> u32 {
    match i {
        Interleave::Pixel => 0,
        Interleave::Line => 1,
        Interleave::Band => 2,
    }
}

fn order(o: StorageOrder) -> u32 {
    match o {
        StorageOrder::Known => 0,
        StorageOrder::Unknown => 1,
    }
}

unsafe fn image_info(
    h: *const AbiSource,
    i: usize,
) -> Result<bil_core::types::ImageInfo, AbiError> {
    Ok(source(h)?.image(i)?.info()?)
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_image_info_get(
    h: *const AbiSource,
    i: usize,
    out: *mut AbiImageInfo,
) -> *mut AbiError {
    guard(|| {
        let info = image_info(h, i)?;
        let (frame_kind, frame_count) = match info.frame_count {
            FrameCount::Known(n) => (0, n),
            FrameCount::AtLeast(n) => (1, n),
            FrameCount::Unknown => (2, 0),
        };
        let (mut white, mut model, mut coefficients, mut lab) = (0, 0, [0.0; 3], 0);
        let representation = match &info.representation {
            Representation::Intensity { white_is_zero } => {
                white = u8::from(*white_is_zero);
                0
            }
            Representation::Color(m) => {
                match m {
                    ColorModel::Rgb => model = 0,
                    ColorModel::YCbCr { coefficients: c } => {
                        model = 1;
                        coefficients = [c.0, c.1, c.2];
                    }
                    ColorModel::Lab { encoding } => {
                        model = 2;
                        lab = match encoding {
                            LabEncoding::Cie => 0,
                            LabEncoding::Icc => 1,
                            LabEncoding::Itu => 2,
                        };
                    }
                    ColorModel::Cmyk => model = 3,
                }
                1
            }
            Representation::Palette => 2,
            Representation::Complex => 3,
            Representation::Spectral => 4,
            Representation::Vector => 5,
        };
        let p = info.pixel_type;
        *out = AbiImageInfo {
            row_count: info.row_count,
            col_count: info.col_count,
            frame_kind,
            frame_count,
            representation,
            white_is_zero: white,
            color_model: model,
            coefficients,
            lab_encoding: lab,
            has_pixel_type: u8::from(p.is_some()),
            pixel_type: AbiPixelType {
                sample_type: p.map_or(0, |p| sample_code(p.sample_type)),
                component_count: p.map_or(0, |p| p.component_count),
                significant_bits: p.map_or(0, |p| p.significant_bits),
                bit_shift: p.map_or(0, |p| p.bit_shift),
            },
            component_count: info.component_count,
            name_count: info.component_names.len(),
            relation_count: info.relations.len(),
            block_size: [info.storage.block_size.0, info.storage.block_size.1],
            interleave: interleave(info.storage.interleave),
            order: order(info.storage.order),
            may_have_missing: u8::from(info.may_have_missing),
        };
        Ok(())
    })
}

/// Every component name, in order, one call of `sink` each.
#[no_mangle]
pub unsafe extern "C" fn bil_abi_image_component_names(
    h: *const AbiSource,
    i: usize,
    sink: Sink,
    ctx: *mut c_void,
) -> *mut AbiError {
    guard(|| {
        for n in &image_info(h, i)?.component_names {
            put(sink, ctx, n);
        }
        Ok(())
    })
}

#[repr(C)]
pub struct AbiRelation {
    pub image: usize,
    /// 0 `SameGrid`, 1 `ScaledGrid`.
    pub kind: u32,
    pub reduction: [u32; 2],
    pub position: [f64; 2],
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_image_relation(
    h: *const AbiSource,
    i: usize,
    k: usize,
    out: *mut AbiRelation,
) -> *mut AbiError {
    guard(|| {
        let info = image_info(h, i)?;
        let r = info
            .relations
            .get(k)
            .ok_or_else(|| Error::NotFound(format!("relation {k} of image {i}")))?;
        *out = match r.kind {
            RelationKind::SameGrid => AbiRelation {
                image: r.image,
                kind: 0,
                reduction: [1, 1],
                position: [0.0; 2],
            },
            RelationKind::ScaledGrid {
                reduction,
                position,
            } => AbiRelation {
                image: r.image,
                kind: 1,
                reduction: [reduction.0, reduction.1],
                position: [position.0, position.1],
            },
        };
        Ok(())
    })
}

#[repr(C)]
pub struct AbiStoredLevel {
    pub rows: u64,
    pub cols: u64,
    pub scale: [f64; 2],
    pub position: [f64; 2],
    /// 1 aligned, 0 not, -1 unknown.
    pub aligned: i8,
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_stored_level_count(
    h: *const AbiSource,
    i: usize,
    out: *mut usize,
) -> *mut AbiError {
    guard(|| {
        *out = source(h)?.image(i)?.list_stored_levels()?.len();
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_stored_level_get(
    h: *const AbiSource,
    i: usize,
    k: usize,
    out: *mut AbiStoredLevel,
    sink: Sink,
    ctx: *mut c_void,
) -> *mut AbiError {
    guard(|| {
        let s = source(h)?;
        let levels = s.image(i)?.list_stored_levels()?;
        let l = levels
            .get(k)
            .ok_or_else(|| Error::NotFound(format!("stored level {k} of image {i}")))?;
        *out = AbiStoredLevel {
            rows: l.rows,
            cols: l.cols,
            scale: [l.scale.0, l.scale.1],
            position: [l.position.0, l.position.1],
            aligned: l.aligned.map_or(-1, i8::from),
        };
        put(sink, ctx, &l.name);
        Ok(())
    })
}

// ------------------------------------------------------------------ storage

pub struct AbiStorage(bil_core::types::Storage);

#[repr(C)]
pub struct AbiStorageHead {
    pub block_size: [u32; 2],
    pub interleave: u32,
    pub order: u32,
    pub block_count: usize,
}

#[repr(C)]
pub struct AbiBlock {
    pub image: u32,
    pub level: u32,
    pub frame: u64,
    pub block_row: u32,
    pub block_col: u32,
    pub components: [u32; 2],
    pub range_count: usize,
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_describe_storage(
    h: *const AbiSource,
    i: usize,
    level: *const u8,
    len: usize,
    out: *mut *mut AbiStorage,
) -> *mut AbiError {
    guard(|| {
        let s = source(h)?
            .image(i)?
            .describe_storage(text(level, len, "a level name")?)?;
        *out = Box::into_raw(Box::new(AbiStorage(s)));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_storage_head_get(s: *const AbiStorage, out: *mut AbiStorageHead) {
    quiet(|| {
        let s = &(&*s).0;
        *out = AbiStorageHead {
            block_size: [s.block_size.0, s.block_size.1],
            interleave: interleave(s.interleave),
            order: order(s.order),
            block_count: s.blocks.len(),
        };
    });
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_storage_block(s: *const AbiStorage, k: usize, out: *mut AbiBlock) {
    quiet(|| {
        let b = &(&*s).0.blocks[k];
        *out = AbiBlock {
            image: b.image,
            level: b.level,
            frame: b.frame,
            block_row: b.block_row,
            block_col: b.block_col,
            components: [b.components.start, b.components.end],
            range_count: b.ranges.len(),
        };
    });
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_storage_block_range(
    s: *const AbiStorage,
    k: usize,
    j: usize,
    out: *mut [u64; 2],
) {
    quiet(|| {
        let r = (&*s).0.blocks[k].ranges[j];
        *out = [r.offset, r.length];
    });
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_storage_free(s: *mut AbiStorage) {
    if !s.is_null() {
        drop(Box::from_raw(s));
    }
}

// ------------------------------------------------------------------ Fragments

pub struct AbiFragment(bil_core::types::Fragment);

#[repr(C)]
pub struct AbiWindow {
    pub row_start: u64,
    pub col_start: u64,
    pub row_count: u32,
    pub col_count: u32,
}

/// A Fragment's fields, its data and validity as pointers valid while the handle lives.
#[repr(C)]
pub struct AbiFragmentView {
    pub window: AbiWindow,
    pub pixel_type: AbiPixelType,
    pub strides: [usize; 3],
    pub data: *const u8,
    pub data_len: usize,
    /// Null where every pixel was recorded.
    pub validity: *const bool,
    pub validity_len: usize,
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_read_fragment(
    h: *const AbiSource,
    i: usize,
    w: *const AbiWindow,
    level: *const u8,
    len: usize,
    out: *mut *mut AbiFragment,
) -> *mut AbiError {
    guard(|| {
        let window = Window {
            row_start: (*w).row_start,
            col_start: (*w).col_start,
            row_count: (*w).row_count,
            col_count: (*w).col_count,
        };
        let f = source(h)?
            .image(i)?
            .read_fragment(window, text(level, len, "a level name")?)?;
        *out = Box::into_raw(Box::new(AbiFragment(f)));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_fragment_view_get(
    f: *const AbiFragment,
    out: *mut AbiFragmentView,
) {
    quiet(|| {
        let f = &(&*f).0;
        let p = f.pixel_type;
        *out = AbiFragmentView {
            window: AbiWindow {
                row_start: f.window.row_start,
                col_start: f.window.col_start,
                row_count: f.window.row_count,
                col_count: f.window.col_count,
            },
            pixel_type: AbiPixelType {
                sample_type: sample_code(p.sample_type),
                component_count: p.component_count,
                significant_bits: p.significant_bits,
                bit_shift: p.bit_shift,
            },
            strides: f.strides,
            data: f.data.as_ptr(),
            data_len: f.data.len(),
            validity: f.validity.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            validity_len: f.validity.as_ref().map_or(0, |v| v.len()),
        };
    });
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_fragment_free(f: *mut AbiFragment) {
    if !f.is_null() {
        drop(Box::from_raw(f));
    }
}

// ------------------------------------------------------------------ Slices

pub struct AbiSlice(bil_core::types::Slice);

/// A Slice's fields, its text and values as pointers valid while the handle lives.
#[repr(C)]
pub struct AbiSliceView {
    pub name: *const u8,
    pub name_len: usize,
    pub label: *const u8,
    pub label_len: usize,
    pub level_name: *const u8,
    pub level_name_len: usize,
    pub status: u32,
    pub origin: [f64; 2],
    pub scale: [f64; 2],
    pub window: AbiWindow,
    pub row_count: u32,
    pub col_count: u32,
    pub data: *const f32,
    pub data_len: usize,
    pub strides: [isize; 2],
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_read_slice(
    h: *const AbiSource,
    i: usize,
    name: *const u8,
    name_len: usize,
    w: *const AbiWindow,
    level: *const u8,
    len: usize,
    out: *mut *mut AbiSlice,
) -> *mut AbiError {
    guard(|| {
        let window = Window {
            row_start: (*w).row_start,
            col_start: (*w).col_start,
            row_count: (*w).row_count,
            col_count: (*w).col_count,
        };
        let s = source(h)?.image(i)?.read_slice(
            text(name, name_len, "a Slice name")?,
            window,
            text(level, len, "a level name")?,
        )?;
        *out = Box::into_raw(Box::new(AbiSlice(s)));
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_slice_view_get(s: *const AbiSlice, out: *mut AbiSliceView) {
    quiet(|| {
        let s = &(&*s).0;
        *out = AbiSliceView {
            name: s.name.as_ptr(),
            name_len: s.name.len(),
            label: s.label.as_ptr(),
            label_len: s.label.len(),
            level_name: s.level_name.as_ptr(),
            level_name_len: s.level_name.len(),
            status: match s.status {
                Status::Complete => 0,
                Status::Partial => 1,
                Status::Missing => 2,
                Status::NotConverged => 3,
                Status::Ambiguous => 4,
            },
            origin: s.origin,
            scale: s.scale,
            window: AbiWindow {
                row_start: s.window.row_start,
                col_start: s.window.col_start,
                row_count: s.window.row_count,
                col_count: s.window.col_count,
            },
            row_count: s.row_count,
            col_count: s.col_count,
            data: s.data.as_ptr(),
            data_len: s.data.len(),
            strides: s.strides,
        };
    });
}

#[no_mangle]
pub unsafe extern "C" fn bil_abi_slice_free(s: *mut AbiSlice) {
    if !s.is_null() {
        drop(Box::from_raw(s));
    }
}
