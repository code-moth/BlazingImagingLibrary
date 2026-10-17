// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Opening a Source through whichever reader recognizes it, and the calls every reader shares
//! (sections 3 and 10): a preview of the Source interface, over the drivers compiled in.

// With no driver compiled in, no Source can be made, and every call on one is unreachable.
#![cfg_attr(
    not(any(feature = "nitf", feature = "geotiff")),
    allow(unreachable_code, unused_variables, unused_mut)
)]

use std::path::Path;
use std::sync::Arc;

use crate::cache::CacheReport;
use crate::error::{Error, Result};
use crate::fields::{Field, FieldGroup};
#[cfg(feature = "geotiff")]
use crate::geotiff::GeoTiffSource;
use crate::io::ReadAt;
#[cfg(feature = "nitf")]
use crate::nitf::NitfSource;
use crate::types::{Fragment, ImageInfo, Scope, Storage, StoredLevel, Window};
use crate::warning::Warnings;

/// What `open_source` is given (section 3.4). This preview holds the options the readers act
/// on; the others join as the code that uses them is written (M50).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenOptions {
    /// Refuse a file whose chain of directories, such as TIFF's IFDs, is longer than this.
    pub max_directories: u64,
    /// Refuse an Image with more subdirectories, such as TIFF's SubIFDs, than this.
    pub max_subdirectories: u64,
    /// Distinct warnings held before more are dropped (section 3.3).
    pub max_held_warnings: usize,
}

impl Default for OpenOptions {
    fn default() -> OpenOptions {
        OpenOptions {
            max_directories: 100_000,
            max_subdirectories: 1_000,
            max_held_warnings: crate::warning::DEFAULT_MAX_HELD,
        }
    }
}

impl OpenOptions {
    /// Opens a file with these options.
    pub fn open(&self, path: impl AsRef<Path>) -> Result<Source> {
        self.open_from(std::fs::File::open(path)?)
    }

    /// Opens bytes from any `ReadAt`, a file or memory, with these options. A preview of
    /// section 10's "Driver selection": each driver compiled in checks its container's leading
    /// bytes, which no two share, so the first that recognizes them opens them, and its error,
    /// if it then fails, is the result (M54).
    pub fn open_from(&self, reader: impl ReadAt + 'static) -> Result<Source> {
        let reader: Arc<dyn ReadAt> = Arc::new(reader);
        let mut tried = Vec::new();
        #[cfg(feature = "nitf")]
        match NitfSource::open_with(reader.clone(), self) {
            Ok(s) => return Ok(Source(Inner::Nitf(Box::new(s)))),
            Err(Error::Unrecognized(t)) => tried.extend(t),
            Err(e) => return Err(e),
        }
        #[cfg(feature = "geotiff")]
        match GeoTiffSource::open_with(reader.clone(), self) {
            Ok(s) => return Ok(Source(Inner::GeoTiff(Box::new(s)))),
            Err(Error::Unrecognized(t)) => tried.extend(t),
            Err(e) => return Err(e),
        }
        let _ = reader;
        Err(Error::Unrecognized(tried))
    }
}

/// Opens a file with the default `OpenOptions` (section 2).
pub fn open_source(path: impl AsRef<Path>) -> Result<Source> {
    OpenOptions::default().open(path)
}

/// One file opened by a driver (section 3).
pub struct Source(Inner);

// Sources are `Send` and `Sync` (section 4), which the compiler checks here.
const _: fn() = || {
    fn shared<T: Send + Sync>() {}
    shared::<Source>();
};

enum Inner {
    #[cfg(feature = "nitf")]
    Nitf(Box<NitfSource<Arc<dyn ReadAt>>>),
    #[cfg(feature = "geotiff")]
    GeoTiff(Box<GeoTiffSource<Arc<dyn ReadAt>>>),
}

/// Calls the same method on whichever reader the Source holds.
macro_rules! each {
    ($inner:expr, $s:ident => $call:expr) => {
        match $inner {
            #[cfg(feature = "nitf")]
            Inner::Nitf(ref $s) => $call,
            #[cfg(feature = "geotiff")]
            Inner::GeoTiff(ref $s) => $call,
        }
    };
}

impl Source {
    /// Closes it now, freeing what it holds: its cache, warnings and readers. It is consumed, so
    /// no later call can be written (section 4, "Closing a Source"); Fragments already read stay
    /// valid. Dropping it does the same.
    pub fn close(self) {}

