// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Codecs: what decodes the compressed blocks a container driver hands over (specification,
//! section 10, "Containers and codecs"). Each is behind a build feature of its own.

#[cfg(feature = "openjpeg")]
pub mod jpeg2000;
pub mod tiles;
