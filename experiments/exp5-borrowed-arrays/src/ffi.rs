// SPDX-License-Identifier: LGPL-2.1-or-later
//! The parts of Python's stable interface (the limited API, version 3.11 on) this experiment
//! uses, declared by hand from CPython's headers.
//!
//! Only the layouts the stable interface promises are relied on: the object header, a module
//! definition, a type specification and its slots, method and attribute tables, and the buffer
//! structure. Free-threaded builds of Python have no stable interface, and are not supported.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use std::ffi::{c_char, c_int, c_long, c_uint, c_void};

pub type Py_ssize_t = isize;

/// The header every Python object starts with, in a build with the interpreter lock.
#[repr(C)]
pub struct PyObject {
    pub ob_refcnt: Py_ssize_t,
    pub ob_type: *mut PyTypeObject,
}

/// Opaque: types are made from a `PyType_Spec` and reached only through calls.
#[repr(C)]
pub struct PyTypeObject {
    _private: [u8; 0],
}

/// Opaque: the thread state handed back by `PyEval_SaveThread`.
#[repr(C)]
pub struct PyThreadState {
    _private: [u8; 0],
}

pub type PyCFunction = unsafe extern "C" fn(*mut PyObject, *mut PyObject) -> *mut PyObject;
pub type PyCFunctionWithKeywords =
    unsafe extern "C" fn(*mut PyObject, *mut PyObject, *mut PyObject) -> *mut PyObject;
pub type Getter = unsafe extern "C" fn(*mut PyObject, *mut c_void) -> *mut PyObject;

#[repr(C)]
pub struct PyMethodDef {
    pub ml_name: *const c_char,
    /// A `PyCFunction`, or a `PyCFunctionWithKeywords` cast to one, as `ml_flags` says.
    pub ml_meth: Option<PyCFunction>,
    pub ml_flags: c_int,
    pub ml_doc: *const c_char,
}

#[repr(C)]
pub struct PyGetSetDef {
    pub name: *const c_char,
    pub get: Option<Getter>,
    pub set: *mut c_void,
    pub doc: *const c_char,
    pub closure: *mut c_void,
}

#[repr(C)]
pub struct PyModuleDef_Base {
    pub ob_base: PyObject,
    pub m_init: Option<unsafe extern "C" fn() -> *mut PyObject>,
    pub m_index: Py_ssize_t,
    pub m_copy: *mut PyObject,
}

#[repr(C)]
pub struct PyModuleDef {
    pub m_base: PyModuleDef_Base,
    pub m_name: *const c_char,
    pub m_doc: *const c_char,
    pub m_size: Py_ssize_t,
    pub m_methods: *mut PyMethodDef,
    pub m_slots: *mut c_void,
    pub m_traverse: *mut c_void,
    pub m_clear: *mut c_void,
    pub m_free: *mut c_void,
}

#[repr(C)]
pub struct PyType_Slot {
    pub slot: c_int,
    pub pfunc: *mut c_void,
}

#[repr(C)]
pub struct PyType_Spec {
    pub name: *const c_char,
    pub basicsize: c_int,
    pub itemsize: c_int,
    pub flags: c_uint,
    pub slots: *mut PyType_Slot,
}

#[repr(C)]
pub struct Py_buffer {
    pub buf: *mut c_void,
    pub obj: *mut PyObject,
    pub len: Py_ssize_t,
    pub itemsize: Py_ssize_t,
    pub readonly: c_int,
    pub ndim: c_int,
    pub format: *mut c_char,
    pub shape: *mut Py_ssize_t,
    pub strides: *mut Py_ssize_t,
    pub suboffsets: *mut Py_ssize_t,
    pub internal: *mut c_void,
}

// From typeslots.h.
pub const Py_bf_getbuffer: c_int = 1;
pub const Py_bf_releasebuffer: c_int = 2;
pub const Py_tp_alloc: c_int = 47;
pub const Py_tp_dealloc: c_int = 52;
pub const Py_tp_doc: c_int = 56;
pub const Py_tp_methods: c_int = 64;
pub const Py_tp_getset: c_int = 73;
pub const Py_tp_free: c_int = 74;

// From object.h, methodobject.h, pybuffer.h and modsupport.h.
pub const Py_TPFLAGS_DEFAULT: c_uint = 0;
pub const Py_TPFLAGS_DISALLOW_INSTANTIATION: c_uint = 1 << 7;
pub const METH_VARARGS: c_int = 0x0001;
pub const METH_KEYWORDS: c_int = 0x0002;
pub const METH_NOARGS: c_int = 0x0004;
pub const PyBUF_WRITABLE: c_int = 0x0001;
pub const PyBUF_FORMAT: c_int = 0x0004;
pub const PyBUF_ND: c_int = 0x0008;
pub const PyBUF_STRIDES: c_int = 0x0010 | PyBUF_ND;
pub const PYTHON_ABI_VERSION: c_int = 3;

// On Windows the data symbols are reached through the import library, which needs the block to
// name it, so that they are imported rather than linked directly.
#[cfg_attr(windows, link(name = "python3"))]
extern "C" {
    pub static mut _Py_NoneStruct: PyObject;
    pub static mut PyExc_ValueError: *mut PyObject;
    pub static mut PyExc_OSError: *mut PyObject;
    pub static mut PyExc_BufferError: *mut PyObject;

    pub fn Py_IncRef(o: *mut PyObject);
    pub fn Py_DecRef(o: *mut PyObject);
    pub fn PyModule_Create2(def: *mut PyModuleDef, apiver: c_int) -> *mut PyObject;
    pub fn PyModule_AddObjectRef(m: *mut PyObject, name: *const c_char, o: *mut PyObject) -> c_int;
    pub fn PyType_FromSpec(spec: *mut PyType_Spec) -> *mut PyObject;
    pub fn PyType_GetSlot(t: *mut PyTypeObject, slot: c_int) -> *mut c_void;
    pub fn PyErr_SetString(t: *mut PyObject, message: *const c_char);
    pub fn PyLong_FromSsize_t(v: Py_ssize_t) -> *mut PyObject;
    pub fn PyBool_FromLong(v: c_long) -> *mut PyObject;
    pub fn PyArg_ParseTupleAndKeywords(
        args: *mut PyObject,
        kwargs: *mut PyObject,
        format: *const c_char,
        keywords: *const *const c_char,
        ...
    ) -> c_int;
    pub fn PyEval_SaveThread() -> *mut PyThreadState;
    pub fn PyEval_RestoreThread(t: *mut PyThreadState);
}

/// `None`, with a new reference.
pub unsafe fn none() -> *mut PyObject {
    let n = std::ptr::addr_of_mut!(_Py_NoneStruct);
    Py_IncRef(n);
    n
}

/// Raises `t` with `message` and returns null, as a failed call must.
pub unsafe fn raise(t: *mut PyObject, message: &str) -> *mut PyObject {
    let text = std::ffi::CString::new(message.replace('\0', " ")).unwrap_or_default();
    PyErr_SetString(t, text.as_ptr());
    std::ptr::null_mut()
}

/// Runs `f` with the interpreter lock released, so that other Python threads run meanwhile.
/// `f` must not touch any Python object.
pub fn without_lock<T>(f: impl FnOnce() -> T) -> T {
    // Restores the lock even if `f` panics, so that the panic can be turned into an error.
    struct Restore(*mut PyThreadState);
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe { PyEval_RestoreThread(self.0) }
        }
    }
    let _restore = Restore(unsafe { PyEval_SaveThread() });
    f()
}
