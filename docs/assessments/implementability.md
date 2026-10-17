This document asks whether BIL, as specification version 56 describes it, can be built, and where building it is likely to go wrong. It is a judgment made from the specification, from knowledge of the libraries named in it, and from how comparable libraries have fared. Version 2 revised section 6 for the arrangement now intended: Claude writes BIL, and the specification's author leads, reviews and supplies what Claude cannot reach. Versions 3 to 9 bring the document in step with specification version 70: every item that section 7 listed as blocking is now settled, and experiment 1 has been run. Version 5 adds access patterns and domain profiles, and section 10 takes stock of what they mean for the code already written. Version 7 records experiment 5, borrowed NumPy arrays, version 8 experiment 3, containing C errors, and version 9 the author's acceptance of the proposals of the user manual and experiments 1b, 3 and 5. Version 10 records GDAL's part of experiment 3 and the first Tier 1 reader, for NITF, and version 11 the author's acceptance, in specification versions 72 and 73, of the proposals they raised and of one the ruling on M28 raised, M21 to M30 in the user manual's chapter 24. Version 12 brings the whole document in step with specification version 81, and version 13 with version 82, which built the C++ interface, and 83, and adds section 11, which records what has been built and how it was tested. Versions 71 to 81 settled M19 to M65 of the user manual's chapter 24: the Source interface, stored levels, representations, the Fragment cache, and, in version 81, warnings held on the Source, a principle that the interface is not driven by edge cases, and a C++ interface in place of the C one, which rewrites H9. The rest of the judgment stands as written for version 56.

**What has changed between versions 56 and 82**

| Change | Where it bears |
| --- | --- |
| Agreement to a tolerance, not to the bit (R27). Experiment 1 found bitwise agreement cheap all the same. | H1 |
| The interpreter is CPython from the machine, run in a helper process for Rust and C++ callers | H5 |
| What ships stays compatible with LGPL 2.1, so SIX, which is LGPL 3, cannot be inside a package | H6 |
| Decoding on request, and the mapped format's layout, start-up, readiness and slot marking, are designed | H8 |
| Geocoding carries full accuracy records (section 13.9), which adds partial derivatives to every step | H7 |
| Tier 1 reads GeoTIFF and NITF with Rust readers, and 22 real NITF test files are pinned | Section 7 |
| Speed aims to be as fast as each function's arithmetic allows, measured against a copy and against that arithmetic (version 66) | H1, section 8 |
| `missing_weights` chooses how coarse Levels weight missing data: `Equal` or `PixelCount` (version 67) | H1 |
| `Random` and `Sequential`: read-ahead and decoding ahead in the storage order each driver reports (versions 68 and 69) | Section 3, section 10 |
| Domain profiles in the metadata dictionary, with 27 new items (version 68) | Section 3, section 10 |
| The proposals of the user manual and experiments 1b, 3 and 5 accepted: `may_have_missing`, closing, Python forms, the configuration file, missing data at coarse Levels, `Private` mapping, PyO3, recovered data as corrupt, `max_scans` and a cap on library memory, JPEG 2000 palettes as indices (version 70) | H1, H2, section 3 |
| Field layouts, NITF pad pixels, structural damage failing an open, codec names, and the TIFF reading rules: the `tiff` crate for directories and BIL for chunks, LZW and PackBits in Tier 1, `max_directories` and `max_subdirectories` (versions 72 to 75) | Section 3, section 11 |
| OpenJPEG through BIL's own bindings, with a private copy only where no shared one is found (version 74) | H2, H3, H6 |
| Stored levels' positions and Fragments, JPEG 2000 resolution levels aligned (version 76) | H1, H3 |
| The Source interface: `OpenOptions`, one error type, `describe_storage` by level, `ImageInfo` with representations and component names (versions 74, 77 to 79) | Section 3, section 11 |
| The Fragment cache: one entry for each block, every block not borrowed, the byte ceiling giving way for one block with a warning, a block decoded once among threads (version 80) | Section 3, section 10 |
| Warnings held on the Source and drained when the caller chooses; the interface not driven by edge cases (version 81) | Section 3, section 11 |
| A C++ interface with Rust's names and shapes in place of the C interface, for callers and external plugins (version 81): C++20, header-only over a narrow binary layer, failures thrown, header code under the linking exception (version 82) | H9, section 9 |

## 1. Verdict

**BIL can be built, but not all at once.** Tier 1 (GeoTIFF and NITF) is a realistic first project. Its hardest parts are known problems with known solutions. The risks grow sharply after Tier 1, for three reasons:

- **The number of C and C++ libraries.** Each one brings its own build, threading, error handling and global state, and BIL must contain all of them.
- **Video.** Frame-accurate, deferred and multi-threaded decoding through FFmpeg is a hard problem that many projects have got subtly wrong.
- **Isolation and the mapped format.** Process isolation that is safe on Linux, macOS and Windows is a project of its own.

**What is done.** Tier 1's two readers, the JPEG 2000 codec, the Source interface, the Fragment cache, the warning store and the C++ interface over them are built and tested on Linux, macOS, Windows and ARM, and four of the five first experiments are done. Section 11 has the detail. The judgment below is unchanged by it: the work so far is the part the verdict called realistic.

