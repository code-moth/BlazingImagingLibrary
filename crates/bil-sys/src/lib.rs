// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Native bindings for the Blazing Imaging Library: BIL's own, where no existing crate passes the
//! tests of the specification's section 10. Each library is behind a feature of its own.
//! See <https://github.com/code-moth/BlazingImagingLibrary>.

#[cfg(feature = "openjpeg")]
pub mod openjpeg;
