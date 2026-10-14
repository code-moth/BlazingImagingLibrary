// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Bindings to OpenJPEG's decoder, version 2.3 or later, declared by hand from its `openjpeg.h`.
//!
//! No existing wrapper or bindings crate passes section 10's four tests for OpenJPEG: none can
//! use the patched shared copy a package manager supplies, which section 10's "JPEG 2000 codecs
//! in order" asks for wherever there is one (the user manual's chapter 24, M31, records the
//! assessment). So these are BIL's own, and
//! they link the system's or conda's `libopenjp2`.
//!
//! Only what decoding needs is declared. OpenJPEG reports every error through the callbacks it is
//! given and never jumps, so no C layer stands between it and Rust. Its image and parameter
//! structures are read and written directly, so their layout must match the library's: the build
//! compiles a check against the C header where one is found (`layout_check`, used by the tests),
//! and `opj_version` is checked when a decoder is made.

#![allow(non_camel_case_types, non_snake_case)]

use std::ffi::{c_char, c_int, c_void};

pub type OPJ_BOOL = c_int;
pub type OPJ_UINT32 = u32;
pub type OPJ_INT32 = i32;
pub type OPJ_UINT64 = u64;
pub type OPJ_OFF_T = i64;
pub type OPJ_SIZE_T = usize;

/// `OPJ_CODEC_FORMAT`: a raw codestream.
pub const OPJ_CODEC_J2K: c_int = 0;
/// `OPJ_CODEC_FORMAT`: a JP2 file.
pub const OPJ_CODEC_JP2: c_int = 2;
pub const OPJ_J2K_STREAM_CHUNK_SIZE: OPJ_SIZE_T = 0x100000;
pub const OPJ_DPARAMETERS_IGNORE_PCLR_CMAP_CDEF_FLAG: u32 = 0x0001;
const OPJ_PATH_LEN: usize = 4096;

/// `opj_dparameters_t`.
#[repr(C)]
pub struct opj_dparameters_t {
    pub cp_reduce: OPJ_UINT32,
    pub cp_layer: OPJ_UINT32,
    pub infile: [c_char; OPJ_PATH_LEN],
    pub outfile: [c_char; OPJ_PATH_LEN],
    pub decod_format: c_int,
    pub cod_format: c_int,
    pub DA_x0: OPJ_UINT32,
    pub DA_x1: OPJ_UINT32,
    pub DA_y0: OPJ_UINT32,
    pub DA_y1: OPJ_UINT32,
    pub m_verbose: OPJ_BOOL,
    pub tile_index: OPJ_UINT32,
    pub nb_tile_to_decode: OPJ_UINT32,
    pub jpwl_correct: OPJ_BOOL,
    pub jpwl_exp_comps: c_int,
    pub jpwl_max_tiles: c_int,
    pub flags: u32,
}

/// `opj_image_comp_t`.
#[repr(C)]
pub struct opj_image_comp_t {
    pub dx: OPJ_UINT32,
    pub dy: OPJ_UINT32,
    pub w: OPJ_UINT32,
    pub h: OPJ_UINT32,
    pub x0: OPJ_UINT32,
    pub y0: OPJ_UINT32,
    pub prec: OPJ_UINT32,
    /// Deprecated in OpenJPEG; kept for the layout.
    pub bpp: OPJ_UINT32,
    pub sgnd: OPJ_UINT32,
    pub resno_decoded: OPJ_UINT32,
    pub factor: OPJ_UINT32,
    pub data: *mut OPJ_INT32,
    pub alpha: u16,
}

/// `opj_image_t`.
#[repr(C)]
pub struct opj_image_t {
    pub x0: OPJ_UINT32,
    pub y0: OPJ_UINT32,
    pub x1: OPJ_UINT32,
    pub y1: OPJ_UINT32,
    pub numcomps: OPJ_UINT32,
    pub color_space: c_int,
    pub comps: *mut opj_image_comp_t,
    pub icc_profile_buf: *mut u8,
    pub icc_profile_len: OPJ_UINT32,
}

/// `opj_codec_t` and `opj_stream_t` are opaque.
#[repr(C)]
pub struct opj_codec_t {
    _private: [u8; 0],
}
#[repr(C)]
pub struct opj_stream_t {
    _private: [u8; 0],
}

pub type opj_msg_callback =
    Option<unsafe extern "C" fn(msg: *const c_char, client_data: *mut c_void)>;
pub type opj_stream_read_fn = Option<
    unsafe extern "C" fn(
        buffer: *mut c_void,
        nb_bytes: OPJ_SIZE_T,
        user: *mut c_void,
    ) -> OPJ_SIZE_T,
>;
pub type opj_stream_skip_fn =
    Option<unsafe extern "C" fn(nb_bytes: OPJ_OFF_T, user: *mut c_void) -> OPJ_OFF_T>;
pub type opj_stream_seek_fn =
    Option<unsafe extern "C" fn(nb_bytes: OPJ_OFF_T, user: *mut c_void) -> OPJ_BOOL>;
