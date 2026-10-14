# bil-sys

Native bindings used by the [Blazing Imaging Library](https://github.com/code-moth/BlazingImagingLibrary) (`bil`), for C and C++ libraries that have no suitable Rust bindings of their own.

**So far, OpenJPEG's decoder,** behind the `openjpeg` feature: the declarations decoding needs, written by hand from `openjpeg.h`, for OpenJPEG 2.3 or later. The build links the OpenJPEG of the system, Homebrew or conda, found under `CONDA_PREFIX`, `/usr`, `/usr/local` or `/opt/homebrew`, or in `BIL_OPENJPEG_LIB_DIR`. Where it finds the header too (`BIL_OPENJPEG_INCLUDE`, or `include/openjpeg-2.5`), it compiles a check that the declared structures match it, which the tests compare. The safe layer is `bil::codec::jpeg2000`. License: LGPL 2.1 or later.
