// SPDX-License-Identifier: LGPL-2.1-or-later
//! Experiment 5 of the implementability assessment: borrowed NumPy arrays.
//!
//! A Python extension module, `bil_exp5`, that maps a raw file and hands out Fragments and Slices
//! that borrow the mapped bytes, with no copy. Each exports Python's buffer protocol, so
//! `numpy.asarray` makes an array over the same memory, whose base keeps the Fragment alive, and
//! the Fragment keeps the mapping alive. The question is whether such an array stays valid after
//! its Source is closed and dropped, under several threads, is read-only under `Shared` mapping,
//! and leaks nothing (sections 4 and 12 of the specification).
//!
//! The file is raw samples in native byte order, row by row, after an `offset`: the simplest
//! format that can be mapped, so that the experiment tests borrowing and nothing else. A Window
//! with a `step` is a strided view, as section 4 says a decimation step is.
//!
//! The module is built against Python's stable interface by hand-written declarations
//! (`ffi.rs`), because this workspace cannot fetch PyO3. The protocol and the reference that
//! keeps a mapping alive are the same either way.

mod ffi;
pub mod map;

use ffi::*;
use map::{Mapped, Mapping};
use std::ffi::{c_char, c_int, c_long, c_void, CStr};
use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------- the core

/// The sample types this experiment reads, in native byte order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleType {
    U8,
    U16,
    F32,
}

impl SampleType {
    fn parse(name: &str) -> Option<SampleType> {
        match name {
            "U8" => Some(SampleType::U8),
            "U16" => Some(SampleType::U16),
            "F32" => Some(SampleType::F32),
            _ => None,
        }
    }
    fn size(self) -> usize {
        match self {
            SampleType::U8 => 1,
            SampleType::U16 => 2,
            SampleType::F32 => 4,
        }
    }
    /// The buffer protocol's format character, in native byte order and alignment.
    fn format(self) -> &'static CStr {
        match self {
            SampleType::U8 => c"B",
            SampleType::U16 => c"H",
            SampleType::F32 => c"f",
        }
    }
}

/// A rectangle of the Image, with a decimation step.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub row_start: usize,
    pub col_start: usize,
    pub row_count: usize,
    pub col_count: usize,
    pub step: usize,
}

/// One Image of raw samples in a mapped file.
pub struct Source {
    map: Arc<Mapped>,
    rows: usize,
    cols: usize,
    sample: SampleType,
    offset: usize,
}