pub type opj_stream_free_user_data_fn = Option<unsafe extern "C" fn(user: *mut c_void)>;

// OpenJPEG's OPJ_CALLCONV is __stdcall in a Windows DLL and the C convention elsewhere, which is
// what "system" means.
extern "system" {
    pub fn opj_version() -> *const c_char;
    pub fn opj_image_destroy(image: *mut opj_image_t);
    pub fn opj_stream_create(buffer_size: OPJ_SIZE_T, is_input: OPJ_BOOL) -> *mut opj_stream_t;
    pub fn opj_stream_destroy(stream: *mut opj_stream_t);
    pub fn opj_stream_set_read_function(stream: *mut opj_stream_t, f: opj_stream_read_fn);
    pub fn opj_stream_set_skip_function(stream: *mut opj_stream_t, f: opj_stream_skip_fn);
    pub fn opj_stream_set_seek_function(stream: *mut opj_stream_t, f: opj_stream_seek_fn);
    pub fn opj_stream_set_user_data(
        stream: *mut opj_stream_t,
        data: *mut c_void,
        free: opj_stream_free_user_data_fn,
    );
    pub fn opj_stream_set_user_data_length(stream: *mut opj_stream_t, length: OPJ_UINT64);
    pub fn opj_set_info_handler(
        codec: *mut opj_codec_t,
        f: opj_msg_callback,
        user: *mut c_void,
    ) -> OPJ_BOOL;
    pub fn opj_set_warning_handler(
        codec: *mut opj_codec_t,
        f: opj_msg_callback,
        user: *mut c_void,
    ) -> OPJ_BOOL;
    pub fn opj_set_error_handler(
        codec: *mut opj_codec_t,
        f: opj_msg_callback,
        user: *mut c_void,
    ) -> OPJ_BOOL;
    pub fn opj_create_decompress(format: c_int) -> *mut opj_codec_t;
    pub fn opj_destroy_codec(codec: *mut opj_codec_t);
    pub fn opj_end_decompress(codec: *mut opj_codec_t, stream: *mut opj_stream_t) -> OPJ_BOOL;
    pub fn opj_set_default_decoder_parameters(parameters: *mut opj_dparameters_t);
    pub fn opj_setup_decoder(
        codec: *mut opj_codec_t,
        parameters: *mut opj_dparameters_t,
    ) -> OPJ_BOOL;
    pub fn opj_read_header(
        stream: *mut opj_stream_t,
        codec: *mut opj_codec_t,
        image: *mut *mut opj_image_t,
    ) -> OPJ_BOOL;
    pub fn opj_set_decode_area(
        codec: *mut opj_codec_t,
        image: *mut opj_image_t,
        start_x: OPJ_INT32,
        start_y: OPJ_INT32,
        end_x: OPJ_INT32,
        end_y: OPJ_INT32,
    ) -> OPJ_BOOL;
    pub fn opj_codec_set_threads(codec: *mut opj_codec_t, num_threads: c_int) -> OPJ_BOOL;
    pub fn opj_set_decoded_resolution_factor(
        codec: *mut opj_codec_t,
        factor: OPJ_UINT32,
    ) -> OPJ_BOOL;
    pub fn opj_decode(
        codec: *mut opj_codec_t,
        stream: *mut opj_stream_t,
        image: *mut opj_image_t,
    ) -> OPJ_BOOL;
}

/// The sizes and field offsets of the C structures, as the C compiler lays them out from the
/// header found at build time, in the order `layout_rust` gives them; `None` where the build
/// found no header to check against.
pub fn layout_check() -> Option<[u64; 12]> {
    #[cfg(bil_sys_opj_layout)]
    {
        extern "C" {
            fn bil_sys_opj_layout(out: *mut u64);
        }
        let mut out = [0u64; 12];
        unsafe { bil_sys_opj_layout(out.as_mut_ptr()) };
        Some(out)
    }
    #[cfg(not(bil_sys_opj_layout))]
    None
}

/// The same sizes and offsets of the Rust declarations above.
pub fn layout_rust() -> [u64; 12] {
    use std::mem::{offset_of, size_of};
    [
        size_of::<opj_dparameters_t>() as u64,
        offset_of!(opj_dparameters_t, decod_format) as u64,
        offset_of!(opj_dparameters_t, DA_x0) as u64,
        offset_of!(opj_dparameters_t, flags) as u64,
        size_of::<opj_image_comp_t>() as u64,
        offset_of!(opj_image_comp_t, prec) as u64,
        offset_of!(opj_image_comp_t, sgnd) as u64,
        offset_of!(opj_image_comp_t, data) as u64,
        offset_of!(opj_image_comp_t, alpha) as u64,
        size_of::<opj_image_t>() as u64,
        offset_of!(opj_image_t, comps) as u64,
        offset_of!(opj_image_t, icc_profile_len) as u64,
    ]
}