**The main danger is breadth, not any single part.** Section 8 and Appendix A name roughly 60 kinds of data and about 40 libraries. With one implementer, an attempt to keep all of them working would spend almost all its time on builds and packaging, not on the core. The tier plan is the right defense, and this document argues for holding to it strictly.

**Five problems should be tried in small experiments before Tier 1 is coded.** They are listed in section 8: the fused loop and its window independence, OpenJPEG's speed on large NITF files, containing C libraries' error handling, packaging with private copies, and the borrowed-array lifetime in Python. The first, the fused loop, is done, in experiments 1 and 1b; the third, containing C errors, for libjpeg-turbo, OpenJPEG and GDAL in experiment 3; and the fifth, borrowed arrays, in experiment 5.

## 2. What makes BIL easier to build than it looks

Several decisions already taken reduce the work and the risk.

| Decision | Why it helps |
| --- | --- |
| Fragments and Slices are 2D, with everything beyond selected | No N-dimensional array machinery. Every loop works on rows. |
| One sample type per Fragment | Every Fragment is a simple strided view. No mixed-type records. |
| Float32 for every Slice | One output type, so one set of output paths |
| Containers apart from codecs | One JPEG 2000 codec serves every container, so codecs are wrapped once |
| Pixels by description for scripted drivers | The interpreter never touches the pixel path, so Python costs nothing per pixel |
| The core owns slice functions, normalization and geocoding steps | Drivers stay small: they describe and decode, and nothing else |
| Window independence | Tiles can be tested against whole images, which gives a strong, cheap test oracle |
| Tiers, each finished before the next | The core is proven on two formats before breadth is added |
| Tested against established references | GDAL, SIX and PROJ give expected answers for most geocoding steps |
| Writing limited to four simple formats | No general writers to maintain |

## 3. Component by component

The difficulty ratings are judgments: Low is routine work, Medium needs care, High needs specialist knowledge or design work still to be done, and Very high is a project in its own right.

| Component | Difficulty | Why | Main risk | Done, as of version 84 |
| --- | --- | --- | --- | --- |
| Window, Fragment, Slice types | Low | Plain data structures | None of note | Built: Window and owned Fragment in `bil::types`; Slice prototyped in experiment 1b |
| Strided Fragments and zero-copy | Medium | Lifetimes of borrowed memory across Rust, C++ and Python | A borrowed array outliving its mapping | Prototyped: borrowed arrays in experiment 5; the readers' Fragments are owned until mapping is built |
| The fused loop | High | Every slice function × sample type × instruction set × kind of Slice | Code size, compile time, and bitwise agreement (H1) | Prototyped in experiments 1 and 1b; kernels built in `bil::kernels` |
| All Levels in one pass | High | Rolling buffers, margins that grow with the Level, mirroring by pattern period at edges | Edge cases at Image borders and in the last rows | Prototyped: the rolling pass of experiment 1b |
| Fragment cache | Medium | Sharing between Readers and threads, deferred and decoded forms | Memory use for very large blocks (pass 2, R32) | Built for both readers, with limits, the one-block rule and decoding once among threads; JPEG 2000 kept tile by tile |
| Access patterns and read-ahead | Medium | A background Reader, storage order from every driver, eviction of passed blocks, operating system hints on three systems | Read-ahead competing with the caller's own threads, and errors met ahead of time reported at the right read | Begun: both readers report storage order for every level; read-ahead not built |
| GeoTIFF driver | Low to Medium | Mature libraries in Rust and C | Choosing between GDAL, libtiff and a Rust reader | Built: none of GDAL's 232 TIFF test files differ |
| NITF driver | Medium | NITRO is mature; many extensions and odd files exist | Breadth of real-world NITF variation | Built: 41 real files agree with GDAL |
| JPEG 2000 in NITF | High | Region and reduced-resolution decoding of very large codestreams | Speed of OpenJPEG on large untiled files (H3) | Built over OpenJPEG, region and reduced-resolution reads; speed on large files unmeasured |
| SICD and SIDD | Medium to High | SIX is the reference but is a large C++ code base | Building and packaging SIX (H6) | Not started |
| ENVI driver | Low | Text header and flat binary | None of note | Not started |
| Video with KLV | Very high | Frame-accurate seeking, deferred decoding, threads, KLV timing | Seeking correctness across real streams (H4) | Not started |
| Metadata framework | Medium | Groups, lazy decoding, names as lists, repeats, the dictionary | Hand-mapping each format to the dictionary | Begun: fields, groups decoded on first read, NITF layouts and extensions, TIFF tags and GeoTIFF keys; no dictionary |
| Domain profiles | Low to Medium | Named lists of dictionary items, checked against a Source | Agreeing what each domain truly requires; the items themselves are the dictionary's work | Not started |
| Field definitions for NITF extensions and DICOM | Medium, and large | Hundreds of definitions | Volume of work, not difficulty | Begun: NITF headers, subheaders and RPC00 extensions |
| Geocoding: affine, projective, RPC | Low to Medium | Well understood, with GDAL as reference | Iterative inverses near the edges of validity | Not started |
| Geocoding: SAR, camera, pose | High | Rigorous models with time-dependent geometry | Agreement to 0.01 pixel against references | Not started |
| Geocoding: replacement sensor model | High | Many sections and grids; no open reference implementation known | Validation (H7) | Not started |
| Terrain | Medium | Tiling, interpolation, geoid, line-of-sight intersection | Robust intersection at grazing angles | Not started |
| Writers | Low | Four simple formats | Geocoding and markings in writers (pass 2, R28 and R30) | Not started |
| Rust interface | Medium | Lifetimes of borrowed Slices and Fragments | Ergonomics of `Cow` and lifetimes for callers | Begun: Source, Image, `OpenOptions`, `ImageInfo`, stored levels, storage, Fragments, the cache and warnings; no Slices or requests |
| C++ interface over all of Rust | High, and large | Every type and call needs a C++ form over a binary layer Rust can export, with the same names and lifetimes as Rust's | Size of the surface, a binary form stable between compilers, and keeping it stable (pass 2, R39; H9) | Built for what the Rust code reads today: the binary layer and the header, tested with GCC, Clang and MSVC on four systems |
| Python interface | Medium | PyO3 and NumPy are mature. Experiment 5 found the stable interface small enough to declare by hand, about 190 lines for what it needs. | The interpreter lock in callbacks. Borrowed arrays keeping their memory alive passed in experiment 5. | Prototyped in experiment 5 |
| Python for scripted drivers | Medium | A helper process for Rust and C++ callers; the caller's own Python inside Python | The round trip's cost, which is benchmarked (H5) | Not started |
| Scripted drivers | Medium | The `PixelLayout` path is simple; the array fallback is not | Keeping the interpreter off the pixel path in every case | Not started |
| External decoding, mapped format, isolation | Very high | Processes, shared memory and sandboxing on three systems | Passing unnamed shared memory and marking slots on three systems; sandbox limits on macOS and Windows (H8) | Not started; containing C errors prototyped in experiment 3 |
| Packaging and coexistence | High | Private copies with hidden symbols, conda and system packages | Clashes with GDAL, HDF5 and FFmpeg from other packages (H2) | Begun: crate and package names held, a publishing workflow; experiment 4 not run |
| Conformance suite | Medium, and large | One suite for every driver, slice function, Level and instruction set | Run time of the full matrix | Begun: each reader's tests and its comparison with GDAL run in CI on four systems |
| Fuzz testing | Medium | Standard tools exist for Rust | Coverage of the Rust layer over C libraries | Begun: seeded corrupt files for both readers in their tests; no fuzzing harness |
| Warning store | Low | One bounded store on each Source, recorded into only when a warning occurs | None of note | Built, with `badly_packed` for files whose blocks pass the cache's ceiling |

