// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The Blazing Imaging Library.
//!
//! **Early preview.** This release holds the vector layer and the first slice-function kernels
//! from the project's first experiment. The reading interface the specification describes is not
//! here yet. See <https://github.com/code-moth/BlazingImagingLibrary>.

// The kernels stay close to the specification's formulas.
#![allow(
    clippy::missing_safety_doc,
    clippy::needless_range_loop,
    clippy::excessive_precision
)]

pub mod kernels;
pub mod simd;
