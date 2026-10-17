// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The Blazing Imaging Library.
//!
//! **Early preview.** This release holds the vector layer and the first slice-function kernels
//! from the project's first experiment, and Tier 1's NITF container reader, which reads headers,
//! extensions and uncompressed pixels. The reading interface the specification describes is not
//! here yet. See <https://github.com/code-moth/BlazingImagingLibrary>.

// The kernels stay close to the specification's formulas.
#![allow(
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::excessive_precision
)]

pub mod cache;
pub mod codec;
pub mod error;
pub mod fields;
#[cfg(feature = "geotiff")]
pub mod geotiff;
pub mod io;
pub mod kernels;
#[cfg(feature = "nitf")]
pub mod nitf;
pub mod simd;
pub mod source;
pub mod types;
pub mod warning;

pub use error::{Error, Result};
pub use source::{open_source, Image, OpenOptions, Source};