## 4. The hard problems

Each problem says what it is, why it is hard, how likely and how damaging it is, and how to retire it early.

### H1. The fused loop: size, speed and agreement to the bit

**What.** Section 12 asks for one compiled loop per slice function for each of six sample types, with AVX2, AVX-512 and NEON versions chosen at run time, every Level made in the same pass, and no branches in the inner loop. Pass 2 (R27) adds that a pixel must come out bit-identical whichever Window it is read through.

**Why it is hard.**

- **Combinations.** About 60 slice functions, 6 sample types and 4 instruction paths, counting the scalar fallback, give over a thousand loop bodies. Written by hand, that cannot be maintained. Generated by Rust generics and macros, it can, but compile times and binary size grow with it.
- **Portable SIMD is not in stable Rust.** As of September 2026, `std::simd` was still available only on nightly builds. BIL must use a crate such as fearless\_simd, pulp or wide, or write intrinsics for each instruction set.
- **Bitwise agreement constrains the code.** Every pixel must take the same sequence of operations, whether it falls in a vector body or at the end of a row. That rules out the usual scalar tail, and needs masked tails or a scalar path that copies the vector one exactly.
- **Transcendental functions.** Arctangent, logarithm and power need vector versions that agree with themselves at every position. Approximations from a library may not.

**Likelihood and impact.** Likely to cost more time than planned. High impact, because every Slice passes through it.

**How to retire it.** A two-week experiment: three slice functions (`pixel.raw`, `complex.polar`, `color.hsv`) on two sample types, two instruction sets, Levels 0 to 4 in one pass, with the Window independence test run bitwise over thousands of random Windows. Measure speed against a plain memory copy, compile time and code size.

**What happened.** Agreement is now required to a tolerance (R27), and experiment 1 ran this plan with two sample types for each function, on scalar, AVX2 and AVX-512 code. Bitwise agreement held everywhere at little cost, the kernels ran at memory speed, and the program built in 13 seconds at 0.6 MB. The whole pass missed the speed criterion, at 5 to 90 times a copy, because it made whole planes. Experiment 1b's rolling buffer kept every bit and brought `pixel.raw` to 1.6 to 2.8 times a copy. `complex.polar` and `color.hsv` are limited by their own arithmetic, at 1.3 to 2.9 times that arithmetic alone. The author ruled that the criterion guides the design, as fast as each function's arithmetic allows, and section 12 now says so. Experiment 1b also added missing data with NaN skipping, checked bitwise (`RESULTS.md` and `RESULTS-1b.md` in `experiments/exp1-fused-loop`).

### H2. Containing many C and C++ libraries in one process

**What.** BIL loads GDAL, NITRO, SIX, LibRaw, HDF5, NetCDF, OpenJPEG, FFmpeg, PROJ and others into the caller's process, beside other packages that may hold their own copies.

**Why it is hard.**

