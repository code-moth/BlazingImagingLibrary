# bil

The Blazing Imaging Library reads imaging sensor data and its metadata from any format, through open source libraries as plugins, and delivers a processable subset as native Fragments or 2D float32 Slices. It also locates every pixel in the world through one geocoding chain.

**This is an early preview.** Version 0.0.x holds the vector layer and the first slice-function kernels, proven in the project's first experiment, and Tier 1's NITF container reader (`bil::nitf`, behind the default `nitf` feature), which reads NITF 2.0, NITF 2.1 and NSIF 1.0 headers, extensions and uncompressed pixels, and JPEG 2000 pixels with the `openjpeg` feature, off by default, which links the OpenJPEG the system or conda provides. The reading interface described in the specification is not here yet, and everything may change.

- Specification and design: https://github.com/code-moth/BlazingImagingLibrary
- License: LGPL 2.1 or later, with a static-linking exception (see the repository's `LINKING-EXCEPTION.md`).