    /// The name of the driver that opened it: `nitf` or `geotiff`.
    pub fn driver(&self) -> &'static str {
        match self.0 {
            #[cfg(feature = "nitf")]
            Inner::Nitf(_) => crate::nitf::DRIVER,
            #[cfg(feature = "geotiff")]
            Inner::GeoTiff(_) => crate::geotiff::DRIVER,
        }
    }

    /// How many Images it holds.
    pub fn image_count(&self) -> usize {
        each!(self.0, s => s.image_count())
    }

    /// An `ImageInfo` for each Image, in order (section 3).
    pub fn list_images(&self) -> Result<Vec<ImageInfo>> {
        (0..self.image_count())
            .map(|i| each!(self.0, s => s.info(i)))
            .collect()
    }

    /// One Image, by its index, counted from 0.
    pub fn image(&self, index: usize) -> Result<Image<'_>> {
        if index >= self.image_count() {
            return Err(Error::NotFound(format!("image {index}")));
        }
        Ok(Image {
            source: self,
            index,
        })
    }

    /// The NITF reader, for what only NITF has, such as its segments and `ImageInfo`.
    #[cfg(feature = "nitf")]
    pub fn as_nitf(&self) -> Option<&NitfSource<Arc<dyn ReadAt>>> {
        match &self.0 {
            Inner::Nitf(s) => Some(s),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    /// The GeoTIFF reader, for what only TIFF has, such as its `ImageInfo`.
    #[cfg(feature = "geotiff")]
    pub fn as_geotiff(&self) -> Option<&GeoTiffSource<Arc<dyn ReadAt>>> {
        match &self.0 {
            Inner::GeoTiff(s) => Some(s),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    /// The warnings recorded since the last call, and how many were dropped past
    /// `max_held_warnings`; empties the store (section 3.3).
    pub fn take_warnings(&self) -> Warnings {
        each!(self.0, s => s.take_warnings())
    }

    /// Sets how many decoded Fragments this Source keeps; 0 turns the cache off (section 4).
    pub fn set_cache_size(&self, fragment_count: usize) {
        each!(self.0, s => s.set_cache_size(fragment_count))
    }

    /// Sets the most bytes of decoded Fragments this Source keeps (section 4).
    pub fn set_cache_bytes(&self, byte_count: u64) {
        each!(self.0, s => s.set_cache_bytes(byte_count))
    }

    /// The Fragments held, the limits, and counts in blocks (section 4).
    pub fn describe_cache(&self) -> CacheReport {
        each!(self.0, s => s.describe_cache())
    }

    /// Frees every cached Fragment (section 4).
    pub fn clear_cache(&self) {
        each!(self.0, s => s.clear_cache())
    }

    /// The field groups of the Source or one Image (section 14.4).
    pub fn list_field_groups(&self, scope: Scope) -> Result<Vec<String>> {
        each!(self.0, s => s.list_field_groups(scope))
    }

    /// A field group, decoded when first asked and then held (section 14.4).
    pub fn read_field_group(&self, scope: Scope, group: &str) -> Result<Arc<FieldGroup>> {
        each!(self.0, s => s.read_field_group(scope, group))
    }

    /// One field, by its name (section 14.4).
    pub fn read_field(&self, scope: Scope, name: &str) -> Result<Field> {
        each!(self.0, s => s.read_field(scope, name))
    }

    /// Lets a held group go (section 14.4).
    pub fn release_metadata(&self, scope: Scope, group: &str) {
        each!(self.0, s => s.release_metadata(scope, group))
    }
}

/// One Image of a Source (section 3).
#[derive(Clone, Copy)]
pub struct Image<'a> {
    source: &'a Source,
    index: usize,
}

impl<'a> Image<'a> {
    /// Its index in the Source.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Its `ImageInfo` (section 3).
    pub fn info(&self) -> Result<ImageInfo> {
        each!(self.source.0, s => s.info(self.index))
    }

    /// Its stored levels, finest first (section 5, `list_stored_levels`).
    pub fn list_stored_levels(&self) -> Result<&'a [StoredLevel]> {
        each!(self.source.0, s => s.list_stored_levels(self.index))
    }

    /// How a level's blocks lie in the file: `level0` or a stored level (section 10).
    pub fn describe_storage(&self, level: &str) -> Result<Storage> {
        each!(self.source.0, s => s.describe_level_storage(self.index, level))
    }

    /// The native pixels of a Window, at `level0` or a stored level (section 4).
    pub fn read_fragment(&self, window: Window, level: &str) -> Result<Fragment> {
        each!(self.source.0, s => s.read_level_fragment(self.index, level, window))
    }
}