- **Error handling that jumps.** libjpeg-family and libpng-family libraries report errors with `setjmp` and `longjmp`. A jump across Rust stack frames is undefined behavior, so every such library needs a small C layer that catches the jump before it reaches Rust.
- **Global state.** GDAL's configuration options and driver registry, PROJ's database, HDF5's global lock and FFmpeg's logging are all process-wide. BIL's rule against touching shared handlers (section 10.5) has to be enforced one library at a time.
- **Plugins of their own.** GDAL, GStreamer and others look for plugins on search paths. Section 10.5 forbids that, so each must be configured to stop.
- **Private copies.** Hiding the symbols of a statically built C library works differently on Linux, macOS and Windows. A Rust shared library exports only what it marks for export, which helps, but libraries that are loaded with global visibility by another package can still collide.

**Likelihood and impact.** Certain to cost effort. Medium impact per library, and high in total.

**How to retire it.** Build the coexistence test of section 10.5 first, before any driver is finished: one process that loads BIL beside rasterio, h5py and PyAV, and reads through each in turn, on all three systems.

### H3. JPEG 2000 speed on large NITF files

**What.** Tier 1 requires JPEG 2000 inside NITF. Until a Rust codec passes BIL's tests, OpenJPEG decodes every file.

**Why it is hard.** Rule 1 says the cost of a small Window should not depend on the size of the image around it. JPEG 2000 meets that only when the file is tiled or has precincts that allow random access. Many NITF files are large single-tile codestreams. OpenJPEG can decode a region, but by its reputation it is several times slower than Kakadu, which many users of NITF already have. A BIL that is much slower than the tools its users have now will not be adopted for this format.

**Likelihood and impact.** Likely. High impact for the main intelligence formats.

**How to retire it.** Measure early. Time OpenJPEG's region and reduced-resolution decoding on large real NITF files, single-tile and tiled, against GDAL with OpenJPEG and, where a license is available, Kakadu. If the gap is large, make the external driver for Kakadu (section A.14) a planned path for licensed users rather than an afterthought.

### H4. Video: deferred, frame-accurate and threaded

**What.** A video Fragment holds packets from the preceding key frame. It is decoded when a Slice is formed, possibly on another thread, and each Reader keeps its place in the stream.

**Why it is hard.**

- **Seeking is not exact.** FFmpeg's seeking lands near a key frame, not on a frame. Finding a given frame needs decoding forward from the key frame and matching time stamps. Open groups of pictures, B-frames, variable frame rates, broken time stamps and MPEG transport streams all defeat simple methods. This has been a known difficulty on FFmpeg's own lists for many years.
- **A frame number is not given by the stream.** BIL numbers frames, but streams number nothing, so the frame index of section 3.2 must be exact for every stream it accepts.
- **Deferred decoding across threads.** Pass 2 (R31) found that a deferred Fragment has no defined decoder. Decoders hold large state, so making one per Fragment is costly, and sharing one needs care.
- **KLV timing.** Metadata packets arrive on their own clock and must be matched to frames by time.

**Likelihood and impact.** Very likely to take longer than planned. High impact for Tier 2.

**How to retire it.** Before Tier 2 is planned in detail, build a frame-accurate reader over FFmpeg for a test set of awkward streams: open groups of pictures, variable rate, transport streams with KLV, and interlaced video. Compare every frame against a full sequential decode.

### H5. The Python interpreter for scripted drivers

**What.** Scripted drivers must work for Rust and C++ callers, so BIL embeds Python. When BIL is itself loaded by Python, it must use that interpreter instead.

**Why it is hard.**

- **Which Python.** Settled in version 62: the Python installed on the machine, and BIL ships none. In version 63, Rust and C++ callers reach it through a helper process, so BIL is not tied to its version.
- **Two roles in one library.** Inside Python, BIL uses that interpreter. Outside it, the helper process does, so the shared library is never an embedder in the caller's process.
- **Libraries the scripts call.** A script that imports SARkit or SarPy needs those installed in whatever Python BIL runs.

**Likelihood and impact.** Likely to cause packaging problems. Medium impact, since scripted drivers are a Tier 2 feature.

**How to retire it.** Build one scripted driver and call it from C, from Rust and from Python on all three systems, and benchmark the helper process's round trip.

### H6. Building and shipping SIX and the other large C++ libraries

**What.** SIX is the reference for SICD and SIDD, and the planned driver for both. It is a large C++ code base built on its own support libraries.

**Why it is hard.** Large C++ libraries are the hardest to build as private copies with hidden symbols, and the C++ standard library itself is a common source of clashes between packages. SIX is also LGPL-3.0. What BIL ships stays compatible with LGPL 2.1, so SIX cannot be inside a BIL package, and it is assessed under that rule when Tier 2 starts.

**Likelihood and impact.** Likely. Medium impact, in Tier 2.

**How to retire it.** Check which package managers already carry SIX, which this review did not confirm. Compare the cost of an optional SIX plugin against a Rust or scripted SICD reader, with SIX or SARkit kept as the test reference.

### H7. Geocoding steps with no open reference

**What.** Section 13.8 requires every step to agree with an established implementation to one hundredth of a pixel.

**Why it is hard.** Some models have no open implementation to test against. The NITF replacement sensor model is the main case: GDAL does not implement it, and the established implementation known to this review is not open source. GRDL lists support for it, which may serve as a check. Camera models depend on a Community Sensor Model plugin existing for the same sensor, which is often not the case.

**Likelihood and impact.** Certain for some steps. Medium impact: those steps fall back to round trip and known points, as section 13.8 already allows.