/// What keeps an array's memory alive.
enum Owner {
    /// The mapping, for a borrowed array.
    Map(#[allow(dead_code)] Arc<Mapped>),
    /// A buffer of its own, in 8-byte words so that any sample is aligned.
    Own(#[allow(dead_code)] Vec<u64>),
}

/// The memory of a Fragment or a Slice, as the buffer protocol describes it: a 2D strided view.
pub struct Array {
    _owner: Owner,
    ptr: *mut u8,
    shape: [isize; 2],
    strides: [isize; 2],
    itemsize: isize,
    format: &'static CStr,
    readonly: bool,
    borrowed: bool,
}

// The memory is held by `_owner` for as long as the Array lives, and a reader on another thread
// sees it as any shared memory: read-only under `Shared`, and the caller's under `Private`.
unsafe impl Send for Array {}
unsafe impl Sync for Array {}

impl Source {
    pub fn open(
        path: &str,
        rows: usize,
        cols: usize,
        sample: SampleType,
        offset: usize,
        mapping: Mapping,
    ) -> Result<Source, String> {
        if rows == 0 || cols == 0 {
            return Err("an Image needs at least one row and one column".into());
        }
        let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
        let map = Mapped::new(&file, mapping).map_err(|e| format!("{path}: {e}"))?;
        let need = rows
            .checked_mul(cols)
            .and_then(|n| n.checked_mul(sample.size()))
            .and_then(|n| n.checked_add(offset));
        // Section 10.3: a header that claims more pixels than the file can hold is rejected.
        if need.is_none_or(|n| n > map.byte_count()) {
            return Err(format!(
                "{rows} × {cols} samples of {sample:?} after {offset} bytes do not fit in a file of {} bytes",
                map.byte_count()
            ));
        }
        Ok(Source {
            map: Arc::new(map),
            rows,
            cols,
            sample,
            offset,
        })
    }

    fn check(&self, w: Window) -> Result<(), String> {
        let fits = w.row_count > 0
            && w.col_count > 0
            && w.step > 0
            && w.row_start
                .checked_add(w.row_count)
                .is_some_and(|e| e <= self.rows)
            && w.col_start
                .checked_add(w.col_count)
                .is_some_and(|e| e <= self.cols);
        if fits {
            Ok(())
        } else {
            Err(format!(
                "the Window {w:?} is not inside the Image of {} × {}",
                self.rows, self.cols
            ))
        }
    }

    /// The address of the sample at (r, c).
    fn at(&self, r: usize, c: usize) -> *mut u8 {
        let byte = self.offset + (r * self.cols + c) * self.sample.size();
        unsafe { self.map.ptr().add(byte) }
    }

    /// Whether a view of this Source can borrow: rules 2 to 5 of the zero-copy rule hold by
    /// construction (uncompressed, native order, byte-aligned, mapped, evenly spaced rows), so
    /// what remains is rule 5, each sample on a multiple of its own size.
    fn borrowable(&self) -> bool {
        (self.at(0, 0) as usize).is_multiple_of(self.sample.size())
    }

    fn out_shape(w: Window) -> [usize; 2] {
        [w.row_count.div_ceil(w.step), w.col_count.div_ceil(w.step)]
    }

    /// The native samples of `w`: borrowed where the zero-copy rule holds, else copied once.
    pub fn read_fragment(&self, w: Window) -> Result<Array, String> {
        self.check(w)?;
        let size = self.sample.size();
        let [h, wd] = Source::out_shape(w);
        let readonly = self.map.mapping() == Mapping::Shared;
        if self.borrowable() {
            return Ok(Array {
                _owner: Owner::Map(Arc::clone(&self.map)),
                ptr: self.at(w.row_start, w.col_start),
                shape: [h as isize, wd as isize],
                strides: [
                    (self.cols * size * w.step) as isize,
                    (size * w.step) as isize,
                ],
                itemsize: size as isize,
                format: self.sample.format(),
                readonly,
                borrowed: true,
            });
        }
        // Owned: the samples of the Window, packed row by row.
        let bytes = h * wd * size;
        let mut own = vec![0u64; bytes.div_ceil(8)];
        let dst = own.as_mut_ptr().cast::<u8>();
        for i in 0..h {
            for j in 0..wd {
                let src = self.at(w.row_start + i * w.step, w.col_start + j * w.step);
                unsafe { std::ptr::copy_nonoverlapping(src, dst.add((i * wd + j) * size), size) };
            }
        }
        Ok(Array {
            _owner: Owner::Own(own),
            ptr: dst,
            shape: [h as isize, wd as isize],
            strides: [(wd * size) as isize, size as isize],
            itemsize: size as isize,
            format: self.sample.format(),
            readonly: false,
            borrowed: false,
        })
    }

    /// `pixel.raw.intensity` as float32: borrowed from float32 data that can be borrowed, as
    /// section 6 requires, and otherwise converted once into an array of its own.
    pub fn read_slice(&self, w: Window) -> Result<Array, String> {
        self.check(w)?;
        if self.sample == SampleType::F32 && self.borrowable() {
            return self.read_fragment(w);
        }
        let [h, wd] = Source::out_shape(w);
        let mut own = vec![0u64; (h * wd).div_ceil(2)];
        let dst = own.as_mut_ptr().cast::<f32>();
        for i in 0..h {
            let r = w.row_start + i * w.step;
            for j in 0..wd {
                let src = self.at(r, w.col_start + j * w.step);
                // Unaligned reads, since a Source that cannot borrow may be misaligned.
                let v = unsafe {
                    match self.sample {
                        SampleType::U8 => f32::from(*src),
                        SampleType::U16 => f32::from(src.cast::<u16>().read_unaligned()),
                        SampleType::F32 => src.cast::<f32>().read_unaligned(),
                    }
                };
                unsafe { *dst.add(i * wd + j) = v };
            }
        }
        Ok(Array {
            _owner: Owner::Own(own),
            ptr: dst.cast(),
            shape: [h as isize, wd as isize],
            strides: [(wd * 4) as isize, 4],
            itemsize: 4,
            format: c"f",
            readonly: false,
            borrowed: false,
        })
    }
}

// ---------------------------------------------------------------- counts, for the checks

/// Fragments and Slices not yet freed.
static LIVE_ARRAYS: AtomicUsize = AtomicUsize::new(0);
/// Buffer views handed out and not yet released.
static LIVE_EXPORTS: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------- the Python objects

#[repr(C)]
struct SourceObject {
    ob_base: PyObject,
    /// Null once the Source is closed.
    source: *mut Source,
}

#[repr(C)]
struct ArrayObject {
    ob_base: PyObject,
    array: *mut Array,
}

/// The three types, made when the module is first imported.
static SOURCE_TYPE: AtomicPtr<PyObject> = AtomicPtr::new(null_mut());
static FRAGMENT_TYPE: AtomicPtr<PyObject> = AtomicPtr::new(null_mut());
static SLICE_TYPE: AtomicPtr<PyObject> = AtomicPtr::new(null_mut());

/// Runs a call's body, turning a panic into a Python error rather than unwinding into Python.
fn guard(f: impl FnOnce() -> *mut PyObject) -> *mut PyObject {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(p) => p,
        Err(_) => unsafe { raise(PyExc_ValueError, "internal error in bil_exp5") },
    }
}

/// A new object of `type_`, with `payload` stored after its header.
unsafe fn new_object(type_: &AtomicPtr<PyObject>, payload: *mut c_void) -> *mut PyObject {
    let t = type_.load(Ordering::Acquire).cast::<PyTypeObject>();
    let alloc: unsafe extern "C" fn(*mut PyTypeObject, Py_ssize_t) -> *mut PyObject =
        std::mem::transmute(PyType_GetSlot(t, Py_tp_alloc));
    let o = alloc(t, 0);
    if !o.is_null() {
        // Both object layouts hold one pointer after the header.
        (*o.cast::<ArrayObject>()).array = payload.cast();
    }
    o
}

/// Frees an object of one of the module's types, after its payload has been dropped.
unsafe fn free_object(o: *mut PyObject) {
    let t = (*o).ob_type;
    let free: unsafe extern "C" fn(*mut c_void) =
        std::mem::transmute(PyType_GetSlot(t, Py_tp_free));
    free(o.cast());
    // Instances of a heap type hold a reference to it.
    Py_DecRef(t.cast());
}

fn wrap_array(type_: &AtomicPtr<PyObject>, a: Result<Array, String>) -> *mut PyObject {
    match a {
        Ok(a) => unsafe {
            let o = new_object(type_, Box::into_raw(Box::new(a)).cast());
            if !o.is_null() {
                LIVE_ARRAYS.fetch_add(1, Ordering::Relaxed);
            }
            o
        },
        Err(e) => unsafe { raise(PyExc_ValueError, &e) },
    }
}

unsafe extern "C" fn array_dealloc(o: *mut PyObject) {
    let a = (*o.cast::<ArrayObject>()).array;
    if !a.is_null() {
        drop(Box::from_raw(a));
        LIVE_ARRAYS.fetch_sub(1, Ordering::Relaxed);
    }
    free_object(o);
}

/// The buffer protocol: a 2D strided view of the array's memory.
unsafe extern "C" fn array_getbuffer(
    o: *mut PyObject,
    view: *mut Py_buffer,
    flags: c_int,
) -> c_int {
    let a = &*(*o.cast::<ArrayObject>()).array;
    if flags & PyBUF_WRITABLE != 0 && a.readonly {
        (*view).obj = null_mut();
        raise(
            PyExc_BufferError,
            "a borrowed array under Shared mapping is read-only",
        );
        return -1;
    }
    let contiguous = a.strides[1] == a.itemsize && a.strides[0] == a.shape[1] * a.itemsize;
    if flags & PyBUF_STRIDES != PyBUF_STRIDES && !contiguous {
        (*view).obj = null_mut();
        raise(
            PyExc_BufferError,
            "this array is strided, and the request cannot take strides",
        );
        return -1;
    }
    Py_IncRef(o);
    let v = &mut *view;
    v.buf = a.ptr.cast();
    v.obj = o;
    v.len = a.shape[0] * a.shape[1] * a.itemsize;
    v.itemsize = a.itemsize;
    v.readonly = c_int::from(a.readonly);
    v.format = if flags & PyBUF_FORMAT != 0 {
        a.format.as_ptr().cast_mut()
    } else {
        null_mut()
    };
    if flags & PyBUF_ND != 0 {
        v.ndim = 2;
        v.shape = a.shape.as_ptr().cast_mut();
    } else {
        v.ndim = 1;
        v.shape = null_mut();
    }
    v.strides = if flags & PyBUF_STRIDES == PyBUF_STRIDES {
        a.strides.as_ptr().cast_mut()
    } else {
        null_mut()
    };
    v.suboffsets = null_mut();
    v.internal = null_mut();
    LIVE_EXPORTS.fetch_add(1, Ordering::Relaxed);
    0
}

unsafe extern "C" fn array_releasebuffer(_o: *mut PyObject, _view: *mut Py_buffer) {
    LIVE_EXPORTS.fetch_sub(1, Ordering::Relaxed);
}

unsafe extern "C" fn array_is_borrowed(o: *mut PyObject, _: *mut c_void) -> *mut PyObject {
    PyBool_FromLong(c_long::from((*(*o.cast::<ArrayObject>()).array).borrowed))
}

unsafe extern "C" fn source_dealloc(o: *mut PyObject) {
    close_source(o);
    free_object(o);
}

/// Drops what the Source holds. Arrays borrowed from it keep the mapping until they go.
unsafe fn close_source(o: *mut PyObject) {
    let s = &mut (*o.cast::<SourceObject>()).source;
    if !s.is_null() {
        drop(Box::from_raw(*s));
        *s = null_mut();
    }
}

unsafe extern "C" fn source_close(o: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
    close_source(o);
    none()
}

unsafe extern "C" fn source_closed(o: *mut PyObject, _: *mut c_void) -> *mut PyObject {
    PyBool_FromLong(c_long::from((*o.cast::<SourceObject>()).source.is_null()))
}

/// Parses `(row_start, col_start, row_count, col_count, step=1)`.
unsafe fn parse_window(args: *mut PyObject, kwargs: *mut PyObject) -> Option<Window> {
    let names: [*const c_char; 6] = [
        c"row_start".as_ptr(),
        c"col_start".as_ptr(),
        c"row_count".as_ptr(),
        c"col_count".as_ptr(),
        c"step".as_ptr(),
        std::ptr::null(),
    ];
    let (mut r0, mut c0, mut rc, mut cc, mut step): (isize, isize, isize, isize, isize) =
        (0, 0, 0, 0, 1);
    let ok = PyArg_ParseTupleAndKeywords(
        args,
        kwargs,
        c"nnnn|n".as_ptr(),
        names.as_ptr(),
        &mut r0 as *mut isize,
        &mut c0 as *mut isize,
        &mut rc as *mut isize,
        &mut cc as *mut isize,
        &mut step as *mut isize,
    );
    if ok == 0 {
        return None;
    }
    let all = [r0, c0, rc, cc, step];
    if all.iter().any(|&v| v < 0) {
        raise(PyExc_ValueError, "a Window has no negative parts");
        return None;
    }
    Some(Window {
        row_start: r0 as usize,
        col_start: c0 as usize,
        row_count: rc as usize,
        col_count: cc as usize,
        step: step as usize,
    })
}

/// A read on the Source, with the interpreter lock released while it runs.
unsafe fn source_read(
    o: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
    type_: &AtomicPtr<PyObject>,
    read: fn(&Source, Window) -> Result<Array, String>,
) -> *mut PyObject {
    let Some(w) = parse_window(args, kwargs) else {
        return null_mut();
    };
    let s = (*o.cast::<SourceObject>()).source;
    if s.is_null() {
        return raise(PyExc_ValueError, "the Source is closed");
    }
    // The Source is not freed while the lock is released: only `close` and the Source's own
    // deallocation free it, and both need the lock, which this call holds again before return.
    // Another thread could close it meanwhile, so the read works on an Arc of its own.
    let source = Source {
        map: Arc::clone(&(*s).map),
        ..*s
    };
    let result = without_lock(|| read(&source, w));
    wrap_array(type_, result)
}

unsafe extern "C" fn source_read_fragment(
    o: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject {
    guard(|| unsafe { source_read(o, args, kwargs, &FRAGMENT_TYPE, Source::read_fragment) })
}

unsafe extern "C" fn source_read_slice(
    o: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject {
    guard(|| unsafe { source_read(o, args, kwargs, &SLICE_TYPE, Source::read_slice) })
}

unsafe extern "C" fn module_open_source(
    _m: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject {
    guard(|| unsafe {
        let names: [*const c_char; 7] = [
            c"path".as_ptr(),
            c"row_count".as_ptr(),
            c"col_count".as_ptr(),
            c"sample_type".as_ptr(),
            c"offset".as_ptr(),
            c"mapping".as_ptr(),
            std::ptr::null(),
        ];
        let mut path: *const c_char = std::ptr::null();
        let (mut rows, mut cols, mut offset): (isize, isize, isize) = (0, 0, 0);
        let mut sample: *const c_char = c"U16".as_ptr();
        let mut mapping: *const c_char = c"Shared".as_ptr();
        let ok = PyArg_ParseTupleAndKeywords(
            args,
            kwargs,
            c"snn|sns".as_ptr(),
            names.as_ptr(),
            &mut path as *mut *const c_char,
            &mut rows as *mut isize,
            &mut cols as *mut isize,
            &mut sample as *mut *const c_char,
            &mut offset as *mut isize,
            &mut mapping as *mut *const c_char,
        );
        if ok == 0 {
            return null_mut();
        }
        let text = |p: *const c_char| CStr::from_ptr(p).to_string_lossy().into_owned();
        let (path, sample, mapping) = (text(path), text(sample), text(mapping));
        let Some(sample) = SampleType::parse(&sample) else {
            return raise(
                PyExc_ValueError,
                &format!("unknown sample type {sample}: U8, U16 or F32"),
            );
        };
        let mapping = match mapping.as_str() {
            "Shared" => Mapping::Shared,
            "Private" => Mapping::Private,
            other => {
                return raise(
                    PyExc_ValueError,
                    &format!("unknown mapping {other}: Shared or Private"),
                )
            }
        };
        if rows < 0 || cols < 0 || offset < 0 {
            return raise(PyExc_ValueError, "sizes and offsets are not negative");
        }
        let opened = without_lock(|| {
            Source::open(
                &path,
                rows as usize,
                cols as usize,
                sample,
                offset as usize,
                mapping,
            )
        });
        match opened {
            Ok(s) => new_object(&SOURCE_TYPE, Box::into_raw(Box::new(s)).cast()),
            Err(e) => raise(PyExc_OSError, &e),
        }
    })
}

unsafe extern "C" fn module_live_mappings(_m: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
    PyLong_FromSsize_t(map::LIVE.load(Ordering::Relaxed) as isize)
}

unsafe extern "C" fn module_live_arrays(_m: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
    PyLong_FromSsize_t(LIVE_ARRAYS.load(Ordering::Relaxed) as isize)
}

unsafe extern "C" fn module_live_exports(_m: *mut PyObject, _: *mut PyObject) -> *mut PyObject {
    PyLong_FromSsize_t(LIVE_EXPORTS.load(Ordering::Relaxed) as isize)
}

// ---------------------------------------------------------------- the module

/// A `PyCFunctionWithKeywords` in the slot of a `PyCFunction`, as `METH_KEYWORDS` asks.
fn with_keywords(f: PyCFunctionWithKeywords) -> Option<PyCFunction> {
    Some(unsafe { std::mem::transmute::<PyCFunctionWithKeywords, PyCFunction>(f) })
}

const END_METHOD: PyMethodDef = PyMethodDef {
    ml_name: std::ptr::null(),
    ml_meth: None,
    ml_flags: 0,
    ml_doc: std::ptr::null(),
};

const END_GETSET: PyGetSetDef = PyGetSetDef {
    name: std::ptr::null(),
    get: None,
    set: null_mut(),
    doc: std::ptr::null(),
    closure: null_mut(),
};

fn getset(name: &'static CStr, get: Getter, doc: &'static CStr) -> PyGetSetDef {
    PyGetSetDef {
        name: name.as_ptr(),
        get: Some(get),
        set: null_mut(),
        doc: doc.as_ptr(),
        closure: null_mut(),
    }
}

fn method(
    name: &'static CStr,
    f: Option<PyCFunction>,
    flags: c_int,
    doc: &'static CStr,
) -> PyMethodDef {
    PyMethodDef {
        ml_name: name.as_ptr(),
        ml_meth: f,
        ml_flags: flags,
        ml_doc: doc.as_ptr(),
    }
}

/// Makes a type from its slots. The tables are leaked: they must outlive the type, which lives
/// as long as the interpreter.
unsafe fn make_type(
    name: &'static CStr,
    basicsize: usize,
    slots: Vec<PyType_Slot>,
) -> *mut PyObject {
    let mut slots = slots;
    slots.push(PyType_Slot {
        slot: 0,
        pfunc: null_mut(),
    });
    let spec = Box::leak(Box::new(PyType_Spec {
        name: name.as_ptr(),
        basicsize: basicsize as c_int,
        itemsize: 0,
        flags: Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION,
        slots: Box::leak(slots.into_boxed_slice()).as_mut_ptr(),
    }));
    PyType_FromSpec(spec)
}

unsafe fn array_type(name: &'static CStr, doc: &'static CStr) -> *mut PyObject {
    let getsets = Box::leak(Box::new([
        getset(
            c"is_borrowed",
            array_is_borrowed,
            c"True if these are the stored bytes themselves, not a copy",
        ),
        END_GETSET,
    ]));
    let slots = vec![
        PyType_Slot {
            slot: Py_tp_doc,
            pfunc: doc.as_ptr().cast_mut().cast(),
        },
        PyType_Slot {
            slot: Py_tp_dealloc,
            pfunc: array_dealloc as *mut c_void,
        },
        PyType_Slot {
            slot: Py_tp_getset,
            pfunc: getsets.as_mut_ptr().cast(),
        },
        PyType_Slot {
            slot: Py_bf_getbuffer,
            pfunc: array_getbuffer as *mut c_void,
        },
        PyType_Slot {
            slot: Py_bf_releasebuffer,
            pfunc: array_releasebuffer as *mut c_void,
        },
    ];
    make_type(name, std::mem::size_of::<ArrayObject>(), slots)
}

unsafe fn source_type() -> *mut PyObject {
    let methods = Box::leak(Box::new([
        method(
            c"read_fragment",
            with_keywords(source_read_fragment),
            METH_VARARGS | METH_KEYWORDS,
            c"read_fragment(row_start, col_start, row_count, col_count, step=1): the native samples",
        ),
        method(
            c"read_slice",
            with_keywords(source_read_slice),
            METH_VARARGS | METH_KEYWORDS,
            c"read_slice(row_start, col_start, row_count, col_count, step=1): pixel.raw.intensity",
        ),
        method(
            c"close",
            Some(source_close),
            METH_NOARGS,
            c"Frees what the Source holds; arrays borrowed from it stay valid",
        ),
        END_METHOD,
    ]));
    let getsets = Box::leak(Box::new([
        getset(c"closed", source_closed, c"True once the Source is closed"),
        END_GETSET,
    ]));
    let slots = vec![
        PyType_Slot {
            slot: Py_tp_doc,
            pfunc: c"A mapped raw file".as_ptr().cast_mut().cast(),
        },
        PyType_Slot {
            slot: Py_tp_dealloc,
            pfunc: source_dealloc as *mut c_void,
        },
        PyType_Slot {
            slot: Py_tp_methods,
            pfunc: methods.as_mut_ptr().cast(),
        },
        PyType_Slot {
            slot: Py_tp_getset,
            pfunc: getsets.as_mut_ptr().cast(),
        },
    ];
    make_type(
        c"bil_exp5.Source",
        std::mem::size_of::<SourceObject>(),
        slots,
    )
}

/// The module's entry point, called by Python on import.
///
/// # Safety
/// Called by the interpreter, with its lock held, once for each import of the module.
#[no_mangle]
pub unsafe extern "C" fn PyInit_bil_exp5() -> *mut PyObject {
    let methods = Box::leak(Box::new([
        method(
            c"open_source",
            with_keywords(module_open_source),
            METH_VARARGS | METH_KEYWORDS,
            c"open_source(path, row_count, col_count, sample_type='U16', offset=0, mapping='Shared')",
        ),
        method(c"live_mappings", Some(module_live_mappings), METH_NOARGS, c"Mappings not yet unmapped"),
        method(c"live_arrays", Some(module_live_arrays), METH_NOARGS, c"Fragments and Slices not yet freed"),
        method(c"live_exports", Some(module_live_exports), METH_NOARGS, c"Buffer views not yet released"),
        END_METHOD,
    ]));
    let def = Box::leak(Box::new(PyModuleDef {
        m_base: PyModuleDef_Base {
            ob_base: PyObject {
                ob_refcnt: 1,
                ob_type: null_mut(),
            },
            m_init: None,
            m_index: 0,
            m_copy: null_mut(),
        },
        m_name: c"bil_exp5".as_ptr(),
        m_doc: c"BIL experiment 5: borrowed NumPy arrays over memory-mapped files".as_ptr(),
        m_size: -1,
        m_methods: methods.as_mut_ptr(),
        m_slots: null_mut(),
        m_traverse: null_mut(),
        m_clear: null_mut(),
        m_free: null_mut(),
    }));
    let m = PyModule_Create2(def, PYTHON_ABI_VERSION);
    if m.is_null() {
        return m;
    }
    let types = [
        (&SOURCE_TYPE, c"Source", source_type()),
        (
            &FRAGMENT_TYPE,
            c"Fragment",
            array_type(c"bil_exp5.Fragment", c"The native samples of a Window"),
        ),
        (
            &SLICE_TYPE,
            c"Slice",
            array_type(
                c"bil_exp5.Slice",
                c"pixel.raw.intensity of a Window, as float32",
            ),
        ),
    ];
    for (slot, name, t) in types {
        if t.is_null() || PyModule_AddObjectRef(m, name.as_ptr(), t) < 0 {
            Py_DecRef(m);
            return null_mut();
        }
        // The module keeps one reference and this static another, for the interpreter's life.
        slot.store(t, Ordering::Release);
    }
    m
}
