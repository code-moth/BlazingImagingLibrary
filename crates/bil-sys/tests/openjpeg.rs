// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The bindings against the library and its header.

#![cfg(feature = "openjpeg")]

use bil_sys::openjpeg::*;

#[test]
fn layout_matches_the_header() {
    match layout_check() {
        Some(c) => assert_eq!(layout_rust(), c, "the declarations do not match openjpeg.h"),
        // In CI the header is always installed, so this is never the path taken there.
        None => eprintln!("openjpeg.h was not found at build time: layout not checked"),
    }
}

#[test]
fn the_library_answers() {
    let v = unsafe { std::ffi::CStr::from_ptr(opj_version()) }.to_string_lossy();
    assert!(v.starts_with("2."), "OpenJPEG {v}");
    let mut p = std::mem::MaybeUninit::<opj_dparameters_t>::zeroed();
    unsafe { opj_set_default_decoder_parameters(p.as_mut_ptr()) };
    let p = unsafe { p.assume_init() };
    assert_eq!((p.cp_reduce, p.cp_layer, p.flags), (0, 0, 0));
}