**How to retire it.** List, for every step and format, the reference that will be used, before the step is written. Where none exists, find test files with surveyed ground points.

### H8. Isolation and the mapped format

**What.** Section 10.2 runs decoding in a separate process and shares the result through a memory-mapped format. Section 10.3 builds isolation on it.

**Why it is hard.**

- **Designed, not built.** Decoding on request (R26) and the mapped format's details (version 64) are settled: page-aligned slots, a control channel, slot marking, and shared memory passed without a name.
- **Three systems.** Shared memory, process start-up and clean-up of leftovers differ on Linux, macOS and Windows.
- **Reduced privileges.** Removing network access and limiting file access from a decoder process is well supported on Linux and much weaker or harder elsewhere. Section 10.3 already says "where the platform allows", which is honest, but users may expect more.

**Likelihood and impact.** Very likely to take longer than planned. Medium impact, because isolation is off by default and outside Tier 1.

**How to retire it.** Keep it out of Tier 1, as planned. Build it for Linux first, then pass unnamed shared memory on Windows and macOS.

### H9. The C++ interface over all of Rust

**What.** Version 81 replaced the C interface with a C++ one that has Rust's names and shapes, for callers and for external plugins. Section 10.6 says it covers all of BIL.

**Why it is hard.** Rust cannot export C++ classes, and C++ has no binary form that is stable between compilers and standard libraries, so the classes callers see cannot be what the shared library exports. Every type, option, callback and result still needs a form at that boundary, with rules for ownership; a callback must not let a C++ exception, a Rust panic or a Python exception cross it; and each call is a promise that cannot be broken without a major version. Plugins built by one compiler must load into BIL built by another. The header's own code compiles into callers' programs, which bears on the LGPL (M71).

**Likelihood and impact.** Certain to be large. Medium impact. Smaller than the C interface it replaces in one respect: names and lifetimes map one to one from Rust, as M70, accepted in version 82, has them.

**How to retire it.** The hourglass of M69: a narrow layer of plain functions on opaque handles, exported by the shared library and generated from the Rust interface, and a header-only C++ interface over it, compiled by the caller. Test the header with GCC, Clang and MSVC in CI on all four systems, and load a plugin built by one compiler into BIL built by another. Version 82 settled M69 to M71, and the first part is built: the layer and the header for everything the Rust code reads, tested in CI with GCC, Clang and MSVC, and under AddressSanitizer. Still to come: a plugin built by one compiler loaded into BIL built by another, which waits for external drivers.

### H10. Breadth: the formats and libraries of Tier 3

**What.** Tier 3 is "the remaining formats of section 8 and the remaining libraries of Appendix A".

**Why it is hard.** Each library needs a wrapper, a driver, reference files, conformance passes on three systems, packaging, and keeping up with its releases. At even one to three months each, forty libraries is several engineer-years before any maintenance. Several are maintained by one person.

**Likelihood and impact.** Certain, if Tier 3 is taken literally. High impact on the team's time.

**How to retire it.** Treat Tier 3 as a list to choose from, not a commitment. Add a library only when a user needs it and the conformance suite can be run against it. Prefer scripted drivers over compiled ones where the pixels can be described.

## 5. Dependencies to watch

These are the backing libraries whose health matters most to the early tiers. Health is as their own pages or earlier reviews described it.

| Library | Role | Health | Risk | Fallback |
| --- | --- | --- | --- | --- |
| OpenJPEG | JPEG 2000, for every file until a Rust codec passes | Described by its own repository as unmaintained | Security fixes arrive slowly; speed (H3) | j2k-native on trial; Kakadu through an external driver |
| emuella-j2k | Preferred JPEG 2000 codec, once proven | Unreleased, one owner | It may never reach general use | OpenJPEG and j2k-native |
| j2k-native | Rust JPEG 2000, on trial | Active, one owner | Unproven conformance | OpenJPEG |
| rusty\_ffmpeg | Bindings to FFmpeg | Active, tracking FFmpeg 6 to 8 | A gap after each FFmpeg release | Bindings generated in `bil-sys` |
| FFmpeg | Video | Active and widely used | Version spread across package managers | Version 6 or later through conda |
| NITRO | NITF container | Maintained by its sponsors | Small community | GDAL's NITF driver, as a second route |
| SIX | SICD and SIDD | Maintained by its sponsors | Building and packaging (H6) | A Rust or scripted reader, tested against SIX |
| GDAL | GeoTIFF and general rasters | Very active | Clashes with other packages' copies (H2) | A Rust TIFF reader for GeoTIFF |
| HDF5 | Scientific and hyperspectral data | Active | One call at a time in the whole process | hidefix, for reading |
| PROJ | Datum and projection changes | Very active | Its database and grids must be private to BIL | None needed |

## 6. Who builds it, and at what pace

**The arrangement.** Claude is the only implementer. The specification's author leads the project and reviews the work, and has led a team that built a comparable system. This changes the report in three ways: the pace is set by sessions and review, not by head count; some specialist roles are covered by knowledge but not by field experience; and continuity has to live in the repository.

**Who does what**

