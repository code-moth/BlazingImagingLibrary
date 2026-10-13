// SPDX-License-Identifier: LGPL-2.1-or-later
//! A file mapped into memory, as `mapping` in `OpenOptions` chooses (section 4 of the
//! specification): `Shared`, read-only, or `Private`, copy-on-write, so that a caller can change
//! the array without touching the file.
//!
//! The mapping lives as long as anything holds it. Every borrowed array holds it through an
//! `Arc`, so it is unmapped only when the Source and every array borrowed from it are gone.

use std::fs::File;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Mappings not yet unmapped, for the checks.
pub static LIVE: AtomicUsize = AtomicUsize::new(0);

/// How a file is mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapping {
    Shared,
    Private,
}

pub struct Mapped {
    ptr: *mut u8,
    len: usize,
    mapping: Mapping,
    #[cfg(windows)]
    section: isize,
}

// The mapping is plain memory: under `Shared` no one writes it, and under `Private` writes go
// through arrays the caller owns, as for any memory shared between threads.
unsafe impl Send for Mapped {}
unsafe impl Sync for Mapped {}

impl Mapped {
    /// Maps the whole of `file`.
    pub fn new(file: &File, mapping: Mapping) -> io::Result<Mapped> {
        let len = usize::try_from(file.metadata()?.len())
            .map_err(|_| io::Error::other("the file is larger than memory can address"))?;
        if len == 0 {
            return Err(io::Error::other("an empty file cannot be mapped"));
        }
        let mapped = sys::map(file, len, mapping)?;
        LIVE.fetch_add(1, Ordering::Relaxed);
        Ok(mapped)
    }

    pub fn ptr(&self) -> *mut u8 {
        self.ptr
    }

    pub fn byte_count(&self) -> usize {
        self.len
    }

    pub fn mapping(&self) -> Mapping {
        self.mapping
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        sys::unmap(self);
        LIVE.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(unix)]
mod sys {
    use super::{Mapped, Mapping};
    use std::ffi::{c_int, c_void};
    use std::fs::File;
    use std::io;
    use std::os::fd::AsRawFd;

    // The same values on Linux and macOS.
    const PROT_READ: c_int = 1;
    const PROT_WRITE: c_int = 2;
    const MAP_SHARED: c_int = 1;
    const MAP_PRIVATE: c_int = 2;

    extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            offset: i64,
        ) -> *mut c_void;
        fn munmap(addr: *mut c_void, len: usize) -> c_int;
    }

    pub fn map(file: &File, len: usize, mapping: Mapping) -> io::Result<Mapped> {
        let (prot, flags) = match mapping {
            Mapping::Shared => (PROT_READ, MAP_SHARED),
            Mapping::Private => (PROT_READ | PROT_WRITE, MAP_PRIVATE),
        };
        let p = unsafe { mmap(std::ptr::null_mut(), len, prot, flags, file.as_raw_fd(), 0) };
        // MAP_FAILED is all bits set.
        if p as usize == usize::MAX {
            return Err(io::Error::last_os_error());
        }
        Ok(Mapped {
            ptr: p.cast(),
            len,
            mapping,
        })
    }

    pub fn unmap(m: &mut Mapped) {
        unsafe { munmap(m.ptr.cast(), m.len) };
    }
}

#[cfg(windows)]
mod sys {
    use super::{Mapped, Mapping};
    use std::ffi::c_void;
    use std::fs::File;
    use std::io;
    use std::os::windows::io::AsRawHandle;

    const PAGE_READONLY: u32 = 0x02;
    const PAGE_WRITECOPY: u32 = 0x08;
    const FILE_MAP_COPY: u32 = 0x01;
    const FILE_MAP_READ: u32 = 0x04;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileMappingW(
            file: isize,
            attributes: *mut c_void,
            protect: u32,
            size_high: u32,
            size_low: u32,
            name: *const u16,
        ) -> isize;
        fn MapViewOfFile(
            section: isize,
            access: u32,
            offset_high: u32,
            offset_low: u32,
            bytes: usize,
        ) -> *mut c_void;
        fn UnmapViewOfFile(base: *const c_void) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    pub fn map(file: &File, len: usize, mapping: Mapping) -> io::Result<Mapped> {
        let (protect, access) = match mapping {
            Mapping::Shared => (PAGE_READONLY, FILE_MAP_READ),
            Mapping::Private => (PAGE_WRITECOPY, FILE_MAP_COPY),
        };
        let handle = file.as_raw_handle() as isize;
        let section = unsafe {
            CreateFileMappingW(
                handle,
                std::ptr::null_mut(),
                protect,
                0,
                0,
                std::ptr::null(),
            )
        };
        if section == 0 {
            return Err(io::Error::last_os_error());
        }
        let p = unsafe { MapViewOfFile(section, access, 0, 0, 0) };
        if p.is_null() {
            let e = io::Error::last_os_error();
            unsafe { CloseHandle(section) };
            return Err(e);
        }
        Ok(Mapped {
            ptr: p.cast(),
            len,
            mapping,
            section,
        })
    }

    pub fn unmap(m: &mut Mapped) {
        unsafe {
            UnmapViewOfFile(m.ptr as *const c_void);
            CloseHandle(m.section);
        }
    }
}