| Role | Claude | The author |
| --- | --- | --- |
| Design | Proposes details, flags what the specification leaves open | Rules on every decision, as in the specification's review so far |
| Code | Writes the Rust core, drivers, wrappers, the C++ and Python interfaces, tests, fuzz harnesses and CI workflows | Reads and accepts each change |
| Correctness | Runs the conformance suite, reference comparisons and fuzzing on every change | Judges what tests cannot: whether a sensor model is right in the field, and which real files will break it |
| Test data | Builds synthetic files and small reference files | Supplies real NITF, SICD, WAMI, video and medical files, which this workspace cannot download |
| Other systems | Writes CI for macOS, Windows and ARM | Provides the repository and its CI runners, and a machine for benchmarks |
| Licensed and closed comparisons | None | Kakadu, and any check on a closed network |

**What the workspace can and cannot do**

| | In this workspace |
| --- | --- |
| Systems | Linux on x86 only. macOS, Windows and ARM are reached only through CI. |
| Instruction sets | AVX2 and AVX-512 can be run and tested here. NEON only through CI on ARM. |
| Cores | Two. Enough to build and test, not to measure parallel scaling or run the dedicated benchmarks of section 12. |
| Network | Package registries and GitHub. Most other sites, including those that host sample imagery, are refused. |
| Memory between sessions | None. Each session starts from the repository. |

**How continuity is kept.** The repository holds everything a new session needs: the specification as its source of truth, a decisions log, a notes file that records the state of the work and the next step, and a test suite that says at once whether anything has broken. Each session starts by reading the notes and running the tests.

**How progress is measured.** Elapsed time depends on how many sessions run and how fast changes are reviewed, which only the author controls. So progress is measured by milestones, each with a test that says when it is done, rather than by months.

| Milestone | Done when |
| --- | --- |
| Experiments | The five experiments of section 8 have each passed or changed the design |
| Core | Window, Fragment, Slice, Levels and the cache pass Window independence bitwise on synthetic data, on every instruction set in CI |
| GeoTIFF | The GeoTIFF driver passes the full conformance suite on reference files |
| NITF | The NITF driver, with JPEG 2000 through OpenJPEG, passes the suite on files the author supplies |
| Geocoding for Tier 1 | `image_window`, `affine` and `rational_polynomial` agree with GDAL to within the tolerance of section 13.8 |
| Interfaces | Python and C++ examples from BIL by Example run unchanged against Tier 1 |
| Tier 1 | All of the above, plus writers, markings and fuzzing, on Linux, macOS and Windows |

**What changes in the plan**

- **Hold strictly to the tiers.** With one implementer, Tier 3 as written is out of reach. Each Tier 3 library is added only when a user needs it.
- **Prefer what needs no outside build.** Where a Rust reader is good enough, as a TIFF reader may be for GeoTIFF, it is preferred over wrapping GDAL, because every wrapped C library adds build and packaging work that falls on one implementer.
- **Review is the scarce resource.** Changes are kept small, each with its tests, so they can be reviewed quickly. The areas where review matters most are geocoding, calibration and anything that touches markings.
- **Video and isolation are the riskiest Tier 2 work.** Both need many rounds of testing against real streams and real systems, so each depends on the author's test data and CI more than on code.

## 7. What the specification must settle before coding

These are the points where code cannot be written, or would have to be rewritten, until a decision is made. As of specification version 70, every one is settled. Versions 71 to 84 settled what writing the code raised; open now are M75 to M79, the shape of `read_slice` and its `Slice`, which are coded as proposed and block nothing. Version 68 opened one point that blocked nothing, the details of access patterns, and version 69 settles it.

**Before Tier 1**

| Item | Why it blocks |
| --- | --- |
| R27, bitwise window independence | It decides how every loop is written. Settled: to a tolerance (version 59). |
| R29, stored levels and the Level grid | It decides how stored levels are used in Tier 1's GeoTIFF and NITF. Settled: a misaligned stored level never makes a Level (version 60). |
| R30, writing geocoding | Tier 1 writes GeoTIFF. Settled: writers write the geo model (version 59). |
| R28, markings inside written files | Tier 1 writes TIFF from NITF, which can carry markings. Settled (version 59). |
| U8, how resolution is named in a request | It shapes `SliceRequest` and geocoding calls in every interface. Settled: Levels by name, such as `stored2.level1` (version 57). |
| U9, how "no value" is reported | It shapes every metadata call. Settled: status `Missing` with no value (version 57). |
| The gaps listed in BIL by Example | Each is a name or form every interface will carry. Settled: all 17 closed (version 58). |
| Test data | Tier 1 needs GeoTIFF and NITF reference files. Settled: two layers, with 22 real NITF files pinned (version 61). |
| R39, interface stability policy | Before the first release, not the first line of code. Settled: semantic versioning (version 59). |

**Before Tier 2**

| Item | Why it blocks |
| --- | --- |
| R26, isolation on request | The mapped format's design depends on it. Settled (version 59). |
| R31, decoders for deferred Fragments | Video's threading depends on it. Settled: deferred Fragments are self-contained (version 59). |
| R34, live streams | Timeouts and cancellation shape the stream interface. Settled (version 59). |
| Interpreter packaging | Scripted drivers depend on it. Settled: CPython from the machine, in a helper process (versions 62 and 63). |
| Mapped format details | External decoding depends on it. Settled (version 64). |

## 8. The first experiments, in order

Each is small, answers one question, and can change the design while change is cheap. Experiments 1, 3 and 5 can run in this workspace. Experiment 2 needs large NITF files from the author, and experiment 4 needs CI on macOS and Windows.

1. **The fused loop (H1). Done, in experiments 1 and 1b: correct to the bit, with missing data, and close to the cost of each function's arithmetic.** Three slice functions, two sample types, two instruction sets, Levels 0 to 4 in one pass. Pass when Window independence holds bitwise over random Windows and speed is within a stated factor of a memory copy. Version 66 replaced that speed test with one against each function's own arithmetic as well.
2. **JPEG 2000 on real NITF (H3). Begun: the codec is built and reads regions and reduced resolutions, and agrees with GDAL on the three JPEG 2000 Images among the real test files; none of them is large, so the timing waits for the author's files; tile-by-tile decoding, which the cache uses, is timed on synthetic codestreams of 4096 × 4096 and 2048 × 2048 pixels in `crates/bil/tests/RESULTS-jpeg2000-tiles.md`.** Time region and reduced-resolution reads on large files with OpenJPEG and, if possible, Kakadu. Pass when a small Window's cost does not grow with the file, or the shortfall is known and a route around it is chosen.
3. **Containing C errors (H2). Done, in experiment 3: 5.2 million corrupt JPEG and JPEG 2000 files through libjpeg-turbo and OpenJPEG, and 4.4 million NITF and TIFF files through GDAL, with no crash, exit, hang, output, leak or change of process state, GDAL's own included. GDAL needed one more cap, on the size of a block, since it offers none on its working memory: proposed as M22, and accepted in specification version 73 (section 10.3).** One libjpeg-family library and GDAL behind BIL's C layer, with corrupt files from a fuzzer. Pass when no corrupt file crashes the process or leaves global state changed.
4. **Packaging (H2).** BIL as a Python wheel and a conda package, with private GDAL and PROJ, loaded beside rasterio and pyproj. Pass when both read correctly in one process on Linux, macOS and Windows.
5. **Borrowed arrays in Python. Done, in experiment 5: valid after the Source is closed and dropped, under eight threads, read-only under `Shared`, nothing leaked, on all four systems.** A borrowed NumPy array that outlives the Python object for its Source, under several threads. Pass when the mapping stays valid, the array is read-only under `Shared` mapping, and nothing leaks.

A sixth, for video (H4), should run before Tier 2 is planned, not before Tier 1.

## 9. If it has to be cut

If time runs short, these cuts keep BIL's purpose and remove the most risk.

| Cut | What is kept | What is lost |
| --- | --- | --- |
| AVX-512 paths | AVX2 and NEON | Some speed on the newest Intel servers |
| Isolation and external decoding | Limits, configured plugin locations and fuzzing | Protection from a library that a hostile file takes over |
| The Python interpreter for scripted drivers | Scripted drivers when BIL is called from Python | Scripted drivers for Rust and C++ callers |
| Most of Tier 3 | The formats users ask for | The claim to read nearly everything |
| C++ interface beyond the core calls | Opening, reading, metadata and geocoding from C++ | Advanced calls from C++, which Rust and Python keep |

BIL's claim in section 1.7 rests on its combination of features, so the cuts that matter least are those that reduce breadth, and the ones to avoid are those that break the read model, window independence or geocoding.

## 10. What version 68 means for the code already written

The code written so far is experiment 1 and 1b and the `bil` crate's vector layer and kernels. None of it reads files, holds a cache or reads metadata, so neither addition changes a line of it. The impact falls on the layers still to be built.

| Code | Access patterns | Domain profiles |
| --- | --- | --- |
| Kernels and the rolling pass (`crates/bil`, experiment 1b) | None. A pass already reads its rows top to bottom, which is storage order for strips and for tiles stored row by row, as section 12 rule 11 asks within one request. | None |
| Threads by bands of rows, the next step | Read-ahead adds a background thread beside the pass's threads, so the thread count must leave room for it, and both must be measured together | None |
| Fragment cache, still to be built | Must hold a read position for each Reader, evict passed blocks first under `Sequential`, and count blocks read ahead apart from those asked for | None |
| Reader and input layer, still to be built | A background Reader for `Parallel` and `OneHandlePerThread` drivers; merged byte ranges; `madvise`, `posix_fadvise` and Windows' sequential scan flag; errors held until the read that needs the block | None |
| GeoTIFF reader, Tier 1 | `describe_storage` from `TileOffsets` or `StripOffsets` and `PlanarConfiguration`. The order is by byte offset, which most files keep row by row, but not all. | Items such as `reference_system` provided by GeoTIFF keys |
| NITF reader, Tier 1, now written | `describe_storage` from `IMODE` (B, P, R or S), the block counts and the block mask table, which can leave blocks out: built, in file order | Field definitions name the items they provide, as before, now for more items |
| JPEG 2000 in NITF | Read-ahead decodes the next tiles or precincts of one codestream, so the codec must allow a second decoder on the same stream | None |
| Dictionary and field definitions, still to be built | None | Profiles are data over the dictionary: a JSON form, profile names resolved at load, and a report built from the item statuses that already exist |

**Two consequences for the design**

- **Band-sequential storage.** A pass reads every component of a row together. In band-sequential storage those lie in separate parts of the file, so storage order runs band by band while a pass runs row by row. For mapped, uncompressed data that costs nothing. For compressed band-sequential data, read-ahead must follow the blocks of every component the request uses. Section 4 of the specification now proposes that, and section 16 leaves it open with the other details of read-ahead.
- **Measuring.** Section 12 rule 11 asks for benchmarks of both patterns on strips, tiles and band-sequential files. They need real large files, so they wait for the Tier 1 readers and for files from the author.


## 11. What has been built, as of version 84

Tier 1's reading side is largely built. Everything below is in `crates/bil` and `crates/bil-sys`, tested in CI on Linux on x86 and ARM, macOS and Windows on every change, and on the latest stable Rust.

| Built | What it does | How it is tested |
| --- | --- | --- |
| Vector layer and kernels | `Scalar`, AVX2, AVX-512 and NEON back ends; the halvings, filters and transcendental functions of the pass | Experiments 1 and 1b's checks in CI, bitwise across instruction sets |
| NITF reader | NITF 2.0, 2.1 and NSIF 1.0: headers and extensions through field layouts, every IMODE and sample layout, masks, pad pixels, TRE overflow, several Images | Unit tests with 20,000 corrupt files; 22 real files and GDAL's own 19 agree with GDAL |
| JPEG 2000 codec | OpenJPEG through BIL's own bindings: one codestream an Image, accepted at open, regions and resolution levels, warnings as errors, a cut codestream refused by BIL's own walk of its tile-parts | Five fixture codestreams; three real JPEG 2000 Images agree with GDAL sample for sample |
| GeoTIFF reader | The `tiff` crate for directories, BIL for chunks: Deflate, LZW, PackBits, predictors, widths 1 to 64, samples as stored, GeoTIFF keys, no-data, overviews and SubIFDs as stored levels, reads in parallel | Unit tests with seeded corrupt files; GDAL 3.8.4's 232 TIFF test files: 160 Images and 9 stored levels agree pixel for pixel, none differ |
| Source interface | `open_source`, `OpenOptions`, one `Error`, `Source` and `Image`, `ImageInfo` with representation and component names, stored levels, storage by level, Fragments by level | Tests through the interface for both readers |
| `read_slice` for `pixel.raw` | Experiment 1b's pass, moved into the crate, run on Fragments read through the cache a band of rows at a time, at `level0`, `leveln`, stored levels and `storedk.leveln`, with missing pixels NaN; in Rust and C++ (M75 to M79, open) | Bit for bit against the pass on the whole Image in memory, from strips, tiles, Deflate, NITF blocks and a JPEG 2000 resolution level; timed against a copy and the arithmetic (`RESULTS-read-slice.md`) |
| Fragment cache | Blocks kept for each Source, least recently used first, bounded by count and bytes, one block held past the ceiling, a block decoded once among threads: GeoTIFF's chunks, NITF's uncompressed units, and JPEG 2000 tiles decoded one at a time at each level | Unit tests, threads included; both readers read through it, and the GDAL comparisons are unchanged by it. Decoding one tile a call costs 0.95 to 0.99 times one call for whole tiles at full resolution, and 1.14 to 1.35 times for a first read across tiles or at level 2 (`crates/bil/tests/RESULTS-jpeg2000-tiles.md`) |
| Warning store | Warnings recorded on the Source, kept once with a count and the first blocks, bounded, drained by `take_warnings` | Unit tests; the cache's ceiling, badly packed files and TIFF's skipped SubIFDs record through it |
| C++ interface | The shared library `bil`, exporting a narrow binary layer, and the C++20 header `bil/bil.hpp` over it: opening, Images, stored levels, storage, Fragments, the cache, warnings and failures, with Rust's names | A C++ test built with GCC, Clang and MSVC, every warning an error, on Linux, macOS and Windows, and under AddressSanitizer with leak checks |
| License and source checks | `cargo-deny` on what ships and on everything | CI |

**Experiments.** 1 and 1b (the fused loop), 3 (containing C errors, GDAL's part included) and 5 (borrowed arrays) are done; 2 is begun (section 8); 4, packaging, is not.

**Not yet built.** Slice functions beside `pixel.raw`, `SliceRequest` and its deliveries, and bands on threads in `read_slice`; mapping and borrowed Fragments; Readers and read-ahead; the metadata dictionary and profiles; geocoding; writers; the Python package's reading; the C++ interface beyond what the Rust code reads, and external plugins through it; and everything of Tiers 2 and 3.

**What writing it found.** Seventy-eight points the specification left open or stated twice, M2 to M79 in the user manual's chapter 24 (M1 and M10 became F6 and F7), each proposed to the author; M2 to M74 are ruled, and M75 to M79, from `read_slice`, are open, M72 otherwise than proposed: a sixth error kind, `Internal`, for BIL's own failures; M74 by its alternative, a `Block` carrying a list of byte ranges. None changed the verdict. The largest were in the readers' details: what fails an open, how a stored level is placed, and what a cache entry is.

## Sources

- [Rust's SIMD ecosystem, September 2026](https://zeli.app/story/49844629), summarizing the original at shnatsel.github.io, on `std::simd` remaining nightly-only.
- [FFmpeg trac ticket 5093](https://trac.ffmpeg.org/ticket/5093), on accurate seeking in MPEG program and transport streams.
- [libav-user: what's the ultimate correct way of seeking](https://ffmpeg.org/pipermail/libav-user/2014-October/007590.html), on the long-standing difficulty of exact seeking.
- [OpenJPEG on GitHub](https://github.com/uclouvain/openjpeg), on its maintenance status.
