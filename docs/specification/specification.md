# Blazing Imaging Library (BIL) Specification

Oct 16, 2026 · @Master

## 1. Purpose and scope

BIL extracts a processable subset of imaging sensor data, efficiently, from any format. It delivers that subset as native Fragments or as 2D float32 Slices.

### 1.1 Why BIL is needed

**Sensor data outgrows memory, and always will.** The size of a collection is a product of several things that each keep growing: rows, columns, bands, frames and bytes per sample. Memory grows too, but along one axis while sensors grow along five. By the time a machine can comfortably hold the data from today's leading sensor, the next sensor has arrived. Loading the whole image and then processing it therefore fails for exactly the data that matters most.

**The working form is larger than the stored form.** Algorithms want float32 or complex values, and they need several working copies.

| Data | Held whole as float32 |
| --- | --- |
| One band, 100,000 × 100,000 pixels | 40 GB |
| Hyperspectral cube, 10,000 × 10,000 pixels × 200 bands | 80 GB |
| One hour of 3840 × 2160 video at 30 frames a second, luma only | About 3.6 TB |

These figures are plain arithmetic at 4 bytes per value, not measurements of any sensor.

**A worked example: ARGUS-IS.** ARGUS-IS is a wide area motion imagery sensor, and a single sensor that makes the whole argument.

| Aspect | Figure |
| --- | --- |
| Frame | 1.8 gigapixels, stitched from 368 five-megapixel sensors behind four lenses |
| Rate and depth | 12 frames a second at 12 bits per pixel |
| Raw output | 32.4 GB a second |
| One frame held as float32 | 7.2 GB |
| One second held as float32 | 86 GB |
| What its users are given | Up to 65 separately steered windows, each about VGA size |

The sensor figures are as [reported in 2013](https://newatlas.com/argus-is-darpa-gigapixel-camers/26078/), and the two float32 rows are arithmetic from them. No one processes such a frame whole. The system's own designers gave users small windows cut from it, which is the subset model this specification is built on.

**The same holds in medicine.** A whole-slide pathology scan of 100,000 × 100,000 pixels in three colors, a representative size, is 30 GB as stored bytes and 120 GB as float32. Light-sheet microscopes produce volumes measured in terabytes. Pathologists and microscopists work the same way reconnaissance analysts do: they look at a region, at a chosen magnification, and never at the whole.

**The job is to extract a processable subset.** Each core concept in this specification cuts the data down along one dimension.

| Dimension that is too large | The cut | Concept |
| --- | --- | --- |
| Spatial extent | Take a rectangle | Window |
| Resolution | Take a coarser version | Level |
| Bands or vector components | Take one 2D scalar view | Slice |
| Time | Take one frame | Frame |
| Sample precision | Fix it at 4 bytes | float32 |

**Efficiently means that cost follows the request, not the collection.** Reading a small Window from an enormous file should cost about what reading it from a small file costs. The features in later sections are all instances of this.

- **Do not touch what was not asked for:** lazy opening, seeking, deferred video decoding and region decoding.
- **Do not copy what was asked for:** zero-copy Fragments and borrowed Slices.
- **Convert at most once:** the single fused pass.
- **Bound the memory:** streamed row blocks and caller-owned buffers.
- **Know the cost first:** `plan_slice` lets an algorithm budget memory before it reads.

**A subset must be correct as well as possible.** An algorithm that works through a huge image in pieces must get the same answer as if it had processed the image whole. The window-independence rule in section 5 guarantees this, and it is why slice functions are limited to a single pixel or a small fixed neighborhood.

**One interface in place of per-format code.** Every format solves the subset problem differently, through tiles, strips, chunks, key frames or pyramids, and some do not solve it at all. Without BIL, each algorithm either reimplements this for every format or gives up and loads everything. Section 1.7 sets out what already exists and the gap that remains.

### 1.2 Scope

BIL is a unifying layer. It calls the best open source library for each format and presents one interface over all of them. Its design is influenced by knowledge of previous requirements.

| Aspect | Decision |
| --- | --- |
| Core language | Rust |
| Other interfaces | A C-callable interface over the whole Rust library, delivered with it as one shared library; Python; and a C interface for drivers. Drivers can also be scripted in Python. |
| Backing libraries | Existing open source C and C++ readers, wrapped as drivers |
| Package names | `bil` on crates.io and PyPI; `bil-sys` for the native bindings |
| In scope | Reading pixels and metadata; preparing pixels as float32 Slices in the form analysis starts from (section 9); locating pixels in the world; writing a small set of simple formats |
| Not in scope | Writing every format BIL can read; analysis, and image processing beyond the preparation that section 9 defines |
| Medical use | BIL is not for diagnostic use. It reads medical data for research and processing, and makes no clinical measurement. |
| License | LGPL 2.1 or later. Plugins under a stricter license are optional and packaged separately (section 10.6). |

**Formats BIL must read**

- EO and IR stills: NITF, GeoTIFF, JPEG 2000 and other common rasters
- SAR: SICD, SIDD and other complex and detected products
- Multispectral and hyperspectral: ENVI, HDF5, NetCDF
- Raw camera files: DNG and manufacturer raw formats
- Video: compressed streams and files, including per-frame metadata

**Seven rules every part of the design follows**

1. **Proportional:** the cost of a read follows the data the request covers, never the size of the collection around it.
2. **Lazy:** nothing is decoded until a region is asked for.
3. **Faithful:** a Fragment delivers the native sample type and bit depth unaltered.
4. **Composable:** choosing an image, a frame, a window, a resolution and a slice function are separate steps that stack.
5. **Transparent:** the original metadata is always reachable in its native form, under its native names (section 14).
6. **Contained:** BIL reaches the network only for a location the caller gave it, and writes data only where the caller named a place (section 1.3).
7. **Uniform:** every format, sensor and data type passes through the same path and the same calls. A difference between sources appears as a reported property, never as a different call or a special case the caller must write (section 1.6).

### 1.3 Containment

BIL is used on closed networks and with data that must not leak. Two rules follow, and they bind BIL and every library it loads.

**No network unless asked**

- **Only for a location the caller gave.** BIL connects to the network only to read a remote Source, a stream or a remote terrain store that the caller named.
- **Nothing is fetched by default.** BIL fetches no default terrain, no grids and no updates, and it reports nothing about its use.
- **A reference inside a file is not a request.** Where a file points to remote data, BIL reports the reference and follows it only if the caller allows.
- **Backing libraries are held to the same rule.** PROJ's downloading of grids is turned off, so geoid grids are read from local files. XML parsing does not fetch external references.

**No copy unless asked**

- **Only at a place the caller named.** BIL writes to storage only for an output file, a companion file, a saved frame index, or mapped data with the lock file and control channel of its decoder process (section 10.2).
- **No files of its own.** BIL creates no temporary files and no cache files. The Fragment cache, blocks read ahead and held metadata exist in memory only.
- **One decoded copy is possible.** Mapped data is the only decoded copy BIL can leave on storage, and section 10.2 fixes how it is handled.

**The limit.** A plugin or a scripted driver is code the caller chose to load, and BIL cannot hold it to these rules. `list_drivers` reports every plugin that is loaded.

### 1.4 Build order

BIL is built in tiers. Each tier is finished before the next is started, so the core concepts are proven on a small base before the breadth is added.

**Tier 1 is GeoTIFF and NITF.** These two formats are enough to exercise the core.

| Part | In Tier 1 |
| --- | --- |
| Formats | GeoTIFF and NITF |
| Compression | Uncompressed data in both; Deflate, LZW and PackBits in GeoTIFF; JPEG 2000 in NITF |
| Core | Fragment, Slice and Window; Levels with `bilinear` and `decimate`; stored levels; the Fragment cache |
| Slice functions | `pixel.raw`, `complex`, `color` and `spectral.bands` |
| Geocoding | `image_window`, `affine` and `rational_polynomial`, with a constant-height `surface` |
| Metadata | Fields on demand for TIFF tags and for NITF headers and extensions; markings and companion files |
| Writing | TIFF and GeoTIFF, which the round-trip test needs |
| Interfaces | Rust, Python, and the C-callable interface of the shared library |
| Safeguards | Containment, resource limits and fuzz testing |

**What Tier 1 leaves out.** Video with its Readers and deferred Fragments, scripted drivers and their interpreter, external decoding with the mapped format and isolation, terrain files, and every other format.

**When a tier is finished.** Each of its drivers passes the conformance suite of section 10.4, and its slice functions meet the requirements of section 12.

**Later tiers**

| Tier | Adds |
| --- | --- |
| Tier 2 | ENVI; video with KLV; SICD and SIDD. With them come Readers, scripted drivers and the C interface for external plugins. |
| Tier 3 | The remaining formats of section 8 and the remaining libraries of Appendix A, each assessed case by case. Its order is not yet set. |

### 1.5 Foundations

Five choices carry the rest of the design. Each was tested in review and held, and later changes are measured against them.

| Foundation | What it says | Why the design rests on it | Set out in |
| --- | --- | --- | --- |
| Index and coordinate | Data is addressed by index and described by coordinate | An address is always a whole number, so reads stay exact and cheap. Time, range and position can be irregular without disturbing them. | Section 3.1 |
| Window independence | A Slice of a Window equals the same region cut from a Slice of the whole Image | An algorithm can work through an image of any size in pieces and get the answer it would get from the whole | Section 5 |
| Containers apart from codecs | A driver reads a container, and a codec chosen separately decodes the compressed blocks inside it | One codec serves every container that uses it, and a better codec improves them all at once | Section 10 |
| Geocoding as a chain | Every geo model is a chain of steps between named spaces | Any format's model fits the form, models join end to end, and BIL can add its own step for the Window and Level | Section 13.1 |
| Pixels by description | A scripted driver says where the pixels are, and the compiled core reads them | A new or unusual format costs a small script, with no loss of speed | Section 10.1 |

**How they are used.** A proposal that would break one of these is a change to the design itself, not a detail, and is weighed as one.

### 1.6 Uniformity

BIL is a unifying interface, so its design is symmetric: each thing is done one way, for every format and every sensor.

**One mechanism each**

| For | The one mechanism |
| --- | --- |
| Pixels as stored | A Fragment: pixels on one grid, in one sample type |
| Data on different grids or in different sample types | Separate Images, with their relation declared |
| Pixels for algorithms | A Slice: 2D float32, on the grid its Level sets |
| Reading | One request: any Slices, at any Levels, delivered any way |
| Naming a Slice | Three parts: family, function and name |
| Missing pixels | A validity mask on a Fragment, and NaN in a Slice, or the caller's `missing_value` |
| How a request turned out | One status, carried by every result |
| Values along an index | A field attached to that index, of which a coordinate is one view |
| Essential metadata in every format | The dictionary: each item provided, derived or taken from a companion file |
| Locating pixels | A chain of steps |
| Everything that is not pixels | Named fields, listed and then read |

**Two tests**

- **For a caller.** Code that uses BIL holds no branch on format or sensor type.
- **For the design.** A new case must fit an existing mechanism, or the mechanism is generalized. An exception is not added.

**Differences are reported.** Sources do differ: one can be borrowed and another must be decoded, and one decodes at once while another is deferred. Each difference is a property the caller can read, such as `is_borrowed` and `is_deferred`, and `plan_slice` reports them before a read.

**Uniform does not mean equal cost.** The interface is the same for a mapped float file and for compressed video. The cost is not, and BIL says so in advance.

**One deliberate asymmetry.** The axes of the data are not treated alike. Rows and columns are read as a range, through a Window. Frame and component are selected one at a time, by index or by name. This follows from the decision that every Slice is 2D, and it is one rule: everything beyond 2D is selected, never ranged. A read across frames is therefore outside BIL, and a caller reads frame by frame.

**One known duplication.** A Window `step` and the `decimate` level method both reduce resolution without filtering. Both are kept.

### 1.7 Existing work and the gap BIL fills

Existing projects were surveyed on 12 October 2026, and Appendix A.15 lists them. Several do part of what BIL does. None does all of it.

**The nearest projects**

| Project | What it shares with BIL | What it lacks for BIL's purpose |
| --- | --- | --- |
| GDAL | The widest format coverage, NITF included. Windowed reads, overviews, and RPC, geolocation array and control point models | A map-raster model of bands. No video. Limited complex SAR and sensor models. BIL uses it as a backing library (section 10). |
| GRDL | The same intelligence-imagery problem: readers for SICD, SIDD, CPHD and EO NITF, chip reads, and geolocation by affine, SICD, RPC, RSM and control point models | Pure Python, alpha quality, one maintainer. No video or medical data, and no compiled interface for other languages. |
| OxiGeo | A Rust core, and one open call over many formats | Map and cloud data only. No NITF, SAR, video or raw camera files (A.11). |
| OpenImageIO | One reading interface, a plugin per format, and tiles with MIP levels | Film and animation formats. No sensor formats and no geocoding. |
| SarPy, SARkit, SIX | Rigorous SAR formats and models | SAR only |
| ITK, Bio-Formats | Medical and microscopy readers | Their own fields only |
| libvips | Demand-driven reading in bounded memory | Processing of ordinary images. No sensor formats or models. |

**What none of them does.** BIL is the only one to combine all six of these.

1. **One subset model for every sensor.** Window, Level, Slice, Frame and float32 work the same way for a NITF still, a SICD, a hyperspectral cube, video with KLV and a medical volume. Each existing project serves one community.
2. **Faithful and processable at once.** A Fragment keeps the native samples. Named Slices give float32 views such as magnitude, hue, demosaiced color or a polarimetric term, made in one fused pass and independent of the Window. Other projects give stored bands or channels and leave the conversion to every algorithm.
3. **Video as imagery.** Frames, deferred decoding, chroma, KLV metadata and per-frame pose are handled through the same calls as stills. Video libraries do not geocode, and geospatial libraries do not read video.
4. **One geocoding chain for every format.** Map grids, RPC, camera, SAR and per-frame video models are all chains of the same steps, each tested against an established implementation (section 13.8).
5. **One compiled core for every language.** A Rust core with a C interface, delivered as one shared library, and a Python interface over it, all with the same behavior. GRDL and SarPy are Python only, OxiGeo has no C interface, and GDAL's interface is built around map rasters.
6. **Built for untrusted files.** Limits, configured plugin locations, and drivers that can be run in a separate process (section 10.3).

**The gap.** Today, a developer whose algorithm must take imagery from several kinds of sensor assembles several libraries. A typical set is GDAL for map rasters, SarPy or SIX for SAR, FFmpeg with a KLV parser for video, and ITK for medical volumes. The developer then writes code for each format to cut subsets, convert samples and locate pixels. BIL is the single fast layer beneath such algorithms that stays faithful to the sensor: one read request and one geocoding chain, callable from Rust, C, C++ and Python.

**What is not unique**

- **Format coverage.** GDAL leads, and BIL builds on it and on the other libraries of Appendix A rather than competing with them.
- **Python tools for intelligence imagery.** GRDL and SarPy already exist. BIL can sit beneath them as a fast reader.
- **Analysis.** It sits above BIL. BIL prepares data for analysis (section 9), and analysis is no part of BIL's claim.

**Risks to the gap.** GRDL is closest in purpose and is growing quickly. GDAL could add more sensor models. BIL's case therefore rests on the combination above, not on any single feature.

## 2. Naming and namespaces

BIL has one naming rule, the same in every language: types are UpperCamelCase, and every other name is snake\_case.

| Kind of name | Form | Examples |
| --- | --- | --- |
| Types, traits and enum values | UpperCamelCase | `Source`, `Fragment`, `SampleType::ComplexF32`, `Borrowed` |
| Functions and methods | snake\_case | `open_source`, `read_slice`, `list_slice_names` |
| Fields, parameters and options | snake\_case | `row_start`, `pixel_type`, `level_method` |
| Slice functions and Slice names | snake\_case, joined by dots | `color.hsv`, `color.hsv.hue`, `complex.polar.magnitude` |
| Level names and level methods | snake\_case | `level0`, `stored2`, `bilinear` |
| Geocoding steps and normalization functions | snake\_case | `affine`, `rational_polynomial`, `gain_offset` |
| Metadata field names | A snake\_case namespace, then the native name | `common.sensor_name`, `nitf.file_header.FTITLE` |
| Crate, package and namespace | lowercase | `bil` |

**Why types are capitalized.** Rust and Python both require UpperCamelCase for types and enum values. The Rust compiler warns about a type written any other way, just as it warns about a function that is not snake\_case. Following both rules is what makes BIL read as native code.

**Native names are the one other exception.** The part of a metadata field name that comes from a format is kept exactly as the format spells it, so that it can be looked up in the format's own documents (section 14.1).

**Acronyms and abbreviations** are written as words: `hh`, `db`, `ndvi`, `rcs`.

**In C.** C has no namespaces, so every name carries a prefix: `bil_` on functions, `Bil` on types, and `BIL_` on enum values, which are in upper case. Examples are `bil_read_slice`, `BilSource` and `BIL_COMPLEX_F32`.

**The C interface follows five conventions,** so that every call has the same shape.

| Convention | Rule | Examples |
| --- | --- | --- |
| Handles | Every object is an opaque handle. Its fields are read through accessors. | `BilSource *`, `BilImage *`, `BilSlice *`, `BilFragment *` |
| Status | Every function returns a `BilStatus`. Results come back through the last arguments. | `BIL_OK`, `BIL_ERROR`, `BIL_PARTIAL` |
| Ownership | What a call creates, the caller frees with the matching `bil_free_` call. A handle obtained from another, such as an Image from its Source, is valid while its parent is. A Fragment or Slice is created by its read, and lives until it is freed (section 4). | `bil_free_slice`, `bil_free_fragment`, `bil_close_source` |
| Data | Array data is reached through an accessor that gives a pointer, the size and the strides. The pointer is valid until the object is freed. | `bil_slice_data(slice, &values, &rows, &cols, &row_stride)` |
| Errors | The message for the last failed call on this thread is read with `bil_last_error`. | `bil_last_error(buffer, buffer_size)` |

- **Text and options.** Strings are UTF-8 and end with a zero byte. Options are passed as a struct whose first field is its size, so that new options can be added without breaking callers.
- **Callbacks.** A callback receives a context pointer the caller supplied. It must not let an exception or a `longjmp` cross into BIL, and BIL catches its own failures before they reach the caller.

**Spelling.** Every identifier, and this document, uses American spelling: `color`, `normalization`, `center`.

**No rule beyond the language's own.** A function is not required to start with a verb. A call that returns a property is named for what it returns, as in `level_count`, which is the Rust convention.

**The same call in each language**

```rust
let source = bil::open_source("scene.ntf")?;
let slice  = source.image(0)?.read_slice("complex.polar.magnitude", window)?;
```

```python
source = bil.open_source("scene.ntf")
slice  = source.image(0).read_slice("complex.polar.magnitude", window)
```

```c
BilSource *source;
BilImage  *image;
BilStatus status = bil_open_source("scene.ntf", &source);
if (status == BIL_OK) status = bil_image(source, 0, &image);
if (status == BIL_OK) status = bil_read_slice(image, "complex.polar.magnitude", &window, &slice);
```

**Stability.** The crate and the shared library follow semantic versioning. The C interface changes incompatibly only at a major version, and the plugin interface carries its own version (section 10). The Python package is built against Python's stable interface, so one build serves several Python versions. It is built on PyO3, the better tested route to that interface.

**The acronym.** BIL also names the Band Interleaved by Line raster layout, which BIL reads. The same letters are also used for burst illumination LIDAR, a sensor type BIL reads. Documentation spells out Blazing Imaging Library on first use.

## 3. Core concepts

BIL has two ways to deliver pixels: a Fragment, which is the native data, and a Slice, which is a 2D float32 array made from it.

&#91;embedded content: pixel path · file to Fragment to Slice\]

A driver opens the file and produces Fragments. The core applies a slice function to a Fragment to make one or more named Slices. Image processing code can take either.

| Concept | Definition |
| --- | --- |
| `Source` | One file, set of files or stream opened by a driver. |
| `Image` | One 2D pixel grid inside a Source, with a single pixel type and named components. A Source holds one or more. An Image may repeat along a frame axis (video, sequences). |
| `Window` | A rectangle of an Image: start row and column, size, decimation step and frame. |
| Frame | The nth complete 2D array of an Image, counted from 0. Usually one collection cycle of the sensor. |
| Index | A natural number that says where data is packed: row, column, component or frame. |
| Coordinate | A physical value with a unit, attached to an index: time, range, wavelength or ground position. |
| `Fragment` | The native pixels of a Window. Each pixel is a scalar or a vector. Zero-copy where the storage allows. |
| `Slice` | One named 2D float32 array for a Window. A single Fragment can yield several. |
| Level | A resolution level of a Slice. Level n holds the Image pixels whose row and column are multiples of 2ⁿ, made by a named downsampling method, `bilinear` by default. Level 0 is full size. |
| Slice function | A pointwise or local mapping, defined for one or more representations, from the Fragments of one Image, or of several related Images, to one or more named Slices. `color.hsv` yields `hue`, `saturation` and `value`. |
| Representation | What an Image's samples are, named with its parameters: for example intensity, color, a mosaic with its pattern, complex, spectral with its band coordinates, or a polarization camera with its angles. A driver declares it for each Image, and it decides which slice functions apply. |
| Field | One named item of metadata with its value, decoded when first needed and then held. Essential items share one name across formats, through the metadata dictionary (section 14). |
| `Driver` | A plugin that opens Sources of one format family and produces Fragments. |

**Representations.** A representation is named in snake\_case, as slice families are, with its parameters: `intensity`, `color` with its model (`rgb`; `ycbcr` with its luma coefficients; `lab` with its encoding; `cmyk`), `palette`, `complex`, `spectral`, and `vector` for samples whose meaning the file does not state. Complex samples are `complex` whatever the file declares, each component named `complex`. A component name that would repeat takes its place number, counted from 1, as a second red band in place 4 is `red4`; otherwise components are named `component1` onward.

| NITF IREP | TIFF Photometric | Representation | Component names |
| --- | --- | --- | --- |
| `MONO` | 0 (WhiteIsZero, with `white_is_zero`) or 1, one sample; none stated | `intensity` | `intensity` |
| `RGB` | 2 | `color` `rgb` | NITF: by IREPBAND, `R`, `G`, `B` as `red`, `green`, `blue`; TIFF: `red`, `green`, `blue` |
| `RGB/LUT` | 3 | `palette` | `index` |
| `YCbCr601` | 6, BT.601's luma coefficients unless YCbCrCoefficients says otherwise | `color` `ycbcr` | `luma`, `chroma_blue`, `chroma_red` |
| | 8, 9 and 10, the encoding as a parameter | `color` `lab` | `lightness`, `green_red`, `blue_yellow` |
| | 5 with InkSet 1, TIFF's default, and four samples | `color` `cmyk` | `cyan`, `magenta`, `yellow`, `black` |
| `MULTI` | 0, 1 or none stated, with more than one sample beside any ExtraSamples | `spectral` | NITF: by IREPBAND where it names a color, else `component`k; a numeric ISUBCAT is the band's wavelength in nanometers, when coordinates come |
| `NODISPLY`, `NVECTOR`, `POLAR`, `VPH`, any other | 5 otherwise, 32803, 34892, 32844, 32845, any other | `vector` | `component`k |

In TIFF, ExtraSamples 1 and 2 name their component `alpha`, and its other extra samples are `component`k. NITF `MONO` is `intensity` whatever its bands: the first is `intensity`, the rest `intensity2` onward.

**Why a Source holds several Images.** A vector pixel means all its components share one grid and one sample type. Data on different grids are separate Images: a panchromatic band beside lower-resolution multispectral bands, the image segments of one NITF file, or the luma and reduced chroma of video.

**Related Images.** A driver declares how Images relate to one another, so that a slice function can use them together (section 5).

| Relation | Meaning | Example |
| --- | --- | --- |
| Same grid | The Images line up pixel for pixel | Float32 range beside 16-bit intensity; an interferogram beside its coherence |
| Scaled grid | One Image covers the same scene at a whole-number reduction, with the position of its samples stated | Video chroma at half size beside luma |

**Calls on a Source and an Image**

| Call | Result |
| --- | --- |
| `image_count()` | How many Images the Source holds |
| `list_images()` | An `ImageInfo` for each Image, in order |
| `image(index)` | One Image, by its index, counted from 0 |
| `info()` | On an Image: its `ImageInfo` |

**`ImageInfo`** describes one Image, the same whether it comes from a driver or is read by a caller.

| Field | Meaning |
| --- | --- |
| `row_count`, `col_count` | The size of the Image, in pixels |
| `frame_count` | The number of frames, with whether it is `Known`, `AtLeast` or `Unknown` (section 3.2) |
| `pixel_type` | The sample type, component count, significant bits and bit shift (section 4), or none where BIL's types do not hold the samples, whose reads then fail |
| `component_count` | The number of components, given even where there is no `pixel_type` |
| `representation` | What the samples are, with its parameters (section 3, above) |
| `component_names` | The name of each component |
| `relations` | Each related Image: its index, the relation, and for a scaled grid the reduction, a whole number down and across, and the position of the other Image's first sample's center, in this Image's pixels, counted as a stored level's position is (section 5) |
| `storage` | How the pixels lie in the file: a `Storage` (section 4) with its block size, interleave and whether the block order is known, and no blocks listed; `describe_storage` lists them |
| `may_have_missing` | Whether the driver states that this Image can have missing pixels. Where it is false, no checks are made for them (section 5). |

### 3.1 Indexes and coordinates

BIL addresses data by index and describes it by coordinate.

| | Index | Coordinate |
| --- | --- | --- |
| Type | Natural number | 64-bit value with a unit |
| Says | Where the data is packed | What the data physically is |
| Used to | Address the data | Describe it, or search for it |
| Examples | Row, column, component, frame | Time, range, wavelength, ground position |

Every coordinate attaches to one index. A coordinate is a metadata field attached to that index, seen along the whole of it (section 14).

**One field, not two.** Where a dictionary item describes the same thing as a coordinate, the coordinate's field is that item. The wavelength coordinate of the component index is `common.band_center`, its polarization coordinate is `common.polarization`, and the time coordinate of the frame index at frame 0 is `common.collection_time`. A driver fills one field, never two.

| Index | Coordinates that can attach to it |
| --- | --- |
| Row and column | Ground position, through the geo model of section 13. Time, for line-scan sensors and rolling shutters. |
| Component | Wavelength for spectral bands, range for the bins of a return waveform, polarization |
| Frame | Time. Gate range, for range-gated sensors. |

- **Reads take indexes only.** A coordinate is never used to address data. Time in particular is not a regular quantity: frames are dropped and some sensors vary their rate, so the time of a frame is looked up, not calculated from its number.
- **Searching by coordinate returns an index.** Asking which frame is nearest a given time gives a frame number, which is then used to read.
- **Coordinates are 64-bit.** A 32-bit float cannot hold an absolute time to useful precision.

| Call | Result |
| --- | --- |
| `list_coordinates()` | The coordinates this Image has, with the unit and the index each attaches to |
| `read_coordinate(coordinate_name)` | The coordinate's values along its index |
| `find_index(coordinate_name, value)` | The index whose coordinate is nearest the value. In Python a time is passed as a `datetime`. |

### 3.2 Frames

A frame is an index, like row and column. It is the nth complete 2D array in an Image, and it is always a natural number. Time is a coordinate linked to it.

For a framing sensor, a frame is one physical collection cycle: one exposure and readout of the array. That is the usual case, but the definition rests on packing, because the cycle is not always a frame.

| Sensor | Its collection cycle | Where time attaches |
| --- | --- | --- |
| Framing camera, video | One exposure of the whole array | Frame |
| Burst illumination LIDAR | One burst and gate | Frame, together with the gate range |
| Line-scan, pushbroom | One line | Row |
| Rolling shutter | One row within a frame | Frame, with an offset per row |
| SAR | One pulse; an image is formed from thousands | Row or column of the formed image |
| Camera mosaic | One readout of every camera in the array | Frame |
| Interlaced video | One field, which is every second row | Row, alternating between two times |

**Frame count and the frame index.** Some streams carry no index, so the number of frames and the time of each cannot be known without reading the whole stream.

| Call | Result |
| --- | --- |
| `count_frames()` | The number of frames, and whether it is `Known`, `AtLeast` or `Unknown` |
| `build_frame_index()` | Reads through the stream once and records where each frame is and its time |
| `write_frame_index(location)` | Saves the index, so a later `open_source` that names it does not scan again |
| `latest_frame()` | The index of the newest frame received. For a file it is the last frame. |

- **Opening never scans.** A count may be `AtLeast` or `Unknown` when the Source is opened.
- **A saved index checks its stream.** A saved frame index records the stream's size and a checksum of its start. An index that no longer matches is refused, and the stream is scanned instead.
- **The index is built on first need.** That is the first call that needs a `Known` count, a time coordinate or a jump to a distant frame.
- **A live stream has no end.** Its count is `AtLeast`, and its frames are numbered from the first one received.
- **A live stream has one position.** All Readers of a live Source see the same frames. A Reader cannot hold a place of its own in a stream that cannot be read again, so a live stream ignores the access pattern of section 4, and `plan` reports that it does.
- **Past frames are `Missing`.** A request for a frame older than those still held is delivered with status `Missing`, never by waiting or seeking. A read can be given a `timeout` and a `cancel` handle (section 3.4), so it never waits without end.

**Additional index dimensions.** Where collection cycles nest, an Image may declare further named indexes. A sensor that sweeps 50 range gates once a second can declare `sweep` and `gate`. These are natural numbers too, and they always resolve to a frame number through `resolve_frame`. `Window` is unchanged: it still carries one `frame`.

### 3.3 Results and status

Every call returns a result, and everything that is delivered carries a status. This is the one way BIL says how a request turned out.

**A call either delivers or fails.** A call that cannot deliver returns an error and no value. Errors are a file that cannot be opened, a bad argument, a failed read, a limit exceeded and corrupt data. Every driver reports them through one error type, and a file that no driver recognizes is reported with the names of the drivers tried, in the order tried, such as `nitf` and `geotiff`. A driver's own reader, opened directly on a file it does not recognize, names itself alone.

**What is delivered carries a status**

| Status | Meaning | For pixels | For positions | For accuracy | For fields |
| --- | --- | --- | --- | --- | --- |
| `Complete` | Everything asked for is there | No pixel is missing | The position is sound | Every step has an error estimate | The value was found |
| `Partial` | Delivered, with some of it missing | Some pixels are missing, and are marked | Not used | Some steps have no estimate, and are named | Some items of a group could not be read |
| `Missing` | Nothing is there to deliver | Every pixel is missing | No surface was available at that place | No step has an estimate | The field has no value at that index |
| `NotConverged` | An iteration did not settle on an answer | Not used | The inverse did not converge | Not used | Not used |
| `Ambiguous` | More than one answer exists | Not used | As in the fold of a coordinate grid | Not used | Not used |

- **Per element, where elements differ.** A set of positions carries a status for each position. For pixels, the per-pixel form of the status is the validity mask of a Fragment and NaN in a Slice.
- **An unknown name is an error, and a known one with no value is `Missing`.** A field name that is not defined for this Source is an error. A defined field or a dictionary item that has no value here is delivered with status `Missing` and no value. The same holds along an index.
- **Corrupt data is an error by default.** A caller may ask for a corrupt block to be delivered as missing data instead, and the result is then `Partial`.
    - **What stops an open.** Opening fails where the structure that locates the Images cannot be trusted: a header or segment that runs past the end of the file, block counts that do not cover the Image, or uncompressed data larger than its segment holds. Damage within an extension area does not stop it: the extensions read before the damage are listed, their header's group is `Partial`, and the pixels can still be read.
    - **In TIFF,** the structure that locates the Images is the chain of IFDs and the chunk tables. Opening fails for a chain that loops or leaves the file, a missing chunk table, a table whose count disagrees with its Image, and a chunk past the end of the file. What a damaged table leaves out is not inferred. An IFD whose layout is sound but whose coding BIL cannot decode, such as an unknown predictor, does not stop the open: it is listed, and its reads fail (section 10, "When no codec accepts an Image"). A SubIFD that cannot be read is skipped, not a failure of the open: it is not in the chain that locates the Images, and holds at most an overview.
    - **Recovered data is corrupt data.** A library that decodes a damaged block by filling in what it lost, and warns, as libjpeg does, has met corrupt data. The warning is an error by default.
    - **The whole block is missing.** Under `corrupt_as_missing`, a block a library recovered is marked missing as a whole, since a library does not say which of its pixels it filled in.
- **A failure never becomes a crash.** A failure inside BIL is caught at the interface and returned as an error.
- **In C.** Every function returns a status code, and the value comes back through an argument.

### 3.4 Options and held data

Every option is listed here, with its default, and every kind of data BIL holds between calls has a stated limit and a report.

**`OpenOptions`**, given to `open_source`. In Rust it is a struct with public fields and a `Default`, opened as Rust's own `std::fs::OpenOptions` opens a file: `OpenOptions::open(path)`, or `open_from(reader)` for bytes from memory or any other reader. `open_source(path)` opens with the defaults. A caller sets the options it needs and takes the defaults for the rest (`..Default::default()`), so an option added later breaks no caller.

| Option | Default | Meaning |
| --- | --- | --- |
| `mapping` | `Shared` | `Shared`, `Private` or `Off` (section 4) |
| `max_image_pixels` | None | Refuse an Image larger than this (section 10.3) |
| `max_decoded_bytes` | None | Refuse a compressed block that decodes to more than this |
| `max_field_group_bytes` | None | Refuse a metadata group larger than this |
| `max_scans` | None, or 500 when `untrusted` is set | Refuse a progressive image with more scans than this (section 10.3) |
| `max_directories` | 100,000 | Refuse a file whose chain of directories, such as TIFF's IFDs, is longer than this (section 10.3) |
| `max_subdirectories` | 1,000 | Refuse an Image with more subdirectories, such as TIFF's SubIFDs, than this (section 10.3) |
| `untrusted` | False | Open the Source as untrusted, for `isolation` (section 10.3) |
| `companion` | The file's name with `.metadata` added | A companion file to read (section 14.2) |
| `frame_index` | None | A saved frame index to use instead of scanning (section 3.2) |
| `max_held_positions` | 1,000 | Index positions for which metadata is held (section 14.3) |
| `max_held_group_bytes` | None | Bytes of extracted metadata groups held |
| `corrupt_as_missing` | False | Deliver a corrupt block as missing data, with status `Partial` (section 3.3) |
| `remote_timeout` | 30 seconds | The longest one request to a remote Source may take (section 10.3) |
| `remote_retries` | 3 | How many times a failed remote request is tried again |
| `remote_merge_gap` | 1 MB | Byte ranges closer than this are read in one request |
| `trust_companion` | True, or False when `untrusted` is set | Read the companion file's values (section 14.2) |
| `access_pattern` | `Random` | How the caller will move through the Source: `Random` or `Sequential` (section 4) |
| `read_ahead_bytes` | 64 MB | Under `Sequential`, the most bytes of blocks read ahead of the caller, within the cache's limits |

**`SliceOptions`**, given to any read

| Option | Default | Meaning |
| --- | --- | --- |
| `level_method` | `bilinear` | The downsampling method (section 5) |
| `level_source` | `Computed` | `Stored` lets BIL start from a stored level it chooses |
| `decimate_first` | 0 | Halvings taken by striding before the level method |
| `missing_value` | None | A number to fill missing positions with, instead of NaN |
| `missing_weights` | `Equal` | How a Level weights the values below it where some are missing: `Equal` or `PixelCount` (section 5) |
| `timeout` | None | The longest a read may wait, as for the next frame of a live stream. A read that exceeds it is an error. |
| `cancel` | None | A handle the caller can trigger from another thread to stop the read, which then returns an error |

**`WriteOptions`**, given to `create_source`

| Option | Default | Meaning |
| --- | --- | --- |
| `keep_markings` | True | Write markings (section 11) |
| `keep_identity` | False | Write identity fields |
| `require_internal_markings` | False | Refuse to write a marked Source to a format with no internal place for markings (section 11) |

**Held data**

| What is held | Limit | Reported by |
| --- | --- | --- |
| Decoded Fragments | `set_cache_size`, 16 by default, and `set_cache_bytes`, a quarter of physical memory by default (section 4) | `describe_held` and `describe_cache` |
| Blocks read ahead | `read_ahead_bytes`, and they sit in the Fragment cache, within its limits (section 4) | `describe_held` and `describe_cache` |
| Metadata attached to an index | `max_held_positions` | `describe_held` |
| Extracted metadata groups | `max_held_group_bytes` | `describe_held` |
| Terrain tiles | None of their own: terrain is read through BIL's drivers, so its tiles sit in the Fragment cache of the terrain Sources | `describe_held` on those Sources |
| The frame index | None: it holds one entry for each frame | `describe_held` |

`describe_held()` reports, for a Source, every kind of held data with its count and bytes. `describe_cache()` is its short form for the Fragment cache.

## 4. Fragment

A Fragment is the native pixels of a Window, and it depends on nothing except pixel position and the stored vector values.

**Definition.** For a Window, element (r, c) of the Fragment is the stored pixel at absolute position (`row_start` + r × `step`, `col_start` + c × `step`) in the chosen frame. Values are not converted, scaled or reordered.

**Pixel type.** A pixel is `component_count` samples of one `SampleType`. Data that mixes sample types is held as separate Images that share a grid (section 3), which keeps every Fragment a simple strided view.

| Data | Pixel type |
| --- | --- |
| Intensity (panchromatic, monochrome, detected SAR) | 1 × `U8` or `U16` |
| Color | 3 × `U8` or `U16` |
| Multispectral, hyperspectral | N × `U16`, `I16` or `F32` |
| Complex SAR | 1 × `ComplexI16` or `ComplexF32` |
| Quad-pol SAR | 4 × `ComplexF32` |

- **Complex is a sample type, not two components.** This keeps the component axis free for polarization channels.
- **Sample types:** `U8`, `I8`, `U16`, `I16`, `U32`, `I32`, `U64`, `I64`, `F16`, `F32`, `F64`, `ComplexI8`, `ComplexI16`, `ComplexF32`, `ComplexF64`.
- **Significant bits are recorded separately.** A 12-bit sensor is delivered as `U16` with `significant_bits` = 12.
- **Samples stay where they are stored.** A Fragment does not shift samples. Where the significant bits sit above unused low bits, `bit_shift` records how many low bits are unused.
- **One-bit data is unpacked.** It cannot be borrowed, so it is delivered as `U8` with `significant_bits` = 1.
- **Other widths widen.** Integers of any other width up to 64 bits take the smallest of 8, 16, 32 and 64 bits that holds them, signed where the format says so, with `significant_bits` the stored width; 2- and 4-bit samples become `U8`. In TIFF such samples are a stream of bits, most significant first, whatever the file's byte order, as libtiff and GDAL read them. Complex samples map to the complex types. Floats of 24 bits are refused as unsupported.
- **Compressed samples are aligned to the right.** A codec delivers whole numbers whatever the container says of storage. A compressed Image of integers takes its type as above from the container's bits per pixel (NITF's NBPP), with `bit_shift` 0 and `significant_bits` the actual bits (NITF's ABPP); NITF's PJUST applies only to uncompressed storage. Coded samples wider than the container states disagree with it, and are corrupt.
- **Stored, not displayed.** Samples are delivered as stored, whatever the photometric interpretation: TIFF's WhiteIsZero is not inverted, extra samples of unspecified meaning are kept, and a palette Image delivers its indexes, with the palette among its fields.

**Where the significant bits sit.** Formats and decoders differ, and a Fragment reports what it was given.

| Data | Usual form | `bit_shift` |
| --- | --- | --- |
| 8-bit video | One byte per sample | 0 |
| 10 or 12-bit video from a software decoder | A 16-bit word with the value in the low bits | 0 |
| 10 or 12-bit video from a hardware decoder | A 16-bit word with the value in the high bits | 6 for 10-bit, 4 for 12-bit |
| NITF with fewer significant bits than stored bits | Either, as the file's justification field states | 0, or the number of unused low bits |
| Bit-packed data, once unpacked by BIL | The value in the low bits | 0 |

A Slice always works from the sample value: the stored word shifted right by `bit_shift`. Ten-bit video therefore gives 0 to 1023 in a `pixel.raw` Slice, whichever decoder produced it. The shift happens in the same loop as the conversion to float32.

**Position.** A Fragment carries its Window, so every pixel knows its absolute row and column in the full Image. Position-dependent normalization and sensor geometry both rely on this.

**Layout.** A Fragment is a strided view with a row stride, a column stride and a component stride. Pixel-interleaved (BIP), line-interleaved (BIL) and band-sequential (BSQ) storage are all described this way without rearranging the data. A decimation step is also expressed through strides where the format has no overviews.

**Zero-copy rule.** A Fragment borrows the stored bytes when all of these hold:

1. The pixel data is uncompressed.
2. The byte order matches the machine.
3. Samples are byte-aligned (not bit-packed).
4. The storage is memory-mapped or already in memory, and the Window's rows are evenly spaced within it.
5. Each sample starts at an address that is a multiple of its own size.

Otherwise the Fragment owns a buffer that was decoded once. The type is the same in both cases, and `is_borrowed` reports which applies.

**Borrowed data**

- **Read-only under `Shared` mapping.** Borrowed pixels cannot be changed, and in Python they appear as a read-only NumPy array. Under `Private` mapping they can be changed, and the changes stay in the caller's memory.
- **Shared within one Source under `Private`.** Every array borrowed from one Source shares its copy-on-write pages, so a change through one is seen through the others. A change while another thread reads the same pages is a data race in the caller's code, as for any memory the caller shares; BIL itself never writes them.
- **It keeps its memory.** A borrowed Fragment, and in Python its array, holds a reference to the mapped or read memory it borrows from for as long as it lives.
- **Closing a Source.** A Source closes when the last reference to it goes, which in Rust is when it is dropped. `close()` in Python, and `bil_close_source` in C, close it at a chosen moment: it frees what the Source holds, its cache, read-ahead, metadata and Readers, and in Python every later read from it is an error. Arrays borrowed from it stay valid, and keep only the memory they borrow, which goes with the last of them. In C, `bil_close_source` also frees the Source's handle, which must not be used again; Fragments and Slices already read from it stay valid until each is freed with its own `bil_free_` call.
- **A choice of mapping.** `mapping` in `OpenOptions` is `Shared` by default. `Private` maps the file copy-on-write, so a caller can change the array without touching the file. `Off` reads into owned buffers, for network file systems where mapping is unreliable.
- **Files that change are not supported.** Mapping a file while another process changes or shortens it is not supported, and on most systems it can end the reading process.

**One shape.** A Fragment is always native pixels on one grid, in one sample type, as a strided view. Two properties report how it came about.

| Property | Reports |
| --- | --- |
| `is_borrowed` | The pixels are the stored bytes themselves, not a copy |
| `is_deferred` | The pixels have not been decoded yet. Asking for them decodes them. |

**Deferred Fragments.** For video, reading a Fragment does not decode anything. The call only seeks to the right place and extracts the compressed data that the Window and frame need. For a streamed format, that means the packets from the preceding key frame up to the frame asked for. The Fragment then reports `is_deferred`. The video is decoded when a Slice is formed from the Fragment, and the decoder's output goes straight to float32.

- **Decoded on demand.** Asking a deferred Fragment for its native pixels decodes them at that moment. Nothing else about the Fragment changes.
- **Self-contained.** A deferred Fragment holds everything needed to decode it: its packets from the preceding key frame.
- **Decoded by its own Reader if it can be.** It is decoded by the Reader that made it, when that Reader is free and in place. Otherwise BIL decodes it with a decoder made for the purpose, at the cost `plan` reported. This holds on any thread.
- **Decoded once.** Once decoded, the Fragment and its cache entry are the same object, so a frame is never decoded twice for one Source.
- **A Window does not reduce the decoding.** Video decodes whole frames, starting from the preceding key frame. The Window reduces only the conversion to float32. `plan_slice` reports how many frames a read will decode.
- **Open to other formats.** Any driver may return deferred Fragments where decoding is costly.

**Readers.** Decoder state and library handles are held in a Reader, not in the Image.

- **One Reader for each thread.** A thread that reads an Image gets its own Reader, made on first use. The Image itself holds nothing that changes during a read.
- **Sequential reads are cheap.** A Reader keeps its place in the stream, so forming a Slice for the next frame decodes one frame, not the whole run from the key frame.
- **More on request.** `open_reader` gives a caller a further Reader, for example to hold two places in one stream. It takes an `access_pattern`, which overrides the Source's for that Reader alone.
- **A Reader reads like its Image.** A Reader offers every read call its Image offers, with the same arguments. A call on the Image uses the Reader of the calling thread; a call on a Reader uses that Reader.

**Sharing between threads**

| Object | Shared between threads |
| --- | --- |
| Source and Image | Yes. Anything they hold that changes, such as the cache and extracted metadata, is guarded inside BIL. |
| Reader | No. It belongs to one thread at a time, and may be handed to another. |
| Fragment and Slice | Yes, once delivered. A borrowed one keeps the memory it borrows wherever it goes. |

In Rust these are the `Send` and `Sync` rules, checked by the compiler. In C and Python the same rules are stated in the documentation.

**How far reads run in parallel** depends on the library underneath, and each driver declares it (section 10). `plan_slice` reports the declaration for a read.

**Fragment cache.** Compressed data decodes in whole blocks: a tile, a strip, a chunk or a video frame. BIL keeps recently decoded blocks, each as a Fragment, so a block that several reads share is decoded once.

- **What it saves.** Adjacent Windows that share a block, the margins BIL reads around a Window, and video frames that are read more than once.
- **One block, one entry.** An entry is one block as `describe_storage` lists it, keyed by its Image, level, frame, block row and column and the components it holds, and held whole, every pixel decoded, so any Window that touches it is cut from it.
- **Every block that is not borrowed.** Uncompressed blocks that are copied, for want of mapping or because their samples are unpacked or regrouped, are cached like decoded ones, so margins, terrain tiles and remote reads keep what they read.
- **Sized by count, with a ceiling in bytes.** The caller sets how many Fragments are kept. Zero turns the cache off, and the default is 16. A second limit, `set_cache_bytes`, caps the bytes held, by default a quarter of physical memory, or 1 GiB where BIL cannot find how much the machine has. Whichever limit is reached first applies, so very large blocks, such as wide-area motion imagery frames, cannot exhaust memory.
- **At least one block.** A block a read decodes that is larger than `set_cache_bytes` is held alone, past the ceiling, rather than decoded again by the next read that needs it: the ceiling gives way for one block, never more, and the next block that fits displaces it. It holds the block's memory until then. The caller is warned each time this happens, so it learns that the ceiling is below the blocks the file holds. A block read ahead never passes the ceiling.
- **Decoded once among threads.** Where several Readers need one block at once, the first decodes it and the others wait for it. A decode that fails is not cached, and a waiting read then decodes the block in turn, so every error arrives with a read of its own.
- **One cache for each Source.** Every Reader of the Source shares it, and it is freed when the Source is closed.
- **Least recently used goes first.** When the cache is full, the Fragment that has gone longest without use is dropped. Under `Sequential`, blocks the reading has passed go before that (below).
- **Memory follows the block size.** A count does not fix the memory used. `describe_cache` reports the bytes held, and `plan_slice` reports the size of the blocks a read will decode.
- **Borrowed data is never cached.** Uncompressed data that meets the zero-copy rule has nothing to decode.
- **Safe to hold.** A Fragment handed to a caller stays valid after its block has left the cache.

| Call | Result |
| --- | --- |
| `set_cache_size(fragment_count)` | Sets how many decoded Fragments this Source keeps |
| `set_cache_bytes(byte_count)` | Sets the most bytes of decoded Fragments this Source keeps |
| `describe_cache()` | The Fragments held and the bytes held; the limits; counted in blocks, the hits, the misses, those served from blocks read ahead, and the blocks held past the ceiling |
| `clear_cache()` | Frees every cached Fragment |

**Access patterns.** Callers move through very large Images in one of two ways, and BIL can read faster when it knows which. The caller says so with `access_pattern` in `OpenOptions`, or for one Reader through `open_reader`. Read-ahead is kept for each Reader, since each holds its own place.

| `AccessPattern` | The caller | What BIL does |
| --- | --- | --- |
| `Random` | Jumps from place to place, as a viewer or a search for targets does | Reads only the blocks a read needs, keeps the cache in order of last use, and tells the operating system the access is random, so it does not read ahead either |
| `Sequential` | Moves through adjacent blocks, normally forward in the order they are stored, as processing a whole Image Window by Window does | Reads and decodes the next blocks ahead of the caller, merges adjacent byte ranges into larger reads, drops passed blocks first, and tells the operating system the access is sequential |

- **A hint, never a change of result.** Values, statuses, sizes and errors are the same under either pattern. Only the time a read takes and the memory held between reads differ.
- **The storage order comes from the driver.** Each driver reports how its blocks lie in the file as a `Storage`, through `describe_storage`. Its first three parts are also the `storage` of `ImageInfo`, known when the Source is opened. Read-ahead follows its order. Where the order is `Unknown`, BIL reads nothing ahead and gives the operating system the hint alone.

| Part of a `Storage` | Content |
| --- | --- |
| `block_size` | Rows and columns of one block: a tile, a strip, a chunk, or the whole Image when it is not blocked |
| `interleave` | An `Interleave`: `Pixel`, `Line` or `Band`, as in a `PixelLayout` (section 10.1) |
| `order` | `Known` or `Unknown`, as a `StorageOrder` |
| `blocks` | Where the order is known, the blocks in the order they lie in the file: each with its Image, stored level, frame, block row and column, the components it holds, and its byte offset and length. For video, only the frames the frame index already covers, since opening never scans (section 3.2). |

- **One for each level.** `describe_storage` takes the level, as `read_fragment` does, and every block it lists carries that level. A TIFF overview's blocks are its own chunks. A JPEG 2000 Image's blocks are its codestream's tiles, each with the byte ranges of its tile-parts, and each decoded alone. A resolution level k is read from its Image's bytes, so it lists its Image's tiles, with a block size of its Image's halved k times, rounding up.
- **One record for every kind of driver.** A compiled driver fills it from its format's own tables. For a scripted driver, BIL derives it from the `PixelLayout`, whose `interleave` and `blocks` already say the same things.
- **What is read ahead.** After each read, BIL reads the blocks that follow the last block read in storage order, for the same Image, Level and components, and for video the frames that follow. In band-sequential storage, where each component's blocks lie apart, it reads ahead along each component the last read used. It stops at `read_ahead_bytes`, and never takes the cache past either of its limits: blocks read ahead count against `set_cache_size` as well as the byte ceiling, so with the cache turned off nothing is read ahead. It runs forward only, in storage order.
- **Decoded ahead too.** Blocks read ahead are decoded on a background thread into the Fragment cache, so the caller's next read finds them ready. Uncompressed blocks, which have nothing to decode, are only fetched, or for mapped files the operating system is asked to bring them into memory.
- **Within the driver's concurrency.** The background thread uses a Reader of its own under `Parallel` and `OneHandlePerThread`. Under `Serial` it takes the library's one lock like any other call, so it reads between the caller's calls rather than beside them.
- **Passed blocks go first.** Under `Sequential`, when the cache is full, a block that lies wholly before the last read in storage order is dropped before any other. Where several Readers of the Source read ahead, a block counts as passed only when every one of them has passed it, since the cache is shared. Blocks the last read shares with the next, such as a block row under adjacent Windows, are kept.
- **Errors arrive with the read that needs them.** A block that fails while read ahead is not reported then. It is read again when the caller asks for it, and any error is reported by that read, as it would be under `Random`.
- **Larger requests for remote Sources.** Under `Sequential`, byte ranges read ahead are merged as `remote_merge_gap` allows, and each request still counts against `remote_timeout` and `remote_retries` (section 10.3). Read-ahead reaches only the Source being read.
- **Reported.** `plan` reports the blocks a read will find read ahead and those it will read itself. `describe_held` and `describe_cache` report the bytes read ahead and how many reads they served.
- **It stops when it is not wanted.** Read-ahead stops when the Source is closed, when the Reader it reads ahead for is dropped, and when a read is canceled.
- **With external decoding.** Where a decoder process serves the Source (section 10.2), read-ahead asks it for the next blocks over the control channel, and it decodes them into free slots. A slot a reader has marked is never taken for read-ahead.
- **Not for live streams.** A live stream has one position for every Reader (section 3.2), so neither pattern changes how it is read. It ignores the pattern, and `plan` says so.
- **Within one request it is already done.** One request reads its own blocks in storage order and streams them through the pass (section 12). The access pattern concerns what happens between requests.

**Reading a Fragment.** `read_fragment(window, level)` returns the native pixels of a Window. `level` is `level0` by default, or the name of a stored level such as `stored2`, read as it stands. A computed Level is not a Fragment, because it holds new values, so naming one is an error. The Window is given in full-resolution rows and columns, as for a Slice. At a stored level, the Fragment's own `window` is in the stored level's rows and columns: the stored pixels it holds, those whose centers fall inside the Window. A Window that holds no center gives a Fragment of no pixels, not an error.

**Missing pixels.** Where part of the array was never recorded, a Fragment is still a full array, and its validity mask marks the missing positions. This is the same for every sample type. Where a format marks missing data with a fill value, the driver turns it into mask entries. Slices made from the Fragment are NaN at those positions.

- **NITF pad pixels.** Where a NITF block mask table states a pad value, a pixel is missing when every one of its samples holds that value, anywhere in the Image, whether or not its block is flagged as holding pad pixels.
- **GeoTIFF's no-data value.** `GDAL_NODATA` holds one value, as text, for every band. As for NITF's pad pixels, a pixel is missing when every one of its samples holds it. `nan` means NaN, which is missing anyway (section 3.3). For integers, a value the sample type cannot hold marks nothing. For float16 and float32 the value is rounded to the sample type, as GDAL does for float32, and a value beyond the type's range marks nothing; for float64 it must be held exactly.

**Illustrative shape** (not final)

```rust
pub enum SampleType {
    U8, I8, U16, I16, U32, I32, U64, I64,
    F16, F32, F64,
    ComplexI8, ComplexI16, ComplexF32, ComplexF64,
}

pub struct PixelType {
    pub sample_type: SampleType,
    pub component_count: u32,
    pub significant_bits: u32,
    pub bit_shift: u32,          // unused low bits below the value; 0 for most data
}

pub struct Window {
    pub row_start: u64,
    pub col_start: u64,
    pub row_count: u32,
    pub col_count: u32,
    pub step: u32,      // 1 = full resolution
    pub frame: u64,     // 0 for stills
}

pub struct Fragment<'a> {
    pub window: Window,
    pub pixel_type: PixelType,
    pub strides: [isize; 3],    // row, column, component, in bytes
    pub validity: Option<Mask>, // marks missing pixels; None when there are none
    pub status: Status,         // section 3.3
    // the pixels are reached through data(), which decodes them first if the Fragment is deferred
}
```

## 5. Slice

A Slice is one named 2D float32 array made from a Fragment by a slice function, and one Fragment can yield several.

**Definition.** Element (r, c) of a Slice is a function of three things only: the Fragment pixels in a fixed neighborhood of (r, c), their absolute rows and columns, and the function's parameters. For most slice functions the neighborhood is the single pixel at (r, c).

This rule has three consequences:

- **Window-independent:** slicing a Window gives the same values as slicing the whole Image and cropping, to within the tolerance below.
- **Lazy and parallel:** any Window can be computed alone, and rows can be computed in any order.
- **Predictable cost:** a Slice never needs more than one pass over its Window.

**Pointwise and local functions.** Most slice functions are pointwise. A local function declares a small fixed radius, and each output value may use the pixels within it. Full-resolution demosaicing is the main case: `cfa.demosaic` turns a Bayer mosaic into `red`, `green` and `blue` Slices at the full size of the Window. Upsampling video chroma is another.

**Margins.** For a local function or a downsampled read, BIL itself reads the extra rows and columns needed around the Window. Blocks decoded for a margin stay in the Fragment cache (section 4), so a neighboring Window does not decode them again. At the edge of the Image, pixels are mirrored by whole periods of any pattern the data has, so the sites of a color filter or a polarizer stay in step. The neighborhood is fixed to the Image grid, so results stay window-independent and the caller never pads a Window.

**How equal is equal.** Window independence, and agreement between instruction sets and machines, hold to a stated tolerance, not to the bit. A value differs by at most one unit in the last place of float32 for arithmetic, and by at most the bound each transcendental function states. Experiment 1 found that bitwise agreement can be had cheaply, and BIL keeps it where it costs nothing, but it is not required.

**The common scalar is float32.** Every Slice has the same element type regardless of sensor. Float32 is as wide as int32, has native vector multiply-add, divide and square root, and is what downstream FFT, convolution, statistics and GPU code expect. It holds every integer up to 16,777,216 exactly, which covers all 8 to 16-bit sensors without loss.

**Anatomy of a slice function.** Each one is up to three steps fused into a single loop:

1. **Select** the components it needs from the pixel.
2. **Normalize** them, if asked, using parameters from metadata or the caller (section 7).
3. **Map** to one float32 value for each Slice requested.

**Several Slices from one read.** A slice function declares the representations it accepts and yields an ordered set of one or more named Slices. It reads the Fragment of one Image, or the Fragments of several related Images. Each Slice is still a 2D float32 array.

| Representation | Slice function | Slices it yields |
| --- | --- | --- |
| Intensity | `pixel.raw` | `intensity` |
| Color | `color.rgb` | `red`, `green`, `blue` |
| Color | `color.hsv` | `hue`, `saturation`, `value` |
| Color | `color.ycbcr` | `luma`, `chroma_blue`, `chroma_red` |
| Complex SAR | `complex.polar` | `magnitude`, `phase` |
| Complex SAR | `complex.cartesian` | `real`, `imag` |
| Multispectral | `spectral.bands` | One per band, such as `blue`, `green`, `red`, `near_infrared` |

Which function to use depends on circumstance. The same color Fragment can be read through `color.rgb`, `color.hsv` or `color.ycbcr`, and each gives a different set of Slices.

**The Level sets the grid.** A Slice is the caller's chosen representation of a Fragment, and its Level is part of that choice. At a given Level, every Slice of an Image has the same size and maps a Slice position to an Image position in the same way, whichever slice function made it.

- **Some data is stored unevenly.** Video chroma at half size, the color sites of a mosaic sensor, the angles of a polarization camera and the fields of interlaced video each cover the Image at less than its full resolution.
- **The slice function evens it out.** It interpolates such data onto the grid of the Level asked for. At Level 0, `luma`, `chroma_blue` and `chroma_red` therefore line up pixel for pixel, and so do the four color sites of a mosaic.
- **Levels then work as for any Slice.** Coarser Levels are made from those values by the level method.
- **The stored samples stay available.** Interpolating never replaces the data. One color site, one polarizer angle or one field is read unaltered from the Fragment as a strided view with a `step` of 2, and stored chroma is read from its own Image.

**Slice names.** Every Slice has a human-readable name. The names are fixed when the slice function is defined for its representation, in one of two ways:

- **Specified:** the definition lists them. `color.hsv` always yields `hue`, `saturation` and `value`.
- **Enumerated:** the definition gives a rule that names them for each Image. `spectral.bands` takes band names from the metadata, and falls back to `band1`, `band2` and so on when the metadata has none.

**Names taken from metadata.** A name found in a file is made into a Slice name by one rule.

1. **Converted to snake\_case.** Letters become lower case, and every run of other characters becomes one underscore. "Near Infrared (B8)" becomes `near_infrared_b8`.
2. **Made unique.** Where a name repeats, every instance carries its number, counted from 1: `red_1`, `red_2`. A name that occurs once has no number. Field names follow the same rule, in brackets, except that a field in a counted run is always numbered (section 14.1).
3. **The original is kept.** Every Slice carries the name as the file wrote it, as its `label`.
4. **Wavelengths carry enough digits.** Bands named by wavelength take as many decimal places as are needed to tell them apart: `nm550`, or `nm549_6` and `nm550_4` where two bands would otherwise share a name.

**Units are stated, not named.** A Slice's unit is part of its definition and is never part of its name: `temperature`, not `temperature_k`.

Names are snake\_case, unique within their function and held in a stable order. A Slice can also be selected by its position in that order. A position is an index, so it counts from 0, as every index in BIL does. A repeat number is not an index, and counts from 1. The name is the primary key.

**Selecting a Slice.** A name has one form, and there are no short forms.

| What is named | Form | Examples |
| --- | --- | --- |
| A slice function | Two parts: family and function | `color.hsv`, `pixel.raw` |
| A Slice | Three parts: family, function and Slice name | `color.hsv.hue`, `pixel.raw.intensity` |

- **Always in full.** There are no aliases. A Slice is named with all three parts even where its function yields only one.
- **The general functions are a family like any other.** The functions of section 9.1 are in the `pixel` family.
- **Parameters never go inside the name.** A function that takes parameters is selected with a `SliceSelector`: the Slice name, and the parameters as named values. Any value can be passed this way, including arrays such as response curves.
- **A string is a selector with no parameters.** It is accepted wherever a `SliceSelector` is, and means the same.
- **A selector can name a function.** With two parts, as in `SliceSelector("pol.pair", a="hh", b="vv")`, it names a function and its parameters, for calls such as `read_slices` that take a function.

```python
hue   = image.read_slice("color.hsv.hue", window)

green = bil.SliceSelector("spectral.integrate.value", from_nm=500, to_nm=600)
band  = image.read_slice(green, window)

hv    = bil.SliceSelector("complex.polar.magnitude", component="hv")
```

| Call | Result |
| --- | --- |
| `list_slice_functions()` | Slice functions valid for this Image's representation |
| `list_slice_names(function_name)` | The ordered Slice names that function yields for this Image |
| `default_slice()` | The Slice used when the caller does not choose |
| `plan_slice(slice_name, window)` | Output size, access class, any stored level used, and the blocks or frames to be decoded, before any pixel is read |
| `read_slice(slice_name, window)` | One `Slice`, borrowed where possible |
| `read_slices(function, slice_names, window)` | A `SliceSet`: several Slices from one Fragment. `function` is a function name, or a `SliceSelector` that names a function and its parameters. |
| `read_slice_into(slice_name, window, buffer)` | Fills a float32 buffer the caller owns |
| `stream_slice(slice_name, window, callback)` | Hands the caller float32 row blocks in turn |
| `level_count()` | How many resolution levels this Image offers: Level 0 and every halving until the Image is one pixel along its shorter side |
| `list_level_methods()` | The named downsampling methods available |
| `list_stored_levels()` | The reduced-resolution data the file itself holds, each with its name, size, scale and method |
| `read_slice_levels(slice_name, window, level_count)` | Every resolution level from full size downwards, made in one pass |

**One request.** Every read is one `SliceRequest`, made with `read(request)`. The read calls in the table above are short forms of it.

| Part of a request | Choice |
| --- | --- |
| `slices` | One or more Slices, from one slice function or from several |
| `window` | The Window |
| `levels` | One or more Levels, by name: `level3`, `stored2`, `stored2.level1`. A whole number n is a short form of `leveln`. |
| `options` | The level method, level source, `decimate_first`, `missing_value` and `missing_weights` (section 3.4) |
| `delivery` | Returned as arrays, written into buffers the caller owns, or streamed as row blocks |

- **Any combination.** Several Slices, at several Levels, streamed, is one request like any other.
- **Still one pass.** The Fragment is decoded once, whatever the request asks for.
- **Planned the same way.** `plan(request)` reports what a request would deliver and cost. `plan_slice` is its short form for one Slice.
- **Where the result goes.** `read(request, target)` takes a target that matches the delivery. `Returned` needs none. `IntoBuffers` takes a mapping from each Slice name and level name to a float32 buffer of the size `plan(request)` reports. `Streamed` takes a callback.
- **Streamed blocks.** The callback is called once for each block of rows. Each block carries its Slice name, its level name, the grid rows it covers and its values. Blocks of one Slice and Level arrive in row order, and blocks of different Slices and Levels may interleave.
- **In Python.** A `SliceRequest` and its `SliceOptions` are made with keyword arguments, and what a returned request delivers is indexed by Slice name and level name together, as `result["complex.polar.phase", "level2"]`.
- **The short forms.** `read_slice` asks for one Slice at one Level. `read_slices` asks for several Slices. `read_slice_levels` asks for several Levels. `read_slice_into` and `stream_slice` choose the delivery.

```rust
pub struct SliceRequest {
    pub slices: Vec<SliceSelector>,
    pub window: Window,
    pub levels: Vec<LevelName>,  // ["level0"] asks for full resolution only; a number n stands for "leveln"
    pub options: SliceOptions,
    pub delivery: Delivery,
}

pub enum Delivery { Returned, IntoBuffers, Streamed }
```

```python
image = bil.open_source("photo.tif").image(0)

image.list_slice_names("color.hsv")          # ["hue", "saturation", "value"]
hue    = image.read_slice("color.hsv.hue", window)
slices = image.read_slices("color.hsv", ["hue", "value"], window)
value  = slices["value"]
```

**One Fragment, one pass.** `read_slices` reads and decodes the Fragment once and computes every requested Slice in the same pass. Slices that are not requested cost nothing, and work they share is done once: `hue`, `saturation` and `value` share one conversion.

**More than one input Image.** A slice function may declare several input Images, provided their relation is declared (section 3).

- **One Window, read from each.** BIL reads from every input Image the part that the Window covers, and passes the Fragments to the function together.
- **Still window-independent.** The function remains pointwise or local, so the rule at the head of this section holds.
- **Called on the first Image.** The caller asks the Image that the function is defined for. `list_slice_functions` offers the function only when every input Image is present.
- **Examples.** `depth.sensor` takes range from one Image and intensity from another. `insar.interferogram` takes coherence from a second Image. `video.planes` and `video.rgb` take luma from one Image and chroma from a second at half size. `pixel_gain_offset` already reads its calibration this way (section 7).

**Output size.** At Level n, a Slice holds the points of the Level's grid that fall inside the Window: the Image pixels whose row and column are multiples of 2ⁿ. At Level 0 that is every pixel of the Window. This holds for every slice function, and `plan_slice` reports the size in advance.

**Where a Slice sits.** Every Slice carries its `origin` and `scale`: the Image position, in pixels, of its first value, and the spacing of its values along rows and columns. Both are 64-bit. Geocoding reads them, so a Slice from any Level or stored level is located without the caller adjusting anything (section 13).

**Resolution levels.** Downsampling is built into the interface. Every Slice, from any slice function and any sensor type, is available at a series of resolution levels. Level 0 is full size, and each further level halves the rows and columns using a named downsampling method, which is bilinear interpolation unless another is chosen. Many algorithms that use BIL work across scales, so they take the levels from BIL instead of building their own.

| Setting | Meaning |
| --- | --- |
| `level` = 0 | Full resolution (the default) |
| `level` = n | Rows and columns each reduced by a factor of 2ⁿ |
| Default method | `bilinear`: weights 1 2 1, divided by 4, applied along rows and then columns at each halving |
| Also built in | `binomial5`: weights 1 4 6 4 1, divided by 16, with a radius of 2 pixels, for less aliasing |
| Alignment | The reduced grid is anchored to the Image origin: its values are centered on Image pixels whose row and column are multiples of 2ⁿ |
| Applied to | The float32 Slice values, after the slice function |

**All levels in one pass.** Each halving already produces the level above it, so one read returns every level from full size down to level n, for about a third more work than the full-size Slice alone. `read_slice_levels` returns them together, named `level0`, `level1` and so on. Bilinear demosaicing uses the same weighting and runs in the same pass, so a raw mosaic becomes a full set of color levels in one read.

**The limit of `bilinear`.** Its 1 2 1 weights still pass half the amplitude at the highest frequency the halved grid can hold, and finer detail folds back into the result as aliasing. It stays the default because it gives every level in one cheap pass. Where aliasing matters, `binomial5` passes a quarter of the amplitude at that frequency, for about twice the arithmetic, and also gives every level in one pass.

**Named downsampling methods.** The method is chosen by name. `bilinear`, `binomial5` and `decimate` are built in. `bilinear` is the default, and further methods can be added as extensions. Each is registered under its own name with a full specification, so a name always means the same arithmetic wherever BIL runs.

| A method specifies | Why |
| --- | --- |
| Name | snake\_case, such as `bilinear`; this is what a caller selects |
| Weights | The exact values applied along rows and columns at each halving |
| Radius | How far the weights reach, so BIL reads the right margin around a Window |
| Alignment | Whether reduced values sit on full-resolution pixels or between them |
| One-pass support | Whether every level can be produced in a single pass |

`list_level_methods` reports the methods available, and `level_method` in `SliceOptions` selects one. A method must use fixed weights tied to the Image grid, so results stay window-independent. `binomial5` is the first method added beyond the default. A 2×2 block average is an example of what a further extension might add.

Averaging suits most Slices but not all, so each Slice name declares its kind when the slice function is defined. The rule below applies to every method that filters.

| Kind of Slice | Examples | How it is reduced |
| --- | --- | --- |
| Linear quantities | `intensity`, `power`, `red`, `luminance`, band values | The chosen method |
| Angles | `phase`, `hue` | The chosen method applied to unit vectors, so wrap-around is handled |
| Labels and indices | `class.labels.value`, `palette.lookup.index` | Every second value kept, with no averaging. Since only the grid point's value is used, a read of coarse Levels alone reads only those points, whatever the level method. |
| Masks | `pixel.mask.valid` | 1.0 wherever any pixel under the level method's filter was valid, else 0.0: the mask averaged by the method, then marked where it is above 0. This holds whatever kind of Slice it is read beside, so beside labels or range it can differ from where a reduced value exists. `valid_fraction` is the averaged mask itself, the share of valid pixels weighted by the filter. |
| Range and depth | `depth.sensor.range` | The minimum of the 3×3 block centered on each point of the reduced grid, which favors the nearest surface. Averaging across a depth edge would invent a range that exists nowhere in the scene. |

For range, the block at each halving is centered on the reduced value's own pixel, so a range Slice sits on the same grid as every other Slice at that Level. Missing values are left out, and the result is NaN only when all nine are missing. The `x`, `y` and `z` Slices of `depth.points` are taken together from whichever pixel in the block has the minimum range, so a reduced point is always a real point.

Because levels are made after the slice function, reduced `power` and reduced `power_db` differ. `power` is the physically meaningful one to reduce.

**What a reduced read costs.** A value that summarizes an area can only be made by reading that area. A computed Level therefore reads all the full-resolution data under its Window, and an averaged view of a whole Image means processing the whole Image. Two other routes cost less, and each gives something up.

| Route | Reads | Cost follows | Gives up |
| --- | --- | --- | --- |
| Computed, with a filtering method such as `bilinear` | Every full-resolution pixel under the Window; for labels read at coarse Levels alone, only their grid points | The full-resolution area | Nothing. Values agree for every file and on every machine, to the tolerance of this section. |
| Computed, with `decimate` | Only the pixels kept | The output, where the storage can be strided | Filtering, so fine detail aliases |
| Stored | Reduced data the file already holds | The output | Control of the method. Values depend on how the file was made. |

**Decimation.** `decimate` is a built-in level method that keeps the Image pixel at each point of the reduced grid and reads no other.

- **It strides.** On uncompressed data BIL steps across the storage and touches only the pixels kept. On float32 data that can be borrowed, a decimated `pixel.raw` Slice is a strided view, so it is `Borrowed`.
- **Compressed data still decodes by block.** A block is decoded if it holds a kept pixel. Blocks that hold none are skipped, which begins once the spacing exceeds the block size.
- **Every kind of Slice is reduced the same way.** The value is the one at the grid point, whatever the kind, because no other pixel is read.
- **It shares the grid.** Decimated values sit at the same positions as bilinear ones, so Slices made by either method register with each other.
- **Its relation to `step`.** A `decimate` Level n holds the same values as a Window `step` of 2ⁿ that starts on the grid. `step` remains for spacings that are not powers of two. It never uses filtered or stored data.
- **Not combined.** A `step` above 1 and a Level above 0 cannot be used in one read. Such a request is an error.

**Decimating first.** For a large reduction, `decimate_first` in `SliceOptions` takes the first halvings by striding and leaves the rest to the level method. Level 8 with `decimate_first` = 5 keeps every 32nd pixel and then makes three bilinear halvings, for about a thousandth of the work.

**Stored levels.** Many files hold their own reduced-resolution data. BIL calls each one a stored level and keeps it distinct from a Level it computes.

| Where stored levels come from | Usual step between levels | Method |
| --- | --- | --- |
| TIFF and GeoTIFF overviews | 2 | The writer's choice, often an average |
| Whole-slide pyramids | 2 or 4 | The scanner's own |
| NITF reduced-resolution sets | 2 | The producer's own |
| JPEG 2000 resolution levels | 2 | The low-pass filter of the wavelet |

- **Listed.** `list_stored_levels` reports each stored level with its name, its size, its scale and position relative to full resolution, its method where the file states one, and whether it is aligned. Its position is where its first pixel's center lies, in full-resolution rows and columns, counting each full-resolution pixel's center as a whole number, so pixel i's center lies at position + i × scale.
- **Aligned or not.** A stored level is aligned when its samples sit exactly on the Level grid: at the Image pixels whose row and column are multiples of its scale. Many overviews are not. A typical TIFF overview averages pixels 2i and 2i + 1, so its samples sit half a pixel off.
- **TIFF overviews.** A reduced-resolution IFD that follows its Image, and an Image's SubIFDs, are both its stored levels, finest first. TIFF does not say how an overview was made, so its alignment is reported as unknown and treated as not aligned. An overview is taken to cover its Image's extent: its scale is the Image's size over its own, which need not be a whole number, and its position half a stored pixel in, scale/2 − ½. An overview whose number of bands differs from its Image's is not its stored level; its sample type may differ, and its Fragments report it. An overview with no `GDAL_NODATA` of its own takes its Image's. A mask IFD (NewSubfileType bit 2) is not an Image; it is read as validity when masks are read.
- **JPEG 2000 resolution levels.** Level k, for k from 1 to the fewest decomposition levels the main header's COD and COC markers state, is a stored level of ⌈rows/2^k⌉ × ⌈cols/2^k⌉, scale 2^k and position 0, and is aligned, since low-pass sample i lies on full-resolution pixel i × 2^k. None is listed where the codestream's grid does not start at the Image's corner. A tile-part's own COD or COC is not read, so a read of a level that tile-part lacks fails. Pad pixels are not marked at a reduced level: its samples are filtered, so one equal to the pad value says nothing of what was recorded.
- **Named.** Stored levels are named `stored1`, `stored2` and so on, from the finest to the coarsest. The scale is reported and is not part of the name, because stored levels do not always halve.
- **Read directly.** Naming a stored level, such as `stored2`, in a request's `levels` reads a Slice or a Fragment from it as it stands. The Window is still given in full-resolution rows and columns, and covers the stored pixels whose centers fall inside it.
- **Used to make a Level.** With `level_source` set to `Stored`, BIL starts from the coarsest stored level that is no coarser than the Level asked for, and makes the remaining halvings with the level method. A stored level qualifies only when it is aligned and what remains is a whole number of halvings. If none qualifies, the Level is computed from full resolution, and `plan` says so.
- **A misaligned stored level never makes a Level.** Every Slice at a given Level must share one grid, so a stored level that is not aligned is not used to make one. A name such as `stored2.level1` is an error when `stored2` is not aligned. It can still be read as it stands, as `stored2`, and its Slices carry their true `origin` and `scale`.
- **Only when asked.** The default `level_source` is `Computed`. BIL never substitutes stored data for computed data unasked. `level_source` exists only to let BIL choose a stored level itself; a caller who wants a particular one names it.
- **Reported.** `plan_slice` says which stored level a read will use, if any.

**Level names.** Every Slice reports how its resolution was made, so a stored value is never mistaken for a computed one.

| Level name | Made by |
| --- | --- |
| `level0` | Full resolution |
| `level3` | Three halvings computed from full resolution |
| `stored2` | The file's second stored level, as it stands |
| `stored2.level1` | One computed halving of that stored level |

`read_slice_levels` returns its Slices under the same names.

**Read options.** Options travel with any read as `SliceOptions`. The short forms also take `level` directly, by name or number.

```python
half   = image.read_slice("cfa.demosaic.green", window, level=1)
levels = image.read_slice_levels("pixel.raw.intensity", window, 4)   # level0 to level4

image.list_stored_levels()                         # stored1 at 1/4, stored2 at 1/16, ...
view   = image.read_slice("color.rgb.green", window, level="stored2")
finer  = image.read_slice("color.rgb.green", window, level="stored2.level1")
quick  = image.read_slice("pixel.raw.intensity", window, level=8, level_method="decimate")
```

```rust
pub struct SliceOptions {
    pub level_method: &'static str,  // "bilinear" unless another is chosen
    pub level_source: LevelSource,   // Computed unless Stored is chosen
    pub decimate_first: u32,         // halvings taken by striding before level_method; 0 by default
    pub missing_value: Option<f32>,  // None: missing positions are NaN; Some(v): filled with v
    pub missing_weights: MissingWeights, // Equal unless PixelCount is chosen
}

pub enum LevelSource { Computed, Stored }
pub enum MissingWeights { Equal, PixelCount }
```

**Missing data.** A Slice value with no data behind it is NaN. This covers three cases: pixels that were never recorded, pixels that were never imaged, and pixels the format marks with a fill value. Some formats store only part of the 2D array, so the unrecorded positions have no description in the file. When BIL unpacks such data to a full array, each of those positions still needs a number, and that number is NaN.

- **Every Slice of a missing pixel is NaN,** whichever slice function made it. `pixel.mask.valid` gives the same information as 1.0 or 0.0 for callers that prefer a mask, at Level 0 and beside every kind of Slice that is averaged. Beside labels or range at coarser Levels it follows the averaged mask (the table of kinds above), and the NaN positions of the Slice itself say where a value is missing.
- **Averaging skips missing values.** Resolution levels and local functions leave NaN out and rescale the remaining weights. A result is NaN only when every value that would contribute is missing.
    - **One average over the whole filter.** The weights are rescaled once, over the filter's rows and columns together, so a Level value is the average of the present values under the 2D filter, not an average rescaled after each pass along rows and then columns.
    - **A stored NaN is missing.** In float data, a stored NaN always counts as missing, exactly as if the validity mask marked it, whether or not the Image has a mask. A driver of a float format therefore sets `may_have_missing` unless the format guarantees there is no NaN, so the fast path without checks is kept only where it is safe.
    - **Two ways to weight a Level.** Each Level is made from the Level below it, and `missing_weights` in `SliceOptions` chooses how the values below are weighted where some are missing.
    - **`Equal`, the default.** Each present value below has its filter weight alone. A Level is the average of the present values of the Level below. This is the common case.
    - **`PixelCount`.** Each present value below has its filter weight times the share of present full-resolution pixels behind it. A Level is then the average of the present full-resolution pixels under the whole filter, as if it were made from full resolution in one step.
    - **The same where nothing is missing.** With every pixel present, both give exactly the values of an Image with no missing data.
- **A number in place of NaN, on request.** One NaN spreads through a Fourier transform or a convolution, and code of that kind does not expect it. `missing_value` in `SliceOptions` fills missing positions with a number the caller chooses. NaN remains the default.
    - **Filled last.** The fill is applied to the finished Slice, after the slice function and the Level, so averaging still leaves missing values out and every other value is unchanged.
    - **Still reported.** The status stays `Partial`. A caller who needs to know which positions were filled asks for `pixel.mask.valid` in the same request, or, beside labels or range at coarser Levels, reads the same Slice without `missing_value`.
    - **Not borrowed.** Filling writes new values, so a Slice with missing positions and a `missing_value` is not `Borrowed`.
- **No cost where nothing is missing.** A driver states whether an Image can have missing pixels, as `may_have_missing` in its `ImageInfo`. Where it cannot, no checks are made and the fast paths in section 6 are unchanged.
- **Fill values take a pass.** Replacing a format's fill value with NaN means computing new values, so that Slice is not `Borrowed`.

**Precision limit.** Every Slice is float32, whatever the Fragment holds, so algorithms that take Slices need only one representation.

| Fragment sample type | As a float32 Slice |
| --- | --- |
| `U8`, `I8`, `U16`, `I16`, `F16`, `F32`, `ComplexI8`, `ComplexI16`, `ComplexF32` | Exact |
| `U32`, `I32`, `U64`, `I64` | Exact up to 16,777,216, and rounded above it |
| `F64`, `ComplexF64` | Rounded to about seven significant digits |

- **Arithmetic in floating point, never in integers.** A slice function computes in float32 for sample types float32 holds exactly, and in float64 for `U32`, `I32`, `U64`, `I64`, `F64` and `ComplexF64`. It narrows to float32 once, at the end. The magnitude of a `ComplexF64` sample is computed in 64-bit, and the power of a `ComplexI16` sample cannot overflow.
- **Exact values come from the Fragment.** Callers who need the wider types exactly read the Fragment.

**Illustrative shape** (not final)

```rust
pub struct Slice<'a> {
    pub name: String,          // "hue"
    pub label: String,         // the name as the file wrote it, where it came from metadata
    pub status: Status,        // section 3.3
    pub level_name: String,    // "level0", "level3", "stored2"
    pub origin: [f64; 2],      // Image position of the first value, in pixels: row, column
    pub scale: [f64; 2],       // Image pixels between values: rows, columns
    pub window: Window,
    pub row_count: u32,
    pub col_count: u32,
    pub data: Cow<'a, [f32]>,  // borrowed when the source is already float32
    pub strides: [isize; 2],   // row, column, in bytes
}

// Several Slices made from one Fragment, found by name or position.
pub struct SliceSet<'a> { pub slices: Vec<Slice<'a>> }

pub enum SliceNames {
    Specified(&'static [&'static str]),         // color.hsv: hue, saturation, value
    Enumerated(fn(&ImageInfo) -> Vec<String>),  // spectral.bands: from metadata
}

pub struct SliceFunction {
    pub name: &'static str,              // "color.hsv"
    pub representation: Representation,  // the sensor representation it is defined for
    pub slice_names: SliceNames,
    // parameters and the fused loop follow
}
```

## 6. Intensity fast path

For intensity imagery, the `pixel.raw` slice is either the stored bytes themselves or one direct conversion from them, with no intermediate copy.

Intensity imagery is the simplest case: one component per pixel and no normalization. BIL treats it as a first-class path with three access classes. An access class describes the data and nothing else. How the result is delivered, returned, written into the caller's buffers or streamed, is a separate choice, and `plan_slice` reports both.

| Stored intensity data | Access class | Full-size copies made by BIL | How |
| --- | --- | --- | --- |
| Float32, native byte order, uncompressed | `Borrowed` | 0 | The Slice points at the stored bytes |
| Float32, one band of an interleaved cube | `Borrowed` | 0 | The Slice is a strided view of that band |
| 8 or 16-bit integer, streamed | `Converted` | 0 | Converted a row block at a time into a cache-sized buffer |
| 8 or 16-bit integer, returned whole | `Converted` | 1 (the output) | Converted straight from the mapped file into the output |
| Opposite byte order, or bit-packed | `Converted` | 1 (the output) | Swap or unpack in the same loop as the conversion |
| Compressed (JPEG 2000, JPEG, video) | `Decoded` | 1 (the output), beyond the decoder's own buffer | Converted as rows leave the decoder |

**The decoder's buffer.** A decoder always writes its output into a buffer of its own, and BIL cannot avoid that copy. The counts above are the copies BIL makes beyond it. Where a decoded block is kept in the Fragment cache, that buffer is the cached Fragment, not a further copy.

**Why integer data is never literally zero-copy.** Stored 8 and 16-bit bytes are not float32 values, so one conversion must run. BIL's commitment is that this conversion is the only work done.

**What rules out borrowing.** A local slice function or a filtered Level always computes new values, so it is `Converted` or `Decoded`, never `Borrowed`. A `decimate` Level of borrowable data computes nothing, so it stays `Borrowed`.

**Requirements**

1. `read_slice("pixel.raw.intensity", window)` on borrowable float32 data returns a borrowed Slice and touches no pixel.
2. Integer data is converted directly from the storage or the decoder's output into the destination. No intermediate Fragment buffer is created.
3. `stream_slice` blocks are sized to stay inside the processor cache, so the float32 form of a large image never occupies main memory. The consumer processes each block while it is still in cache.
4. `read_slice_into` reuses the caller's buffer, so repeated reads of video frames or tiles allocate nothing.
5. Outputs larger than the cache are written with streaming stores, so the destination is written once without first being read.
6. `pixel.raw` on 8 to 16-bit data returns exactly the sample values, as float32: the stored integers, shifted right by `bit_shift` where that is not zero.
7. `plan_slice` reports the access class before any pixel is read, so callers know the cost in advance.

**Memory.** The whole-array path makes the output four times the size of 8-bit data and twice the size of 16-bit data. The streamed path avoids this entirely.

**The same path serves other cases.** `pixel.raw` on float32 multi-band data yields one `Borrowed` Slice per band through strides. `video.planes.luma` converts directly from the decoder's luma plane.

## 7. Normalization

Normalization is a known function whose parameters come from the image's metadata or from the caller.

- **It is pointwise.** It uses only the pixel's values, its absolute row and column, the coordinates of its frame, and the values at the same position in a calibration Image.
- **It lives inside slice functions.** A Fragment is never normalized.
- **The core owns the functions; drivers supply the parameters.** A driver's job is to map its format's metadata onto the functions below.
- **Parameters can vary by frame.** A value such as the gate range of a LIDAR frame is a coordinate of the frame, and a normalization may use it.
- **Parameters can be images.** A calibration can supply a value for every pixel, as a non-uniformity table or flat field does.
- **Bad pixels are missing data.** A bad-pixel map marks pixels as missing, so they are NaN in every Slice.

**Function library**

| Function | Form | Used for |
| --- | --- | --- |
| `gain_offset` | a × value + b, per component | Radiance calibration, scale and offset, black level |
| `polynomial2d` | Polynomial in row and column | SAR radiometric scale factors that vary with range |
| `interpolated_grid` | Coarse grid of factors, interpolated by position | Calibration vectors, flat-field and vignetting correction |
| `lookup_table` | Value to value | Values the format stores through a table: palettes and amplitude tables |
| `matrix` | Linear combination across components | Band synthesis. Color conversion is done by the color functions of section 9.3 only. |
| `planck` | Inverse Planck law with sensor constants | Brightness temperature |
| `pixel_gain_offset` | A gain and offset for every pixel, read from a calibration Image | Infrared non-uniformity correction, flat fields, dark frames |
| `log10` | The base-10 logarithm of the value. Values of zero or less become missing data. | Data to be viewed on a logarithmic scale; decibels, with a `gain_offset` after it |
| `value_polynomial` | A polynomial in the stored value, of any order | Detector linearity correction; formats that store values through a polynomial |

**Motivating cases**

| Need | Function | Parameters from |
| --- | --- | --- |
| Radiometric correction of SAR across range | `polynomial2d` or `interpolated_grid` | Metadata |
| Atmospheric correction of multispectral bands | `gain_offset` per band | Supplied by the caller |
| Range correction of LIDAR intensity | Intensity multiplied by the square of range | The pixel's own range component, or the gate range of its frame |

**Parameters are fields.** A normalization's parameters are fields, as every other input of a function is. A driver maps them from native fields, and they are filled in the same order as dictionary items: the caller first, then the format, then a derivation, then a companion file (section 14.2). A missing calibration can therefore be supplied in a companion file. If a function's inputs are available from none of these, `list_slice_functions` does not offer it for that Image, and `list_required_fields` says which are missing.

## 8. Fragment case catalog

Every sensor and data representation considered so far maps onto a Fragment with a scalar or vector pixel. "Uncompressed only" means the zero-copy rule in section 4 can be met only when the file is stored uncompressed.

### 8.1 Optical stills

| Case | Native pixel | Zero-copy | Slice families |
| --- | --- | --- | --- |
| Panchromatic, monochrome | 1 × U8 or U16 | Uncompressed only | `pixel` |
| RGB, RGBA | 3 or 4 × U8 or U16 | Rarely; usually compressed | `color` |
| Palette | 1 × U8 index, plus table | Uncompressed only | `palette` |
| Bayer raw (RGGB, quad, X-Trans) | 1 × U16, plus pattern | Uncompressed DNG only | `cfa` |
| Multispectral, one grid | 4 to 16 × U16 | Uncompressed only | `spectral` |
| Multispectral, mixed resolutions | One Image per resolution | Uncompressed only | `spectral`, per Image |
| Hyperspectral cube | 100 to 500 × U16, I16 or F32 | ENVI yes; HDF5 if contiguous | `spectral` |
| Snapshot spectral mosaic (4×4, 5×5) | 1 × U16, plus pattern | Uncompressed only | `cfa` |
| Thermal infrared | 1 × U16 counts or F32 | Uncompressed only | `thermal` |
| Polarization camera | 1 × U16, plus angle pattern | Uncompressed only | `polcam` |
| Pushbroom, line-scan | As panchromatic or multispectral; rows are time | Uncompressed only | As above |
| Push-frame sensors | A frame of band strips; each strip sees different ground on each frame | Uncompressed only | `spectral` |
| Scanned film | 1 × U8 or U16, very large, with a panoramic camera geometry | Uncompressed only | `pixel` |
| Low-light and intensified cameras | 1 × U16 | Uncompressed only | `pixel` |

### 8.2 SAR and other coherent sensors

| Case | Native pixel | Zero-copy | Slice families |
| --- | --- | --- | --- |
| Complex single-look (SLC) | 1 × ComplexI16 or ComplexF32 | Only if byte order matches | `complex`, `sar` |
| Complex stored as amplitude and phase bytes | 2 × U8, plus amplitude table | Yes, but needs decoding | `complex`, `sar` |
| Dual or quad-pol SLC | 2 to 4 × complex, or one Image each | Only if byte order matches | `pol`, `complex` |
| Detected (GRD, SIDD) | 1 × U8 or U16 | Uncompressed only | `pixel`, `sar` |
| Interferogram | 1 × ComplexF32, plus coherence Image | Often | `insar` |
| Polarimetric covariance | 9 × F32 | Often | `pixel`, `pol` |
| Phase history (CPHD) | 1 × ComplexI8, ComplexI16 or ComplexF32; axes are pulse × sample | Only if byte order matches | `complex` |
| Sonar, ground-penetrating radar, ultrasound | 1 × U16, F32 or complex | Varies | `pixel`, `complex` |
| Inverse SAR and video SAR | 1 × complex per frame; the axes are range and Doppler | Only if byte order matches | `complex` |
| Burst-mode SAR | Complex bursts, held as frames or as separate Images | Only if byte order matches | `complex`, `sar` |
| Millimeter-wave and terahertz imagers | 1 × complex, or detected | Varies | `complex`, `pixel` |
| Side-scan and synthetic aperture sonar | 1 × U16 or complex; side-scan rows are pings in time | Varies | `pixel`, `complex` |
| Radar scans, imaging sonar, weather radar | 1 × U8 to F32 on a polar grid of range and beam | Varies | `pixel` |

### 8.3 Video and sequences

Extraction is deferred: the frame is part of the Window. Reading a Fragment only seeks to that frame and extracts its compressed data, and the frame is decoded when a Slice is formed (section 4).

| Case | Native pixel | Zero-copy | Slice families |
| --- | --- | --- | --- |
| YUV 4:2:0 or 4:2:2 | Two related Images: luma at full size, 1 × U8 or U16, and chroma at reduced size, 2 × U8 or U16 | Deferred; the decoder's buffer is used without a copy | `video` |
| RGB or 4:4:4 video | 3 × U8 or U16 | Deferred; the decoder's buffer is used without a copy | `color` |
| Monochrome or infrared video | 1 × U8 or U16 | Deferred; the decoder's buffer is used without a copy | `pixel`, `thermal` |
| Raw cine | Bayer per frame | No | `cfa` |
| Full-motion video with KLV | As video, plus per-frame metadata | Deferred; the decoder's buffer is used without a copy | `video` |
| Sequences, bursts, multi-page TIFF | Any still case × frames | As the still | As the still |
| Wide-area motion imagery | Very large monochrome or RGB frames, from tens of megapixels to over a gigapixel, at 1 to 12 frames a second | No; JPEG or JPEG 2000 blocks | `pixel`, `color` |
| Live streams | Any video case, forward-only | Deferred; the decoder's buffer is used without a copy | `video` |
| Interlaced video | Two fields per frame, captured at different times | Deferred; the decoder's buffer is used without a copy | `video` |
| Fisheye and 360° cameras | As video, with a fisheye or equirectangular projection | Deferred; the decoder's buffer is used without a copy | `video`, `color` |
| Endoscopy and surgical video | As video; sometimes stereo, or with a fluorescence channel | Deferred; the decoder's buffer is used without a copy | `video`, `color` |

### 8.4 Derived and non-camera rasters

| Case | Native pixel | Zero-copy | Slice families |
| --- | --- | --- | --- |
| Elevation | 1 × I16 or F32 | GeoTIFF uncompressed; DTED no | `elev` |
| Classification, labels | 1 × U8 or U16 | Uncompressed only | `class` |
| Bit masks | 1 × U8 with 1 significant bit, unpacked from 1-bit storage | No; bit-packed | `pixel` |
| Scientific float products | 1 to N × F32 or F64, with scale and fill | Contiguous only | `pixel` |
| Depth and range images | 1 × U16 or F32 for range, with intensity as a second Image on the same grid where its type differs | Uncompressed only | `depth` |
| Burst illumination LIDAR, range-gated | 1 × U16 intensity per frame, each frame with a gate range | Uncompressed only | `pixel`, `depth` |
| LIDAR with a return waveform per pixel | N × U16, one component per range bin | Uncompressed only | `depth`, `pixel` |
| Stereo, multi-camera | One Image per camera | As the still | Per Image |
| Volumes, z-stacks | Any case × depth × time | As the still | As the still |
| Medical volumes (DICOM, NIfTI, NRRD, MetaImage) | 1 × I16, U16 or F32, × depth × time | Uncompressed NRRD, MetaImage and NIfTI | `pixel` |
| Resolution pyramids | Any case, with each reduced version read as a stored level (section 5) | As the still | Any |
| Sparsely recorded arrays | Any case, with some pixels never recorded or never imaged | No; unpacked to a full array | Any; missing values are NaN |

### 8.5 Medical and microscopy

| Case | Native pixel | Zero-copy | Slice families |
| --- | --- | --- | --- |
| X-ray radiography, fluoroscopy, mammography | 1 × U16, of 10 to 16 bits, sometimes stored inverted | Uncompressed only | `pixel` |
| CT projections | 1 × U16 or F32 per frame, each frame with a projection angle | Uncompressed only | `pixel` |
| MRI k-space | N × ComplexF32, one component per receive coil | Often | `complex` |
| Diffusion and functional MRI | Any MRI case, with additional index dimensions | As the volume | As the volume |
| Nuclear medicine: PET, SPECT, gamma camera | 1 × U16 or F32 counts | Uncompressed only | `pixel`, `nuclear`; SUV is in the extras package |
| Ultrasound | 1 × U8 once converted to a picture; or raw lines on a polar grid, real or complex | Uncompressed only | `pixel`, `complex` |
| Optical coherence tomography | N × U16 or F32, one component per depth sample; or complex | Uncompressed only | `pixel`, `complex` |
| Whole-slide pathology | 3 × U8, gigapixel, tiled with a resolution pyramid | No; JPEG or JPEG 2000 tiles | `color` |
| Light-sheet and confocal microscopy | 1 to N × U16 × depth × time, up to terabytes | Uncompressed, or chunked | `pixel`, `spectral` |
| Fluorescence lifetime imaging | N × U16, one component per time bin | Uncompressed only | `pixel` |
| MR spectroscopy | N × ComplexF32, one component per frequency | Often | `complex`, `pixel` |
| 4D electron microscopy | 1 × U16 diffraction pattern at each scan position | Uncompressed, or chunked | `pixel` |

### 8.6 Security screening

| Case | Native pixel | Zero-copy | Slice families |
| --- | --- | --- | --- |
| Dual-energy X-ray baggage scan | 2 × U16: a high-energy and a low-energy value | Uncompressed only | `pixel` |
| Screening CT | As medical volumes, in the DICOM-based DICOS format | As the volume | `pixel` |

## 9. Slice function catalog

Each slice function is defined for one sensor representation and declares the named Slices it yields. A Slice is selected as `family.function.name`, such as `complex.polar.magnitude`.

- **Specified names** are listed in full below.
- **Enumerated names** are described by their rule and marked *enumerated*.
- **Local functions** are marked *local*. All others are pointwise.
- **Decibels.** A Slice in decibels sits beside its linear Slice in the same function, named with `_db`, as `power_db` sits beside `power`. The `log10` normalization of section 7 remains for formats that store values on a log scale.
- **Choosing a component.** On an Image with several components, a function defined for one component takes that component's name as a parameter, shown here as `complex.polar(hv)`.
- **Parameters.** Brackets in the tables below show a function's parameters. They are passed as named values in a `SliceSelector` (section 5), never as part of the name.
- **Families belong to whoever defines them.** The core families are open to packages of the BIL project, such as `bil-extras`. Any other plugin registers its functions under a family of its own.

**What belongs in the catalog.** Sensors store data in the form that suits the sensor. BIL delivers it in the form that analysis starts from. A slice function belongs in BIL when it does one of three things.

| Kind | What it does | Examples |
| --- | --- | --- |
| Undo the storage | Brings the data onto a regular grid with every component present | Demosaicing, chroma upsampling, fields, unpacking, palettes, class labels and members |
| Calibrate | Turns sensor values into a physical quantity of the scene | Radiance, reflectance, backscatter, temperature, height, range, activity |
| Re-express | Decomposes the measurement into the standard form that analysis of that kind of data begins with | Color spaces, complex parts, Stokes terms with degree and angle of polarization, Pauli components, the peak along an axis |
| Present | Maps values for display, with the statistics it needs passed in by the caller | A display stretch |

A function that presents must be pointwise and take every statistic it uses from the caller, so it stays independent of the Window. A function that re-expresses must pass four checks:

1. **The data's own values only.** It brings in nothing from outside the collection: no reference spectrum, scene model or analyst's threshold. Parameters the caller names explicitly, such as weights or response curves, are allowed: they are part of the request, not information BIL brings in.
2. **A standard definition.** It has one accepted formula, with named parameters at most.
3. **General to the data type.** It is a first step for almost any analysis of that kind of data, not for one application.
4. **Local.** It is pointwise or uses a small fixed neighborhood, so it stays independent of the Window (section 5).

- **Preparation is still local.** Work that needs the whole scene, such as speckle filtering, atmospheric correction or orthorectification, is outside BIL even where it prepares data (section 15).
- **The interferogram.** `insar.interferogram` re-expresses an interferogram already formed: one complex Image with its coherence beside it (section 8.2). Forming an interferogram from two collections is image formation, outside BIL (section 15).
- **General tools.** `pixel.pair` and the other functions of the `pixel` family are general tools that take the components the caller names. The named indices, such as NDVI, stay in the extras package.
- **Analysis goes to the extras package.** A function that derives new information about the scene, such as an index, a comparison with a reference or a clinical measure, is analysis. Those that are widely useful are provided by the extras package (section 9.6).

### 9.1 Any representation: the pixel family

| Slice function | Slices it yields | Computes |
| --- | --- | --- |
| `pixel.raw` | One per component, named from the Image: `intensity`; or `red`, `green`, `blue`; or band names; else `component1` onward (*enumerated*) | Stored values, unchanged |
| `pixel.scaled` | The same names as `pixel.raw` | Metadata gain and offset applied |
| `pixel.reduce` | `mean`, `sum`, `min`, `max`, `norm` | Reductions across the components of one pixel |
| `pixel.weighted(weights)` | `value` | Dot product of the pixel with supplied weights |
| `pixel.pair(i, j)` | `ratio`, `normalized_difference` | i / j, and (i − j) / (i + j) |
| `pixel.mask` | `valid`, `saturated`, `valid_fraction` | 1.0 or 0.0; and, at coarser Levels, the share of valid pixels, weighted by the level method's filter |
| `pixel.nearest(coordinate_name, value)` | `value` | The component whose coordinate is nearest the value |
| `pixel.integrate(coordinate_name, from, to)` | `value` | The average of the components whose coordinates lie between two values |
| `pixel.peak(coordinate_name)` | `value`, `coordinate` | The largest component, and the coordinate at which it occurs |
| `pixel.stretch(low, high)` | `value` | (value − low) / (high − low), limited to 0 to 1, with the limits passed in by the caller: a display stretch |


### 9.2 Complex, SAR and polarimetry

| Slice function | Slices it yields | Computes |
| --- | --- | --- |
| `complex.cartesian` | `real`, `imag` | The two parts of the sample |
| `complex.polar` | `magnitude`, `phase` | Square root of power; angle in radians |
| `complex.power` | `power`, `power_db` | Real squared plus imaginary squared; 10 × log10 of that |
| `sar.calibrated` | `sigma0`, `beta0`, `gamma0`, `rcs`, and `sigma0_db`, `beta0_db`, `gamma0_db`, `rcs_db` | Calibrated backscatter, from metadata polynomials or tables, and the same in decibels |
| `pol.channels` | One per channel, named from the Image: `hh`, `hv`, `vh`, `vv` (*enumerated*) | Magnitude of each channel |
| `pol.pauli` | `single_bounce`, `double_bounce`, `volume` | Pauli decomposition magnitudes |
| `pol.span` | `span` | Total power across channels |
| `pol.pair(a, b)` | `ratio`, `phase_difference` | Power ratio and phase difference between two channels |
| `insar.interferogram` | `phase`, `magnitude`, `coherence` | Wrapped phase and magnitude of a formed interferogram. Coherence comes from a second Image on the same grid, declared related in the same Source. |

### 9.3 Color, video and mosaic sensors

| Slice function | Slices it yields | Computes |
| --- | --- | --- |
| `color.rgb` | `red`, `green`, `blue`, plus `alpha` when present | The color channels |
| `color.hsv` | `hue`, `saturation`, `value` | Hue, saturation and value, from the encoded values as stored |
| `color.ycbcr` | `luma`, `chroma_blue`, `chroma_red` | Gamma-encoded luma and color difference, by the declared matrix and range |
| `color.lab` | `lightness`, `green_red`, `blue_yellow` | Perceptual lightness and opponent colors, from linear values, relative to the white point of the declared primaries |
| `color.luminance` | `luminance` | Linear luminance, with weights from the declared primaries |
| `palette.lookup` | `index`, `red`, `green`, `blue`, `luminance` | The index, and the looked-up color |
| `video.planes` | `luma`, `chroma_blue`, `chroma_red` | Luma, with chroma from its own Image upsampled to the luma grid (*local*) |
| `video.rgb` | `red`, `green`, `blue` | Color from luma and upsampled chroma, using the matrix and range the stream declares (*local*) |
| `video.fields` | `top`, `bottom` | The two fields of an interlaced frame, each interpolated to the full height (*local*) |
| `cfa.mosaic` | `stored`, `linear` | The mosaic as stored; with black and white levels applied |
| `cfa.demosaic(method)` | `red`, `green`, `blue`, `luminance` | Color interpolated from the mosaic at full resolution (*local*) |
| `cfa.sites` | One per color site, named from the pattern: `red`, `green1`, `green2`, `blue` (*enumerated*) | Each site alone, interpolated to the full grid from its own samples (*local*) |
| `cfa.binned` | `luminance` | The four color sites nearest each pixel, combined (*local*) |
| `polcam.angles` | `angle0`, `angle45`, `angle90`, `angle135` | Each polarizer angle alone, interpolated to the full grid (*local*) |
| `polcam.stokes` | `s0`, `s1`, `s2` | Stokes terms, from the four angles interpolated to the full grid (*local*) |
| `polcam.linear` | `degree`, `angle` | Degree and angle of linear polarization, from the same interpolated angles (*local*) |

**One meaning of `luminance`.** Every Slice named `luminance` is linear luminance from the declared primaries, or from the defaults below where none is declared. That includes those of `cfa.demosaic`, `cfa.binned` and `palette.lookup`.

**Color conversion is done here only.** The color functions of this section are the one route from one color representation to another. The normalization functions of section 7 do not convert color.

**How color is described.** BIL describes color with the four properties of ITU-T H.273, the coding-independent code points that video, AVIF and HEIF already use. It adopts the vocabulary, not a library.

| Property | Says | Used by |
| --- | --- | --- |
| `color_primaries` | The red, green and blue primaries and the white point | `color.lab`, `color.luminance` |
| `transfer_characteristics` | The curve between encoded and linear values | `color.lab`, `color.luminance` |
| `matrix_coefficients` | The YCbCr matrix | `color.ycbcr`, `video.rgb` |
| `video_full_range` | Whether values use the full range or the reduced video range | `color.ycbcr`, `video.rgb` |

- **From metadata first.** Each property is a dictionary item (section 14.2), provided by a stream's own code points or by a format's color tags where they are plainly the same.
- **Then a stated default.** For stills, BT.709 primaries with a D65 white point, the sRGB transfer curve, the BT.601 matrix that JPEG uses, and full range. For video, BT.709 primaries, transfer and matrix, and the reduced video range.
- **Reported.** `describe_field` says whether each property came from the file or from the default.
- **Computed by formula, in the fused loop.** Matrices and transfer curves are evaluated in BIL's own loops (section 12). There are no per-pixel lookup tables and no per-pixel calls into a library.
- **Tested against a reference.** Each color function is tested against the colour-science formulas, as geocoding steps are tested against theirs.

**Color profiles are not applied.** Most imaging sensors that BIL serves do not describe their color to the fidelity of an ICC profile. An embedded profile is exposed as a field, with its bytes unaltered, and BIL does not apply it. Color management belongs to a layer above BIL, where a library such as Little CMS can apply the profile.

**Demosaicing.** `cfa.demosaic` works on linear values, after black and white levels are applied. Two methods are built in for 2×2 Bayer patterns: `bilinear`, with a radius of 1 pixel, and `malvar_he_cutler`, with a radius of 2 pixels. `bilinear` is the default. It uses the same bilinear weighting as the resolution levels, so demosaicing and every level come from one pass. A driver can supply further methods, including for other patterns such as X-Trans.

**Sites, angles and fields.** `cfa.sites`, `cfa.binned`, the `polcam` family and `video.fields` bring their samples onto the full grid by bilinear interpolation, with a radius of 1 pixel. At Level 0 their Slices are the same size as every other Slice of the Image (section 5).

**Chroma upsampling.** `video.planes` and `video.rgb` upsample reduced chroma by bilinear interpolation, with a radius of one chroma sample.

- **Placement.** Chroma samples are placed where the stream declares them. Where it declares nothing, they are taken to sit on even luma columns and midway between luma rows, the usual placement for 4:2:0.
- **Full-size chroma.** Where chroma is stored at full size, nothing is upsampled and both functions are pointwise.
- **The stored samples remain.** The chroma as stored is still available, unaltered, as a Fragment of the chroma Image.

### 9.4 Spectral and thermal

| Slice function | Slices it yields | Computes |
| --- | --- | --- |
| `spectral.bands` | One per band, named from metadata, such as `coastal`, `blue`, `green`, `red`, `red_edge`, `near_infrared`; else by wavelength, such as `nm550`; else `band1` onward (*enumerated*) | Stored band values |
| `spectral.radiance` | The same names as `spectral.bands` | Per-band gain and offset |
| `spectral.toa_reflectance` | The same names as `spectral.bands` | Top-of-atmosphere reflectance |
| `spectral.surface_reflectance` | The same names as `spectral.bands` | Reflectance with caller-supplied atmospheric terms |
| `spectral.brightness_temperature` | One per thermal band, by band name (*enumerated*) | Temperature in kelvin |
| `spectral.integrate(from_nm, to_nm)` | `value` | Average over a wavelength range. This is the generic `pixel.integrate` of 9.1 applied to wavelength. |
| `spectral.response(curves)` | One per supplied curve, named by the caller (*enumerated*) | Bands weighted by each response curve |
| `thermal.calibrated` | `counts`, `radiance`, `temperature` | Sensor counts; radiance; temperature in kelvin |

### 9.5 Other rasters

| Slice function | Slices it yields | Computes |
| --- | --- | --- |
| `elev.height` | `height` | Height in meters, scale and offset applied |
| `class.labels` | `value` | The label as a number |
| `class.members` | One per class, named from the class table, such as `water`, `forest`, `urban` (*enumerated*) | 1.0 where the pixel is in that class, else 0.0 |
| `depth.sensor` | `range`, `intensity` | Range in meters; return intensity |
| `depth.corrected` | `intensity` | Intensity corrected for the square of range |
| `depth.points` | `x`, `y`, `z` | The position of each pixel, from its range and the Image's own `camera` step (section 13). Together the three Slices are an organized point cloud. |
| `depth.waveform` | One per range bin, named by its range (*enumerated*) | The return at each range |
| `nuclear.activity` | `activity` | Activity concentration in becquerels per milliliter, from counts with the calibration factor and decay correction the metadata states |

### 9.6 The extras package

Functions that analyze, rather than prepare, are not part of `bil`. The widely useful ones are provided by `bil-extras`, a separate package from the BIL project under the same license.

- **Same names.** Each keeps its place in its family, so `spectral.indices.ndvi` is named the same whether or not it is in the core.
- **Same mechanism.** The package registers its functions through the one registration call of section 10, as any plugin does, and they run in the same single pass as core functions. As a package of the BIL project, it may add to the core families.
- **Same tests.** Window independence and the other conformance tests apply unchanged (section 10.4).
- **Offered when installed.** Without the package, `list_slice_functions` does not offer them.

| Slice function | Slices it yields | Computes | Why it is analysis |
| --- | --- | --- | --- |
| `spectral.indices` | Those the bands allow, such as `ndvi`, `ndwi`, `nbr` (*enumerated*) | Named band indices | Each index serves one application, such as vegetation or water |
| `spectral.angle(reference)` | `value` | Spectral angle to a reference spectrum | It brings in an outside reference |
| `nuclear.uptake` | `suv` | Standardized uptake value normalized by body weight, from `nuclear.activity`, injected dose, decay and patient weight | A clinical measure with several definitions, from fields that are often wrong in practice. Its definition is stated, and it carries the statement that BIL is not for diagnostic use. |


### 9.7 Parameters

Every slice function that takes parameters names them here. They are passed in a `SliceSelector` (section 5).

| Slice function | Parameter | Type | Default | Meaning |
| --- | --- | --- | --- | --- |
| Any function defined for one component | `component` | Component name | The only component, where there is one | The component to use, as in `complex.polar` with `component="hv"` |
| `pixel.weighted` | `weights` | One number for each component | None | The weights of the dot product |
| `pixel.pair` | `i`, `j` | Component names | None | The two components |
| `pixel.nearest` | `coordinate_name`, `value` | Coordinate name; number | The component index's only coordinate; none | The component whose coordinate is nearest the value |
| `pixel.integrate` | `coordinate_name`, `from`, `to` | Coordinate name; numbers | The component index's only coordinate; none | The range to average over |
| `pixel.peak` | `coordinate_name` | Coordinate name | The component index's only coordinate | The coordinate the peak is reported in |
| `pixel.stretch` | `low`, `high` | Numbers | None | The values mapped to 0 and 1 |
| `pol.pair` | `a`, `b` | Channel names | None | The two channels |
| `cfa.demosaic` | `method` | Method name | `bilinear` | `bilinear` or `malvar_he_cutler`, or one a driver adds |
| `spectral.integrate` | `from_nm`, `to_nm` | Numbers | None | The wavelength range, in nanometers |
| `spectral.response` | `curves` | A name and an array of wavelength and weight for each curve | None | The response curves; each yields a Slice of its name |
| `spectral.surface_reflectance` | `gain`, `offset` | One number for each band, or a mapping from band name to number | None | The caller's atmospheric terms: reflectance = `gain` × top-of-atmosphere reflectance + `offset` |
| `spectral.angle`, in the extras package | `reference` | An array of one value for each band | None | The reference spectrum |

- **A parameter with no default is required.** A read that leaves it out is an error.
- **Parameters are checked before any pixel is read.** A wrong name or type is an error from `plan` as from `read`.

## 10. Driver plugin architecture

A driver teaches BIL one format family by describing the Images in a Source, producing Fragments and exposing metadata. The core does everything else.

**A driver does not implement slice functions.** The core computes every Slice from the driver's Fragments, so a new driver gains all applicable slice functions at once.

**What a driver provides**

| Duty | Call | Notes |
| --- | --- | --- |
| Recognize a format | `probe` | Returns a confidence, first for the container and then for its content |
| Open | `open_source` | One file, a set of files or a stream |
| Describe | `list_images` | Size, pixel type, representation, component names, frame count, coordinates, layout and stored levels of each Image, and how the Images are related |
| Read pixels | `read_fragment` | From the full-resolution Image or one of its stored levels. Borrowed or owned, following the zero-copy rule. Marks any pixels that were never recorded. |
| Hold decoder state | `open_reader` | One Reader for each thread that reads. The Image itself holds nothing that changes (section 4). |
| Declare concurrency | `concurrency` | `Parallel`, `OneHandlePerThread` or `Serial`, as below |
| Describe storage | `describe_storage` | For each Image and stored level, a `Storage`: the block size, the interleave, and the blocks with their byte ranges in file order, where that order is known. The core uses it to read ahead and to merge requests (section 4). |
| Expose metadata | `list_field_groups`, `read_field_group` | Named fields, one group at a time and only when asked (section 14) |
| Supply normalization parameters | `read_normalization` | The format's parameters, mapped to the functions of section 7. They become fields, filled in the order of section 14.2. |
| Expose geocoding | `list_geo_models` | The format's geo models, as chains of the steps in section 13 |
| Optional: direct float output | `read_slice_direct` | For decoders that can emit float32 rows themselves. Allowed only for a slice function whose result it reproduces exactly; the Codec agreement test checks it against the core's own path. |
| Optional: extra slice functions | `register_slice_function` | Under the driver's own family, with its Slice names specified or enumerated. A short form of `register_plugin`. |
| Optional: pixel layout | `describe_pixels` | Describes where the pixels are so the compiled core reads them itself; the route scripted drivers take |
| Optional: additional data | `read_auxiliary` | Named extra records, such as tables and arrays, that are not pixels or fields |

**How a plugin is packaged**

| Kind | Built as | Purpose |
| --- | --- | --- |
| Built-in | Rust, compiled into `bil`, each behind a build feature | Wrapping the chosen open source libraries |
| External | Shared library that calls `bil_register_driver` through the C interface | Special drivers in C, C++ or Rust, added without rebuilding BIL |
| Optional | A separate package that BIL finds at run time, through its configured locations (section 10.3) | Plugins under a license stricter than the LGPL, such as Grok, and `bil-extras`, the analysis functions of section 9.6 |
| Scripted | Python code registered at run time and run in CPython (section 10.1) | Esoteric and evolving formats, and additional data the compiled drivers do not read |
| External decoding | A separate decoder process that fills BIL's mapped format (section 10.2) | Any supported format, when configured |

External drivers use the C interface because Rust has no stable binary interface between separately compiled libraries.

**Concurrency.** Each driver and codec declares how far its library allows parallel reads.

| `concurrency` | Meaning | Example |
| --- | --- | --- |
| `Parallel` | Readers on several threads read at the same time, with no lock | BIL's own Rust drivers |
| `OneHandlePerThread` | Each Reader opens its own library handle, and Readers then read in parallel | GDAL, whose dataset handles cannot be shared between threads |
| `Serial` | The library allows one call at a time in the whole process. BIL holds one lock for that library around each call. | The HDF5 C library, built for threads |

- **Conversion stays parallel.** Under `Serial`, only the library call is locked. Conversion to float32, normalization and slice functions still run on several threads (section 12).
- **A wrapper's lock is not a fault when the library needs it.** A lock is counted against a wrapper only when the library itself does not require one (section 10, the tests for wrappers).
- **TIFF reads in parallel.** BIL's TIFF driver reads the directories once, when the Source opens; each read then reads and decompresses its own chunks, so the driver is `Parallel`.

**What a registration gives.** `register_plugin` takes a description whose contents follow the plugin's kind. For a slice function and a level method, the two that add arithmetic, the description is fixed here.

| A slice function gives | Meaning |
| --- | --- |
| `name` | Two parts, in a family the plugin may use (section 9) |
| `representations` | The representations it accepts |
| `slice_names` | Specified, as a list, or enumerated, as a rule over the `ImageInfo` |
| `kinds` | For each Slice, how it is reduced at coarser Levels: linear, angle, label or range (section 5) |
| `radius` | 0 for a pointwise function, or the neighborhood a local one reads |
| `parameters` | Each parameter's name, type and default, as in section 9.7 |
| `inputs` | The fields it needs (section 14.2) |
| `kernel` | The code, as below |

| A level method gives | Meaning |
| --- | --- |
| `name` | Its snake\_case name |
| `weights` | The values applied along rows and columns at each halving |
| `radius` | How far the weights reach |
| `alignment` | Whether reduced values sit on full-resolution pixels or between them |

**How a registered kernel runs.** BIL calls a registered kernel once for each block of rows. It receives each input component already converted to float32, with the margin rows and columns its radius needs, and the values of its parameters, and it writes one row block for each Slice. BIL does everything else: reading, conversion, Levels, delivery and threads. The core's own functions are compiled into the fused loop of section 12; a registered kernel runs as a call for each row block, which is slower but keeps every other guarantee.

**Plugin interface version.** `bil_register_driver` carries the version of the plugin interface the plugin was built against.

- **Same major version.** The plugin is loaded. A plugin built against an older minor version still works, because minor versions only add.
- **Different major version.** The plugin is refused at registration and never called. `list_drivers` shows it as not loaded, with the reason.

**Driver selection.** Probing runs in two stages, because many formats share one container: DNG, GeoTIFF, whole-slide and many raw camera files are TIFF underneath, NetCDF-4 and several SAR products are HDF5, and DICOS is DICOM.

1. **The container.** The file name and leading bytes identify the container, such as TIFF, HDF5 or DICOM. Where a container has one driver, its leading bytes alone decide, and the order of trying matters only to the list of drivers an unrecognized file reports. Confidence and the configured order apply from a container's second driver.
2. **The content.** The drivers for that container examine its contents, such as GeoTIFF keys, DNG tags or NetCDF conventions, and each returns a confidence. The highest wins.
3. **Ties.** Equal confidences are broken by the installation's configured order of preference, never by load order.

A caller can also name the driver explicitly with `open_source_with`.

**JPEG 2000 codecs in order.** emuella-j2k for the profiles it declares, once it has passed BIL's tests; then j2k-native, once it has passed them; then OpenJPEG for everything else.

- **Until then, OpenJPEG.** Until a Rust codec passes, OpenJPEG decodes every JPEG 2000 file.
- **Chosen once.** The codec is chosen when a Source is opened, so a file is never decoded partly by one codec and partly by another.
- **OpenJPEG's risk is bounded.** Its repository describes it as unmaintained. It follows section 10.5, so where a package manager supplies it, BIL uses that patched shared copy. The Codec agreement test checks it against the other codecs. A deployment worried about hostile files can set `isolation` to `Untrusted` (section 10.3).
- **hayro-jpeg2000 is a test reference only.** It forbids unsafe code and has been tested on a large corpus of real files, which makes it a good independent check. It is not a codec for BIL, because its output is 8-bit and it has no region or reduced-resolution decoding.

**Containers and codecs.** A driver reads a container format. Compressed pixel blocks inside it are handed to a codec that BIL chooses separately. One JPEG 2000 codec therefore serves NITF, JP2, GeoTIFF and HEIF alike, and BIL does not depend on the JPEG 2000 support bundled inside GDAL.

- **Codecs by what they decode.** A driver names the codec its blocks need by what is coded, in snake\_case, not by library. NITF's compression codes map as follows: `jpeg` (C3, M3), `jpeg2000` (C8, M8), `bilevel` (C1, M1), `vector_quantization` (C4, M4), `lossless_jpeg` (C5, M5), `downsampled_jpeg` (I1), and `nitf_` with the code, in lower case, for any other, such as `nitf_c7`.
- **TIFF's compression codes** map likewise: `deflate` (8, 32946), `lzw` (5), `jpeg` (6, 7), `packbits` (32773), `jpeg2000` (34712), `lerc` (34887), `zstd` (50000), `webp` (50001), `fax` (2, 3, 4), and `tiff_` with the code in decimal for any other, such as `tiff_34676`. Uncompressed data needs no codec.
- **A NITF JPEG 2000 Image is one codestream,** or one JP2 file, whose tiles the NITF blocks describe; a header that disagrees with the subheader is corrupt. A mask table placing more than one block, and a JP2 file with a palette, component mapping or channel definitions, are refused as unsupported until a real file shows how they are used.
- **When no codec accepts an Image.** The Source still opens and the Image is listed. Every read of it fails: as corrupt where its coded data is damaged or disagrees with its container, and as unsupported where no codec in the build decodes it. Only damage to the structure that locates the Images fails the open (section 3.3).

**Several libraries per format.** More than one driver or codec may cover a format. BIL chooses among them in a set order of preference, and the first that accepts the Source is used.

- **Chosen once, when the Source is opened.** A codec accepts a Source only after checking the coding parameters the file declares, such as a JPEG 2000 profile. The driver and codec chosen are kept for the life of the Source.
- **Never switched part-way.** A block the chosen codec cannot decode is corrupt data (section 3.3). BIL does not move to another codec for the rest of the Image, because lossy decoders round differently and the result would be a mixture.
- **Reported.** `describe_decoding()` on a Source gives the driver and codec chosen, with their versions.
- **Recorded.** Everything BIL writes records the driver and codec, with their versions, that read its input (section 11).

**Plugin kinds.** Bringing every candidate library into the design needs more than one kind of plugin. All kinds register through one call, `register_plugin`, which takes a description of the plugin and its kind, and all are chosen by the same order of preference. `bil_register_driver`, `register_slice_function`, `register_level_method` and the scripted `register_driver` are short forms of it.

| Plugin kind | What it does | Planned plugins |
| --- | --- | --- |
| Container driver | Opens a Source, lists its Images and produces Fragments | GDAL, OxiGeo, nitf-rs, async-tiff, libtiff, LibRaw, RawSpeed, rawler, HDF5, NetCDF, zarrs, CFITSIO, DCMTK, ITK, the native ENVI driver. Under LGPL 3, and so test references or optional plugins only (section 10.6): NITRO, SIX, libheif and hidefix. |
| Codec | Decodes compressed pixel blocks handed over by a container driver | emuella-j2k, j2k-native, OpenJPH, OpenJPEG, Grok (optional), libjpeg-turbo, jxl-oxide, CharLS, LERC, rav1d, dav1d |
| Stream source | Delivers frames from video files, network streams and live cameras | FFmpeg, GStreamer, ts-transformer, Aravis, cameleon |
| Metadata parser | Turns embedded metadata into named fields (section 14) | libmisbklv, klv-uas, ts-transformer, Exiv2 |
| Geocoding step | Supplies a step for the geocoding chains of section 13 | usgscsm and the Community Sensor Model interface as `external` steps; PROJ for `reference_change`; OSSIM |
| Storage backend | Reads bytes from files, memory and object stores | object\_store, OxiGeo cloud storage |
| Time-zone lookup | Finds the time zone at a position, for local times (section 14.2) | Optional only: a plugin over tzf-rs, whose boundary map is under the Open Database License |
| Level method | Adds a named downsampling method, registered with `register_level_method` | None yet; `bilinear`, `binomial5` and `decimate` are built in |
| Scripted driver | Python code that interprets a format's headers and additional data, leaving pixels to the compiled core | Written as needed; they may call SARkit or SarPy |

**Licensing and optional plugins.** BIL is licensed under the LGPL, version 2.1 or later (section 10.6). A plugin under a stricter license is never part of the `bil` package: it ships as a separate optional package that BIL finds at run time. Grok is the case in point. It is AGPL, so it is offered only as an optional plugin, for example `bil-grok`, and a user who installs it has made their own installation AGPL. `list_drivers` reports every loaded plugin with its license, so an installation can be audited.

**Included libraries**

| Format family | Library | Role |
| --- | --- | --- |
| JPEG 2000 codec | [OpenJPEG](https://github.com/uclouvain/openjpeg) | The fallback, and for now the codec for every file: the only open decoder that covers all of Part 1, with region and reduced-resolution decoding |
| JPEG 2000 codec, on trial | [j2k-native](https://docs.rs/crate/j2k-native/latest) | The Rust path. Becomes the fallback, ahead of OpenJPEG, once it passes BIL's conformance and Codec agreement tests on NITF reference files. |
| JPEG 2000 codec, preferred once proven | [emuella-j2k](https://github.com/emuella/emuella-j2k) | Written in Rust, so it joins the core directly. Preferred for the profiles it declares, once it passes the same tests. Not yet released. |
| GeoTIFF | The Rust [`tiff`](https://github.com/image-rs/image-tiff) crate for the directories, with BIL's own reading of chunks and GeoTIFF keys; Deflate through miniz\_oxide, LZW through weezl, PackBits BIL's own | Tier 1's GeoTIFF reader, with no C library. Samples are always as stored (section 4). A mode of the crate that returns stored samples, offered to its maintainers, is a future option; until then BIL reads the chunks. GDAL is the reference it is tested against. |
| General rasters | GDAL | Broad raster coverage beyond Tier 1. Its JPEG 2000 support is not used. |
| NITF | A native Rust NITF reader | Tier 1's NITF container reader. Headers and extensions are read through the field definitions of section 14.1. NITRO, which is LGPL 3.0, and GDAL are the references it is tested against. |
| SICD, SIDD | [SIX](https://github.com/ngageoint/six-library) | SICD, SIDD and CPHD metadata and pixels, in Tier 2. SIX is LGPL 3, so it is assessed under section 10.6 when Tier 2 starts. |
| Raw camera files | LibRaw | Raw camera decoding |
| Video | FFmpeg, version 6 or later | Demuxing and decoding compressed video. Always the shared copy from conda or the system package manager, reached through the rusty\_ffmpeg bindings and a thin safe layer of BIL's own (section 10.5). |
| HDF5, NetCDF | The HDF5 and NetCDF C libraries | Scientific and hyperspectral containers |
| ENVI | A native Rust driver | The format is a text header beside flat binary |

Every library in Appendix A is also part of the plugin design for now. Each will be assessed case by case before it is built.

**Existing wrappers first.** For each C or C++ library, BIL first considers an existing Rust wrapper crate, so that it is not responsible for building the library or tracking its interface. A wrapper is used when it passes all four tests below. Where none does, BIL depends on an existing bindings crate where one passes, and writes its own bindings in `bil-sys` only where none does.

| Test | A wrapper fails it when |
| --- | --- |
| Maintenance | It lags the library's releases, has no active maintainer, or is hard to build on a supported platform |
| Performance | It adds copies, locks or conversions on the pixel path beyond what the library itself requires. hdf5-metno's lock around every read is not counted, because the HDF5 library allows one call at a time anyway |
| Capability | It hides calls BIL needs, such as region decoding, access to raw decoded buffers or reading from memory |
| Packaging | It cannot supply the library from the source that section 10.5 sets for it |

**FFmpeg: bindings and a thin layer of BIL's own.** FFmpeg is the first case decided this way. ffmpeg-next is in maintenance mode, so it is weak on Maintenance. BIL's deferred Fragments and its direct path from decoded planes need raw access to packets and planes either way, so a full safe layer over FFmpeg would add little that BIL does not have to bypass. BIL therefore depends on rusty\_ffmpeg, the bindings crate under rsmpeg, and writes its own small safe layer over it.

- **Why a thin layer.** BIL uses a narrow part of FFmpeg: opening, reading packets without decoding them, seeking to key frames, decoding, and receiving frames, together with the packets of KLV data streams. Deferred Fragments and the direct path from decoded planes into the fused loop (section 12) need raw access in any case.
- **BIL does not generate bindings.** rusty\_ffmpeg provides them, and it is regenerated for each FFmpeg release.
- **Its unsafe code is BIL's.** The layer is BIL's own Rust code that reads outside data, so the fuzz testing of section 10.3 covers it.
- **FFmpeg 6 or later.** rusty\_ffmpeg covers FFmpeg 6 to 8. On a system whose package manager ships an older FFmpeg, the user installs a newer one through conda.

**OpenJPEG: BIL's own bindings, and openjpeg-sys as a supplier.** jpeg2k, the wrapper, fails Capability: it reads the header and decodes in one call, so nothing can be checked between them, and it sends OpenJPEG's warnings to a logger. openjpeg-sys passes Capability and Performance but fails Packaging, since it always builds its own copy. BIL therefore declares OpenJPEG's interface itself, in `bil-sys`, and links the shared copy from conda or the system wherever one is found. Where none is found, as in a Windows build or a PyPI wheel, `bil-sys` builds a private copy through openjpeg-sys, used only to supply the library, with BIL's declarations kept. In the shared library BIL delivers, that copy is linked statically and its symbols are not exported (section 10.5). A Rust program that compiles the `bil` crate in holds the copy's symbols as its own, which is why the shared copy comes first. openjp2, a port of OpenJPEG to Rust, is assessed beside the other Rust JPEG 2000 codecs, by the same measures.

**Illustrative shape** (not final)

```rust
pub trait Driver {
    fn name(&self) -> &str;
    fn probe(&self, location: &Location) -> Confidence;
    fn open_source(&self, location: &Location) -> Result<Box<dyn SourceReader>>;
}

pub trait SourceReader {
    fn list_images(&self) -> Vec<ImageInfo>;
    fn open_reader(&self, image_index: u32) -> Result<Box<dyn Reader>>;   // one for each thread; holds decoder state
    fn describe_storage(&self, image_index: u32, stored_level: u32) -> Storage;  // one for each level, its blocks in file order where known
    fn list_field_groups(&self, image_index: u32) -> Vec<String>;
    fn read_field_group(&self, image_index: u32, group_name: &str) -> Result<FieldGroup>;
}

pub trait Reader {
    fn read_fragment(&mut self, stored_level: u32, window: Window) -> Result<Fragment<'_>>;  // stored_level 0 = full resolution
}
```

### 10.1 Scripted drivers

A scripted driver is written in Python. The script reads the small, format-specific parts of a file, and the compiled core still reads the pixels.

Scripted drivers exist for esoteric and evolving formats, and for special additional data that the compiled drivers do not read. A script can be changed as a format changes, without rebuilding BIL.

**How the work is split**

| Part of the file | Handled by | Cost |
| --- | --- | --- |
| Headers, tags and format quirks | The Python script | Low: usually a few kilobytes, read once when the Source is opened |
| Special additional data, such as calibration tables or per-line records | The Python script | Low: small beside the pixels |
| Bulk pixel data | The compiled core | The same fast path as any other driver |

**Pixels by description.** A scripted driver does not loop over pixels. It returns a `PixelLayout` that says where the pixels are and how they are stored, and the compiled core reads them from that description. The zero-copy rule, Slices and resolution levels all apply unchanged.

| `PixelLayout` field | Describes |
| --- | --- |
| `offset` | Where the pixel data starts |
| `byte_order` | Big or little endian |
| `interleave` | An `Interleave`: `Pixel`, `Line` or `Band` |
| `row_stride` | Bytes from one row to the next, for rows that carry padding or their own headers |
| `blocks` | For tiled or striped data, the position and byte offset of each block |
| `codec` | The codec for compressed blocks, by name |
| `validity` | Which pixels were never recorded, as regions or a mask |

Where a layout cannot be described this way, the script may return the pixels itself as an array. That path is interpreted and slower, and `plan_slice` reports it.

**Additional data.** A script exposes extra records by name, and callers read them with `list_auxiliary` and `read_auxiliary`. A script can also be attached to a compiled driver, to add data that driver does not read, such as a new or vendor-specific metadata block. A script supplies named fields and field definitions in the same way (section 14).

```python
class VendorDriver(bil.ScriptedDriver):
    name = "vendor"

    def probe(self, location):
        is_vendor = location.read_bytes(0, 4) == b"VNDR"
        return bil.Confidence.High if is_vendor else bil.Confidence.NoMatch

    def list_images(self, source):
        self.header = parse_header(source.read_bytes(0, 512))   # interpreted: 512 bytes
        pixel_type = bil.PixelType(bil.SampleType.U16, 1, 12)
        return [bil.ImageInfo(self.header.row_count, self.header.col_count, pixel_type, ["intensity"])]

    def describe_pixels(self, source, image_index):
        return bil.PixelLayout(offset=512, byte_order="big", interleave=bil.Interleave.Pixel)   # compiled from here

    def read_auxiliary(self, source, name):
        return parse_calibration_table(source.read_bytes(self.header.table_offset, self.header.table_size))

bil.register_driver(VendorDriver)
```

**The interpreter.** Scripted drivers run in CPython, so they work for Rust, C and C++ callers as well as from Python.

- **CPython.** The interpreter is CPython, the standard Python, because scripted drivers call existing Python libraries, and many of those, such as NumPy, are built for it alone. Smaller interpreters run only a subset of Python and none of its compiled libraries.
- **Taken from the machine, not shipped.** BIL uses the Python installed on the machine, as it uses any library a package manager supplies (section 10.5). BIL's packages contain no interpreter. Where no Python is installed, a scripted driver fails with an error that says so, and everything else works.
- **Started on first use.** The interpreter starts only when a scripted driver is first needed. Callers that never use one pay nothing.
- **One interpreter.** When BIL is itself running inside Python, it uses that interpreter and does not start a second.
- **A helper process for other callers.** For Rust, C and C++ callers, CPython runs in a separate BIL helper process, started on first use. A failing script, or a clash between the caller's libraries and Python's, stays out of the caller's process, and whichever Python 3 is installed can be used.
- **The round trip is watched.** Each call into a scripted driver from a helper process is a round trip between processes. It is made when a Source is opened and when additional data is read, never for pixels a `PixelLayout` describes. A script that returns pixels as an array pays it for every read, as `plan_slice` reports. Its cost is benchmarked (section 12), and the choice is reviewed if it becomes prohibitive.
- **Only from configured locations.** The helper program is found only where the configuration says, as plugins are (section 10.3).
- **Kept off the pixel path.** The interpreter runs when a Source is opened and when additional data is read. Pixel reads from a `PixelLayout` never enter it, so they run in parallel as for any other driver.

**Python libraries.** A scripted driver may call existing Python libraries, such as SARkit, to interpret metadata.

**Trust.** A script is code. BIL loads scripted drivers only when the caller registers them or names the folder to load them from.

### 10.2 External decoding and the mapped format

BIL can hand the decoding of any supported format to a separate process and read the result through a memory-mapped format of its own.

This is an optional layer, built and supported as part of BIL. It is switched on by configuration for an existing supported format, and it is off by default. The caller's code does not change: the same `open_source`, `read_fragment` and `read_slice` calls are used.

1. **A decoder process** opens the original Source outside the caller's process.
2. **The mapped format** receives the decoded data. It is laid out so that it can be mapped into memory and used as it stands.
3. **The mapped driver** in the caller's process maps that data and serves Fragments and Slices from it directly.

**Two ways to use it.** The configuration chooses, for each format or Source.

| Way | What the decoder process does | Cost |
| --- | --- | --- |
| On request | Decodes only the blocks each read needs, into shared memory that is reused as a cache of bounded size. For a live stream, it keeps a ring of recent frames. | Follows the request, as rule 1 requires |
| Whole copy | Decodes the whole Source once, into mapped data that several reading processes can share and that can be kept | Follows the Source, and is chosen knowingly |

- **Isolation always uses on request.** Isolating a driver must not change what a read costs, beyond the round trip between processes (section 10.3).
- **A whole copy is a deliberate choice.** It is useful when many processes read one Source, or when the decoded data is to be kept.

| Part | What it is |
| --- | --- |
| Mapped format | BIL's own format for decoded data: uncompressed, in native byte order and aligned, with a header describing each Image. It can hold native pixels, float32 Slices, or both. |
| Decoder process | A separate BIL program that decodes a Source with any driver and fills the mapped format, on request or as a whole copy |
| Mapped driver | A built-in driver that reads the mapped format. Every Fragment it returns is `Borrowed`, and so is every Slice the format already holds. |
| Configuration | Names the formats or Sources that are decoded externally. Nothing else in BIL has to know. |

**Where the decoded data lives.** Mapped data is a clear, decoded copy of the Source, so its handling is fixed.

| Rule | Detail |
| --- | --- |
| The caller chooses the place | The configuration names a folder, or shared memory with no file. There is no default, so nothing is decoded externally until one is chosen. |
| Private to its owner | Mapped data is created so that only its owner can read or write it. |
| Marked | It carries the Source's markings, and its identity fields are empty unless the caller keeps them (section 11). |
| Removed after use | It is removed when the decoder process ends, unless the configuration sets `keep_mapped`. |
| Leftovers are cleared | A decoder that stops abnormally can leave data behind. When external decoding next starts in that place, BIL removes mapped data that no running decoder owns and that was not set to be kept. |
| Not encrypted | BIL does not encrypt mapped data. Where that is needed, the caller names a place on encrypted storage or uses shared memory. |

**Why decode in another process.** A slow or fragile decoder cannot stall or crash the caller. It is also how isolation works (section 10.3). One decoding can serve several reading processes at once. Decoders that cannot be loaded into the caller's process can still be used. Bio-Formats is an example: it reads most microscope formats but is written in Java.

**With video.** The decoder process does the decoding, so a Fragment from the mapped driver is not deferred: it is already decoded and simply mapped.

**Writing.** The mapped format is written by BIL's own writer (section 11).

**The layout.** The mapped format is BIL's own, made for mapping and nothing else.

| Part | Content |
| --- | --- |
| Header | JSON text, so a person can inspect it: each Image with its grid, sample type and Levels, its fields and markings, and the block table's place. A whole copy also carries a `complete` flag. |
| Block table | For each block: which Image, Level, frame and tile it holds, where it is, and its state |
| Blocks | Native pixels or float32 Slices, uncompressed, in native byte order, each starting on a memory page so that it can be borrowed as it stands |

- **On request, the blocks are a fixed set of slots.** They form a cache of bounded size. For a live stream they form a ring of recent frames.
- **A whole copy fills every block once.**
- **Other tools do not read it.** A caller who wants a decoded copy for other software writes one with BIL's writers (section 11).

**Starting the decoder process**

- **Spawned on first use.** BIL starts the decoder process when external decoding is first needed, and it ends when its parent ends.
- **Or attached to.** Where a decoder is already running in the configured folder, BIL attaches to it instead, so one decoding can serve several processes. A lock file there names each decoder's owner and its control channel, which is also how leftovers are cleared.
- **A folder is needed to attach or to keep.** Attaching finds a decoder through its lock file, and a kept whole copy lives in a file, so both need the configuration to name a folder. With shared memory and no folder, each caller spawns its own decoder.
- **Only from configured locations.** The decoder program is found only where the configuration says, as plugins are (section 10.3).

**Learning that data is ready**

- **On request, by the control channel.** A reader asks for blocks over a control channel to the decoder, and the decoder replies when they are ready. The same channel carries every request, on every system.
- **For a whole copy, by the `complete` flag.** The decoder sets it in the header last, after every block is safely written.

**Blocks a caller still holds.** On request, a slot is reused once its block is no longer needed, but a caller may still hold a borrowed Fragment from it.

- **A reader marks each slot it is using,** and the decoder reuses only slots that no reader has marked.
- **A dead reader's marks are cleared.** When a reader's control channel closes, the decoder clears that reader's marks.

**Private by construction.** Shared memory is created with no name and passed over the control channel to each reader: as a file descriptor on Linux and macOS, and as a duplicated handle on Windows. No other process can find it, so it is private to its owner and the readers it serves without relying on file permissions. Mapped data in a folder is protected by file permissions, as the table above says.

### 10.3 Untrusted files

Imagery files often arrive from outside, and BIL hands them to many C and C++ libraries. A file crafted to exploit one of those libraries could take over the caller's process, and a Rust core does not protect code it only calls. BIL answers with four measures.

**Resource limits.** `open_source` takes `OpenOptions`, in which the caller sets limits. A read whose header claims more than a size limit allows fails before any memory is taken for its pixels. The scan limit and the cap on a library's working memory are met while decoding, so they stop the decode, which frees what it took.

| Limit | Guards against |
| --- | --- |
| `max_image_pixels` | A header that claims an impossible size |
| `max_decoded_bytes` | One compressed block that expands without bound |
| `max_field_group_bytes` | A metadata group built to exhaust memory |
| `max_scans` | A progressive image with thousands of scans, each a pass over the whole image |
| `max_directories`, `max_subdirectories` | A chain of directories built to be walked without end. These two have defaults, since walking a directory costs little and real files hold few. |

- **No limit is set by default for a trusted Source.** Very large images are BIL's purpose, so any fixed default would turn away real data. `max_directories` and `max_subdirectories` are the exception: they count structure, not data.
- **An untrusted Source gets limits.** Opening a Source as untrusted applies default limits: decoded bytes per block, pixels per Image, bytes per metadata group and 500 progressive scans, each scaled to the size of the file where that makes sense. The caller can raise them.
- **One check is always made.** For uncompressed data, a header that claims more pixels than the file can hold is rejected.
- **A library's working memory is capped too.** Where `max_decoded_bytes` is set, the memory a library uses beside its output for one block, such as a progressive JPEG's whole-image coefficients, is capped at twice that limit, where the library allows a cap.
- **Where a library offers no cap,** and `max_decoded_bytes` is set, the driver refuses, before reading it, a block whose declared size over every component exceeds twice that limit, since a library's working memory can follow the size of the block it reads, as GDAL's does. The refusal is an error.
- **A refusal is an error.** A read that a limit refuses fails with an error, as section 3.3 says. It is reported as unsupported, not as corrupt, since the file may be sound. A library's own limits, such as the `tiff` crate's on the size of a tag's values, are set from `max_decoded_bytes` where they bound decoded data.
- **What a codec would expand, the driver applies.** A codec that would add components after the header is checked, as a JPEG 2000 palette does, is told not to. The driver delivers the palette's indices, which `palette.lookup` turns into color, and applies the component mapping and channel definitions (`cmap` and `cdef`, which say, for one, which component is alpha) itself, or reports them where it does not.

**Plugins from configured locations only.** BIL loads external, optional and scripted plugins only from locations named by the caller or by the installation's configuration.

**The configuration.** The installation's configuration is one file in a stated place, which an environment variable can point elsewhere, and a call reads it. It names plugin and helper locations, the order of preference among drivers, `isolation` and the place for mapped data. Its form, its place, the variable's name and the call are settled when Tier 2 starts.

- **No searching.** BIL does not look in the working directory, on the library search path or beside the data.
- **Installed packages are still found.** Installing an optional package such as `bil-grok` records its location in that configuration.

**Isolation.** C and C++ drivers and codecs can run outside the caller's process, in the decoder process of section 10.2. The `isolation` setting decides when.

| `isolation` | Effect |
| --- | --- |
| `Off` | Every driver runs in the caller's process. This is the default. |
| `Untrusted` | Isolated for any Source the caller opens as untrusted, through `OpenOptions` |
| `Always` | Isolated for every Source |

- **Why it is off by default.** An isolated read cannot borrow from the original file, passes each decoded block through shared memory, and pays a round trip between processes for each read. It decodes only what each read needs (section 10.2).
- **A policy for the deployment.** A system that handles outside files can set `Untrusted` or `Always` once, in its configuration, without changing the calling code.
- **Reduced privileges.** Where the platform allows, the decoder process runs with no network and with access only to the Source and its mapped data. BIL reports whether privileges were reduced.
- **It needs a place for mapped data.** If isolation is called for and no place has been chosen (section 10.2), the open fails. BIL does not fall back to running the driver in the caller's process.
- **BIL's own Rust drivers stay in process.** They are covered by the fuzz testing below.
- **The limit.** Isolation protects the caller's process. A decoder that has been taken over can still return wrong pixels.

**Remote Sources.** A Source read over a network is read through the same calls as a local one, with three differences in how BIL works.

- **A private runtime.** Storage backends such as object\_store are asynchronous. BIL runs them on a small runtime of its own, hidden from the caller, and keeps its blocking interface. The caller's own asynchronous runtime is never touched.
- **Requests are counted and merged.** Over a network, the number of requests costs more than the bytes. `plan` reports both for a read, and BIL merges byte ranges that lie within a stated gap of each other into one request.
- **Timeouts and retries.** `remote_timeout` and `remote_retries` in `OpenOptions` bound each request. A read whose requests all fail is an error.

**Fuzz testing.** BIL's own Rust code that reads outside data is fuzz tested as a condition of release: its drivers and parsers, the mapped driver and the reader of companion files. This does not cover the C and C++ libraries, which rely on their own projects' testing and on isolation.

### 10.4 Driver conformance

A driver is finished when it passes one suite of tests, which is the same for every driver.

| Test | A driver passes when |
| --- | --- |
| Faithful Fragment | Fragment values equal the stored values of reference files, with the stated pixel type and significant bits |
| Window independence | A Slice of any Window equals the same region cut from a Slice of the whole Image, to the tolerance of section 5, for every slice function and Level the driver offers, on every instruction set BIL ships |
| One grid | At a given Level, every Slice of an Image has the same size, origin and scale |
| Plan against actual | The size, access class, stored level and blocks decoded that `plan_slice` reports match what the read then does |
| Zero-copy | A Fragment is `Borrowed` whenever the zero-copy rule is met |
| Bounded memory | A small Window read from a very large file stays within a fixed memory budget |
| Missing data | Pixels that were never recorded, and fill values, are NaN in every Slice under the default options, and take the caller's `missing_value` when one is set |
| Metadata on demand | Opening extracts only what describes the Images, and each group is extracted on first request |
| Markings | Marking fields reach everything written, and identity fields are written empty |
| Containment | The driver makes no network request and writes no file that the caller did not ask for |
| Round trip | For a writable format, a file BIL writes reads back with identical pixel values |
| Codec agreement | For losslessly coded data, every codec that accepts a file gives identical values. For lossy data, no sample differs from the reference decoder's by more than one unit of its stored value. |
| Uniformity | Code written against another driver reads this driver's Sources without change |
| Geocoding | Each step the driver supplies passes the validation tests of section 13.8 |
| Declared concurrency | Reads on many threads give the same values as the same reads on one, under the driver's declared `concurrency` |
| Access patterns | Every read gives the same values and statuses under `Random` and `Sequential`, and the storage order reported matches the file |
| Deferred decoding | A deferred Fragment's native pixels equal those of the same Fragment decoded at once |
| Status | Files built with known gaps and damage report `Partial` and `Missing` where they should, and `Complete` elsewhere |
| Field names | Names with escaped dots and numbered repeats are listed and read back correctly |

- **Reference files.** Each driver has a set of small files with known contents, kept with the tests.
- **A failed test blocks release.** A driver that fails any test is not shipped as part of its tier.

### 10.5 Packaging and coexistence

Installing BIL is enough to read its formats. BIL does not duplicate libraries that standard package management already supplies, and its libraries do not disturb other copies in the caller's process.

**Where each library comes from.** BIL takes each backing library from the first of these sources that works without loss.

| Order | Source | What BIL does |
| --- | --- | --- |
| 1 | A Rust crate that already provides the library: a Rust reader, or a wrapper crate whose own build supplies the C or C++ library | Depends on it in the ordinary way, and builds nothing itself |
| 2 | The package manager that installs BIL, where it supplies the library as a shared dependency and resolves it automatically, as conda and Linux distributions do | Uses that one shared copy |
| 3 | A build of BIL's own | Carries a private copy. This is the last resort. |

**What "without loss" means.** A source is acceptable only when all of these hold.

- **One step.** The user installs BIL once and can read the formats, with no separate installs and no paths to set.
- **No clash.** The library cannot collide with another copy in the same process (below).
- **The wrapper tests pass.** Maintenance, performance, capability and packaging, as set out in section 10.

**FFmpeg comes from a package manager only.** For FFmpeg alone, BIL never carries a private copy, and source 3 does not apply.

- **One step, through conda or a system package manager.** For FFmpeg, the one-step test is met when conda or the system's package manager supplies it as a shared dependency. Installing BIL from PyPI reads video only where FFmpeg is already installed on the system.
- **Nothing to clash with.** BIL and other packages, such as PyAV and OpenCV, load the same shared copy.
- **Its license is the installation's.** BIL's packages never contain FFmpeg, so whether the installed copy is built with GPL parts is decided by its packager and is the user's concern (section 10.6). `list_drivers` reports the license it was built under.
- **Only FFmpeg.** Every other backing library still follows the three sources above, and may be carried as a private copy.

**Avoiding clashes.** A process that uses BIL will often also hold GDAL, HDF5 or FFmpeg, loaded by another package such as rasterio or h5py. Two versions of one C library that can see each other may crash or corrupt data. There are two safe arrangements.

- **One shared copy.** Where the package manager gives every package the same copy, there is nothing to clash with. This is source 2.
- **A private copy.** Where the library is built into BIL, by a crate BIL depends on or by BIL itself, its symbols are hidden, and it uses its own data files, such as the PROJ database.

**Rules that hold whichever source is used**

| Rule | What it prevents |
| --- | --- |
| BIL configures its libraries through their own calls, never through environment variables | Settings meant for BIL changing another package, or the caller's settings changing BIL |
| BIL installs no signal handlers and does not change the locale, the working directory or shared error handlers | Side effects on the rest of the process |
| BIL does not let its libraries load plugins of their own from search paths | Stray plugins, and a way around section 10.3 |
| BIL registers the drivers of a library it shares with the caller one at a time, only those it uses, and opens each Source with only the driver it chose allowed. It never registers all of a library's drivers at once where that would load plugins from search paths, as GDAL's `GDALAllRegister` does. For GDAL: one `GDALRegister_` call for each driver, and `GDALOpenEx` with a list of allowed drivers. | Plugins loaded through a shared library, and other packages' drivers probing BIL's Sources. Registering only adds, so another package that registers everything later still works. |

**Where a library reads the environment anyway.** OpenJPEG reads `OPJ_NUM_THREADS` when a decoder is made, before any call can stop it, and where the variable is set it starts that many threads, which BIL's call then stops. BIL keeps OpenJPEG on the calling thread, spreads a read over threads by Windows or tiles on its `Pool` (section 12), and accepts this residue as the most OpenJPEG's interface allows. It does not change the process's environment, which would affect the caller's other code. If one large tile, read alone, proves too slow, this is revisited.

**A coexistence test.** Each release is tested in one process alongside the common packages that carry the same libraries, reading through BIL and through each of them in turn.

**Where neither arrangement is safe.** A library that can be neither shared nor kept private runs in the decoder process, as an isolated driver does (section 10.3).

**What this costs.** BIL builds and carries only the libraries it must, so its own package stays as small as the sources allow. The result differs by package manager: an installation from conda shares its libraries, and one from PyPI carries private copies.

**Still separate.** Plugins under a stricter license, such as `bil-grok`, remain separate packages (section 10).

### 10.6 License and responsibility

BIL is licensed under the LGPL, version 2.1 or later, with a static-linking exception. The aim is to leave users free to make their own decisions and installations, while BIL itself stays a shared library that anyone can use.

**How BIL is delivered.** BIL is written in Rust. A C-callable interface is laid over the Rust library and covers all of it, and the two are delivered together as one shared library.

- **Any language can call it.** Whatever can call C can use the whole of BIL.
- **It can be replaced.** A user can put a modified build of the shared library in place without rebuilding their own program, which is what the LGPL asks for.
- **Rust callers have a choice.** A Rust program may use the shared library, or compile the `bil` crate into itself.
- **A static-linking exception.** BIL's license adds an exception to the LGPL: a program may link BIL statically, as Rust programs normally do, without the LGPL's duty to let its users relink it with a modified BIL. Changes to BIL itself remain under the LGPL. Its wording is in `LINKING-EXCEPTION.md`.
- **Python is built on the same Rust library.**

**Who answers for what**

| What | Who answers for its license |
| --- | --- |
| Everything inside BIL's own packages | The BIL project |
| Anything a user installs beside BIL, such as a GPL build of FFmpeg or an AGPL plugin | The user |
| Any combination a user passes on to others | The user |

**What BIL's own packages may contain**

- **Only LGPL-compatible builds.** Nothing under the GPL or the AGPL is ever inside a BIL package.
- **The permissive build of each library.** Where a library BIL carries can be built with or without GPL parts, BIL's packages use the build without them. FFmpeg is never carried (section 10.5).
- **Notices.** Each package carries the license notices of everything in it.
- **Checked at every release.** The contents of each package are checked against these rules before it is published.

**Compatible with version 2.1 itself.** Everything in a BIL package is under a license compatible with LGPL 2.1, so that no package becomes LGPL 3 by accident. Apache-2.0 components, such as emuella-j2k, are accepted by the author's ruling, which accepts the risk that the Free Software Foundation holds Apache-2.0 incompatible with version 2.1. LGPL 3 code is never inside a BIL package. It is used in tests, as NITRO is, or as an optional plugin packaged separately, as plugins under stricter licenses are. SIX is LGPL 3, so how SICD and SIDD are read is assessed under this rule when Tier 2 starts (section 16).

**Checked automatically.** In the build, cargo-deny checks the license of every dependency of what ships against one list, and of test-only code against a wider one. Every source file carries an SPDX license line.

**What `list_drivers` reports.** It gives the user what they need to decide for themselves.

| For each library and plugin | Reported |
| --- | --- |
| Name and version | As the library states them |
| Source | BIL's package, an existing crate, the package manager, or a plugin the user installed |
| License as built | Recorded at build time for anything BIL built; otherwise what the library declares; otherwise `Unknown` |

## 11. Writing

BIL writes a small set of simple formats through writers of its own. Reading stays its main purpose.

- **Its own writers.** Writing does not go through the backing libraries. Each writer is code inside BIL, so the files it produces do not depend on which plugins are installed.
- **The geo model is written.** A writer writes the geo model of what it writes, without being asked, as far as the format holds it: GeoTIFF its affine, tie-point or RPC tags, and ENVI its map information. The model includes the `image_window` step of the Slice or Window written. What the format cannot hold goes to the companion file. `write_geo_model` writes a different model instead.
- **A Slice is written on its own grid.** The output Image has the Slice's size, and its geo model starts from the Slice's `origin` and `scale`, so a Slice from a reduced Level or a stored level is located correctly.
- **Sample types are never converted silently.** Each writer lists the sample types it holds. Writing any other is an error.
- **A subset by design.** A format is writable only where a simple, lossless writer is practical. Being able to read a format never implies BIL can write it.
- **New files only.** BIL never changes a Source it has opened for reading.
- **Round trip.** A file BIL writes must read back through BIL with identical pixel values.
- **How the input was read.** Everything BIL writes records the driver and codec, with their versions, that read its input, as the fields `bil.driver` and `bil.codec`. Where the format cannot hold them, they go to the companion file.
- **Markings are kept, and identity is not.** A security marking is written to everything BIL writes. A person's identity is left empty unless the caller asks to keep it. The rules are set out below.

**What can be written.** Native pixels as a Fragment, or float32 values as one or more Slices, each with its name kept as the component name.

| Call | Purpose |
| --- | --- |
| `list_writers()` | The formats BIL can write |
| `create_source(location, format_name, image_info)` | Starts a new file with a stated size, pixel type and component names. Takes `WriteOptions`. |
| `write_fragment(window, fragment)` | Writes native pixels for a Window |
| `write_slice(window, slice, component)` | Writes float32 values for a Window into one component. `component` is named; by default it is the component whose name is the Slice's name. |
| `write_geo_model(model)` | Writes a geo model, as far as the format can hold it. What it cannot hold goes to the companion file. |
| `write_metadata(fields)` | Writes the fields the format can hold (section 14). Markings and identity follow the rules below. |
| `close()` | Completes the file |

Writing is by Window, as reading is, so a large image can be written a block at a time without holding it all in memory.

**Markings and identity.** Field definitions flag two kinds of field for special handling when writing (section 14.1).

| Kind | Examples | Default when writing | The caller can |
| --- | --- | --- | --- |
| `Marking` | Security classification, handling caveats, releasability | Always written | Turn it off, with `keep_markings` set to false |
| `Identity` | A patient's name, identifier and date of birth | Left empty | Keep it, with `keep_identity` set to true |

- **Inside the file wherever there is room.** BIL writes markings inside the file wherever the format has a place for text: as keys in an ENVI header, and as text metadata in a TIFF. It also writes them to the companion file. A `.npy` file has no such place.
- **A companion file when the format has no place.** Where the output format cannot hold a marking field, BIL writes it to the output's companion file (section 14.2), as it does for any dictionary item the format cannot hold.
- **A missing companion is noticed.** A file BIL writes records that a companion file belongs to it. Reading it without that companion file gives status `Partial` and a reported warning, so markings are never lost silently.
- **Refusal on request.** With `require_internal_markings` set in `WriteOptions`, writing a marked Source to a format with no internal place for markings is an error. It is off by default.
- **Companion files are read back.** When BIL opens a file that has a companion file, it attaches the markings in it to the Source. They therefore survive any number of steps through BIL.
- **Carried without being asked.** Every Fragment and Slice keeps a reference to the markings of its Source. `write_fragment` and `write_slice` write them whether or not the caller calls `write_metadata`.
- **Not interpreted.** BIL does not rank or combine markings. When data from several Sources is written to one file, the markings of every Source are written.
- **Empty, not absent.** Where the output format has a place for an identity field, that place is written empty. Identity is never written to a companion file unless the caller keeps it.
- **The mapped format follows the same rules.** It holds markings itself, so it needs no companion file.

```rust
pub struct WriteOptions {
    pub keep_markings: bool,  // true unless the caller turns it off
    pub keep_identity: bool,  // false unless the caller turns it on
}
```

**Limits.** BIL empties only the fields its definitions flag as identity. It is not a tool for removing identity from data: identity can also sit in free text, in fields with no definition and in the pixels themselves. Once pixel data leaves BIL as a plain array, its reference to the markings is gone, and keeping them becomes the caller's task.

**Writable formats**

| Format | Why it suits a BIL writer |
| --- | --- |
| ENVI | A text header beside flat binary, with any number of bands. Holds `U8`, `U16`, `I16`, `U32`, `I32`, `U64`, `I64`, `F32`, `F64`, `ComplexF32` and `ComplexF64`. Not `I8`, `F16`, `ComplexI8` or `ComplexI16`. |
| TIFF and GeoTIFF | Uncompressed or Deflate, including BigTIFF. Holds every sample type, including complex integers. The most widely read raster format. |
| NumPy `.npy` | A minimal array file that Python code reads directly. Holds every sample type except `ComplexI8` and `ComplexI16`, which NumPy has no type for. |

**Also written.** BIL's own mapped format for externally decoded data (section 10.2).

**Not written.** Compressed video, raw camera files, JPEG 2000 and the SAR product formats are read-only.

## 12. Performance requirements

A Slice costs at most one pass over its Window, and that pass is built to run at the speed memory can be read and written, or at the speed of its arithmetic where that is slower.

**Bounded memory.** These follow from the purpose in section 1.1 and apply to every driver.

- Opening a Source reads no pixel data.
- Between reads, BIL keeps pixel data only in the Fragment cache, and the caller sets its size (section 4).
- No call needs memory in proportion to the whole Image, unless the caller asks for the whole Image.
- The cost of reading a Window depends on the Window, its Level and its slice function, not on the size of the Image around it.
- An Image of any size can be processed within a fixed memory budget, by Window or by stream.
- Where a format's own structure prevents this, such as one compressed block covering a whole image, `plan_slice` reports the real cost before the read.
- Blocks read ahead under `Sequential` are held within `read_ahead_bytes` and the cache's limits.

**The pass itself**

1. **One fused loop.** Byte swap, unpacking, component selection, normalization and conversion to float32 happen together, with no intermediate buffer.
2. **One request, one pass.** Every Slice and every Level a request asks for is computed in a single pass over a Fragment that is decoded once. What is not requested costs nothing.
3. **Local work stays in the pass.** Demosaicing and every bilinear resolution level are produced in the same pass, on a rolling buffer of a few rows that stays in the processor cache. No full-size intermediate image is created.
4. **Specialized loops.** One compiled loop exists per slice function for each common sample type: `U8`, `U16`, `I16`, `F32`, `ComplexI16` and `ComplexF32`. It is chosen once per read, so the inner loop has no branches or indirect calls. The other sample types are converted a row block at a time by a general loop and then mapped, which is slower but keeps the number of compiled loops small.
5. **Vector instructions chosen at run time.** Each loop has AVX2, AVX-512 and NEON versions, selected from the processor's features.
6. **Caller-owned buffers.** `read_slice_into` and `stream_slice` allocate nothing per call.
7. **Parallel rows and tiles.** Large Windows are split across threads. Gains flatten after a few cores, because memory bandwidth becomes the limit. How far decoding itself runs in parallel follows the driver's declared `concurrency` (section 10).
8. **Cheap variants offered.** Where a Slice needs costly arithmetic, a cheaper relative is provided: `power` avoids the square root in `magnitude`, and both avoid the arctangent in `phase` and the logarithm in `power_db`.
9. **No-copy hand-off to Python.** Slices and Fragments are exposed as NumPy arrays over the same memory, including strided and borrowed ones. Borrowed arrays are read-only under `Shared` mapping (section 4). The interpreter lock is released during every read.
10. **Measured, not assumed.** Each slice function has a benchmark against a plain memory copy of the same Window, and against its own arithmetic alone on data already in memory. The aim is the least cost each function's arithmetic allows: a function that only converts should run near the speed of a copy, and one that needs an arctangent or a square root for every pixel should run near the speed of that arithmetic. The figures guide the design, and no single factor applies to every function. Benchmarks run on dedicated machines, outside the build, because timing on shared build machines varies. Each has a threshold, and a regression past it is reported and must be fixed or explained before a release. The round trip to the interpreter's helper process is benchmarked in the same way (section 10.1).
11. **Reads follow the storage.** One request reads its blocks in the order they are stored. Between requests, `Sequential` reads and decodes ahead in that order, so a caller working through an Image finds its next blocks ready, and `Random` reads nothing it was not asked for (section 4). Both are benchmarked on files stored in strips, in tiles and band by band, locally and remotely.

## 13. Geocoding

BIL geocodes with a chain of steps between named coordinate spaces. Any format's own model fits this form, and so does several models joined end to end.

BIL computes where pixels are, in both directions. It does not resample imagery onto a map: orthorectification is a layer above BIL, as registration is.

### 13.1 Spaces, steps and chains

| Term | Meaning |
| --- | --- |
| Space | A named coordinate space: the pixels of a Slice, the full Image, the sensor, the Earth in a stated reference system, a patient, or the sky |
| Step | A mapping from one space to the next. It has a forward direction and, where one exists, an inverse. |
| Chain | An ordered sequence of steps. A chain is itself a step, so models nest. |
| `GeoModel` | A named chain that takes Image pixels to a world space |

This is the concatenated operation of ISO 19111, and it is how PROJ pipelines work. The word "space" is used because "frame" already means an index in this specification.

**The world is not always the Earth.** A medical image maps to a patient, an astronomical one to the sky, and a planetary one to another body. The world space of a `GeoModel` is whatever its last step produces.

### 13.2 The step library

The core owns the steps, and drivers supply the parameters. A driver's job is to map its format's metadata onto steps, as it does for normalization.

| Step | Form | Typical source |
| --- | --- | --- |
| `affine` | A linear map with an offset, in two or three dimensions | Map grids, world files, chips and crops, patient orientation |
| `projective` | A plane-to-plane map with eight parameters | Four corner points |
| `polynomial` | A polynomial in the input coordinates | Lens and detector distortion, replacement sensor models |
| `rational_polynomial` | A ratio of two cubic polynomials in latitude, longitude and height | RPC coefficients |
| `coordinate_grid` | A regular grid of known positions, interpolated. It may have a height dimension. | Per-pixel latitude and longitude arrays, SAR geolocation grids |
| `tie_points` | Scattered control points with a stated interpolation | Ground control points |
| `camera` | A pinhole, fisheye or equirectangular projection with lens distortion, giving a line of sight for each pixel | Frame cameras, video, surveillance domes, LIDAR and depth cameras |
| `pose` | Position and attitude, either fixed or looked up from the time coordinate | Platform and sensor angles, ephemeris and attitude records |
| `range_doppler` | Range and range-rate, giving a curve of possible positions | Complex SAR |
| `polar` | Range and beam angle to a position. Rows and columns are range and beam, not x and y. | Ultrasound, radar scans, imaging sonar, weather radar |
| `surface` | Intersects a line of sight or curve with a surface | A constant height, a terrain model read through section 13.6, or range measured by the sensor |
| `reference_change` | A change of datum or map projection | Computed by PROJ |
| `sphere_rotation` | A rotation of longitude and latitude on a sphere, by three Euler angles | The GIPS `mapping` field; the transverse and oblique aspects of any map projection |
| `external` | Hands the step to a sensor model plugin | Community Sensor Model plugins |
| `image_window` | The `origin` and `scale` of the Slice, which cover the Window, the Level and any stored level | BIL itself |

### 13.3 Rules

1. **BIL adds its own steps.** A Window, a resolution Level and a stored level all move pixels, and only BIL knows about them. BIL puts an `image_window` step at the front of every chain, so any pixel of any Slice can be located without the caller adjusting for how it was read.
2. **Pixel positions refer to pixel centers.** Whole-number positions are the centers of pixels. Drivers convert formats that measure from pixel corners.
3. **Image to world needs a surface.** A pixel gives a line of sight or a curve, not a point. The chain needs a `surface` step: a constant height, a terrain model, or measured range. A terrain model is itself read through BIL, by the `Terrain` interface of section 13.6. World to image needs no surface.
4. **Chains run both ways.** Each step states its native direction. A chain is inverted by reversing its order and inverting each step, by iteration where a step has no direct inverse. RPC, for example, runs natively from ground to image.
5. **Geometry can vary by index.** Video carries a model for each frame and a pushbroom sensor a pose for each row. A `pose` step looks these up through the time coordinate of section 3.1.
6. **Positions are 64-bit.** A 32-bit float resolves latitude and longitude to only about a meter, so geocoded positions are returned as 64-bit coordinates, not as Slices. Slices that carry positions, such as `depth.points`, are in a local space such as the sensor's, where 32 bits are enough.
7. **An Image can carry several models.** A NITF file may hold corner coordinates, RPC coefficients and a camera model at once. BIL lists them all, most rigorous first, and the caller may choose one by name.
    - **Named for the step that does the main work,** such as `rational_polynomial`, `projective`, `coordinate_grid`, `camera` or `range_doppler`. Where two models would share a name, both are numbered, as `rational_polynomial_1` and `rational_polynomial_2`.
    - **The format's own name is kept as the model's label,** as a Slice keeps its label.
8. **Callers can extend a chain.** A step or another chain can be added to either end, for example a `reference_change` to a map projection, or a correction found by a registration layer above BIL.
9. **A line of sight stops at the first surface it meets.** Where it crosses the terrain more than once, the result is the crossing nearest the sensor, which is the point the sensor saw.
10. **A position is returned only when it is sound.** Every result carries a status. An inverse that does not converge, or that has more than one answer, says so and returns no position.
11. **Longitude wraps.** Differences in longitude are taken the short way round, so a chain works across the antimeridian. A `coordinate_grid` is interpolated as positions on the sphere, not as separate latitude and longitude numbers, so it also stays correct over a pole.

The statuses are those of section 3.3. A position is `Complete`, `Missing`, `NotConverged` or `Ambiguous`.

### 13.4 Calls

| Call | Result |
| --- | --- |
| `list_geo_models()` | The models this Image carries, most rigorous first |
| `read_geo_model(model_name, frame)` | The chain for one frame |
| `for_slice(slice)` | The same chain with BIL's `image_window` step in front, built from the Slice's `origin` and `scale`, so it covers any Window, Level, stored level or `step` |
| `for_window(window, level)` | A short form of `for_slice` for a computed Level, before any Slice is read |
| `image_to_world(positions, surface)` | World positions, as 64-bit values, for pixel positions, each with a status. `surface` may be left out: a chain that needs one then uses the constant height of section 13.6, and a chain that ends in a plane, such as patient space, needs none. With `accuracy`, each position also carries its `Accuracy` (section 13.9). In Python each position has `lat`, `lon`, `height`, `status` and `accuracy`. |
| `world_to_image(positions)` | Pixel positions for world positions, each with a status |
| `list_steps()` | Each step of a chain, with its spaces and parameters |
| `concatenate(step_or_chain)` | A new chain with a step or chain added |
| `make_step(step_name, parameters)` | A step from the library of section 13.2, made by the caller, such as `make_step("reference_change", target="EPSG:32611")` |

**A worked chain.** A chip cut from a satellite image, stored as NITF with RPC coefficients and read at Level 2, is located like this.

1. `image_window`: from a Slice pixel to a pixel of the chip, undoing the Window offset and the factor of 4.
2. `affine`: from a chip pixel to a pixel of the full image the chip was cut from.
3. `rational_polynomial` with `surface`: from a full-image pixel and a height to latitude, longitude and height.
4. `reference_change`: from latitude and longitude to map coordinates.

### 13.5 How the formats supply chains

These mappings are from knowledge of the formats, and each is to be confirmed when its driver is built.

| Format family | Steps it supplies |
| --- | --- |
| GeoTIFF, JPEG 2000, world files, DTED | `affine`. Sometimes `tie_points` or `rational_polynomial` as alternative models. |
| NITF | `projective` from corner coordinates; `rational_polynomial`; a replacement sensor model as `polynomial` or `coordinate_grid`; `camera` and `pose` from frame camera data. A chip adds an `affine` in front. |
| SICD | `range_doppler`, with `pose` from the radar's position over time |
| SIDD and other detected SAR | `affine` on a plane or map grid, or `coordinate_grid` |
| Optical satellite products | `rational_polynomial`, or `camera` with a `pose` for each row |
| ENVI | `affine`, or `coordinate_grid` |
| HDF5, NetCDF, HDF4 | `affine`, or `coordinate_grid`, often coarser than the pixels |
| Video with KLV | For each frame: `camera` and `pose`, or `projective` from corner points |
| Wide area motion imagery | `camera` and `pose` for each camera and frame |
| Raw camera files | `camera` from lens data; position only as a single point |
| LIDAR and depth cameras | `camera`, with measured range as the `surface` |
| DICOM | `affine` to patient space |
| FITS | `affine`, `polynomial` and a sky projection |
| Ultrasound, radar scans, imaging sonar, weather radar | `polar`. Turning the polar grid into a picture is resampling, which stays above BIL. |
| Fisheye and 360° cameras | `camera` with a fisheye or equirectangular projection |

### 13.6 Terrain

BIL reads external terrain models through a `Terrain` interface, which gives the `surface` step its heights and intersections.

No open source library offers terrain access as a part that can be taken on its own (see the survey below). The job divides cleanly between things BIL already has and a thin layer of its own.

| Part of the job | Done by |
| --- | --- |
| Reading elevation pixels from DTED, GeoTIFF and cloud-optimized GeoTIFF | BIL's own drivers, by Window, so memory stays bounded |
| Treating many tiles as one surface | `Terrain` |
| Interpolating between elevation posts | `Terrain` |
| Converting heights from a geoid to the ellipsoid | PROJ, through geoid grids read from local files |
| Intersecting a line of sight or curve with the surface | `Terrain` |

**Calls**

| Call | Result |
| --- | --- |
| `open_terrain(location)` | A `Terrain` from one file, a folder of tiles or a remote store |
| `constant_terrain(height)` | A level surface at one height above the ellipsoid |
| `caller_terrain(function)` | A surface supplied by the caller's own function |
| `height_at(positions)` | Heights above the ellipsoid, as 64-bit values |
| `intersect(lines_of_sight)` | Where each line of sight or curve first meets the surface, counting from the sensor |
| `coverage()` | The area the terrain covers and its post spacing |

A `Terrain` is what a caller passes as the `surface` argument of `image_to_world`.

**Rules**

1. **Heights come out above the ellipsoid.** Elevation data is usually stored relative to a geoid, which approximates mean sea level, while sensor models work in height above the ellipsoid. The two can differ by tens of meters. `Terrain` converts, using the vertical reference stated in the data.
2. **Interpolation is `bilinear` by default.** `nearest` and `bicubic` are also available.
3. **Voids are missing data.** Where the terrain has no value, `Terrain` falls back in a stated order: to a coarser terrain if one was given, then to a constant height, and otherwise it reports the position as `Missing`.
4. **Tiles are read on demand.** A lookup touches only the tiles it needs, and recently used tiles stay in the Fragment cache of their Source, so the cost follows the request (section 3.4).
5. **Surface or bare earth is reported.** A terrain model may describe the bare ground, or the surface including buildings and vegetation. `Terrain` says which, because it changes where a line of sight lands.
6. **No terrain is fetched by default.** With no surface given, `image_to_world` uses a constant height. That is the reference height the geo model states, such as the height offset of RPC coefficients, or zero above the ellipsoid where the model states none. The result says which surface was used. Remote terrain is read only when the caller opens it with `open_terrain` (section 1.3).

**What already exists** (checked on 11 October 2026)

| Library | What it offers | How BIL uses it |
| --- | --- | --- |
| [GDAL](https://gdal.org/en/stable/api/gdal_alg.html) | Its RPC transformer takes a DEM in any format GDAL reads, with nearest, bilinear or cubic sampling, a value for missing heights and an automatic shift from geoid to ellipsoid heights | The most complete, but built into the RPC transformer and not offered to other sensor models. It stays available as an alternative terrain source through the GDAL plugin. |
| OSSIM, as described by [Orfeo ToolBox](https://www.orfeo-toolbox.org/SoftwareGuide/SoftwareGuidech7.html) | A handler over a folder of DTED or SRTM tiles, with a geoid file and a default height, returning height above the ellipsoid or above sea level | The closest design to `Terrain`, and its model. It is part of a large processing library that BIL does not otherwise load. |
| [PROJ](https://proj4.org/en/stable/operations/transformations/vgridshift.html) | Vertical datum shifts from geoid grids such as EGM96 | Adopted for the geoid conversion |
| [SARkit](https://sarkit.readthedocs.io/en/stable/reference/generated/sarkit.sicd.image_to_dem_surface.html) | Projects SAR image points onto a DEM that the caller supplies as a function | It does not read terrain itself. Its pattern is kept as `caller_terrain`. |
| Rust crates such as [dted](https://github.com/fizyk20/dted) | Reading single elevation files | Small and little maintained, with no tiling, interpolation or geoid handling |

**Candidate remote data.** BIL never fetches terrain unasked, but a caller can open a remote store. The [Copernicus DEM](https://registry.opendata.aws/copernicus-dem/) is published as cloud-optimized GeoTIFF tiles at 30 m and 90 m, in public storage that needs no account. It is a surface model, so it includes buildings and vegetation, and some 30 m tiles are withheld.

### 13.7 The GIPS model

BIL adopts the geocoding model of GIPS, which describes any map projection, in any aspect, with one short set of parameters. A reader for GIPS files is not planned.

GIPS is the General Image Processing System, written by Peter G. Ford at the MIT Center for Space Research and used for Pioneer Venus and Magellan radar data. This section is drawn from the GIPS command manual of February 1992.

**Two layers.** A GIPS image is located in two stages, each held in its text header.

| Layer | Header fields | What it does | BIL step |
| --- | --- | --- | --- |
| Axes | `xmin`, `xmax`, `ymin`, `ymax`, `xscale` | Maps pixel positions linearly to "realworld" x and y, usually longitude and latitude in degrees. `xscale = w` marks an x axis that wraps through 360 degrees. | `affine` |
| Mapping | `mapping` | Present on an image that has been projected. Relates pixel positions to longitude and latitude through a map projection. | `affine`, `reference_change` on a sphere, and `sphere_rotation` |

**The mapping field.** It is one line of text with six parts after the name.

| Part | Meaning |
| --- | --- |
| `type` | The projection, numbered 0 to 22 |
| `origin` | The pixel on which the center of the projection falls |
| `radius` | The scale of the projection. Its exact meaning depends on the type. |
| `euler` | Three angles that rotate longitude and latitude on the sphere before the projection is applied |
| `lat` | One or two special latitudes, for conic projections and others that need them |
| `xyrot` | When not zero, a final quarter turn counterclockwise |

**Why the model is worth adopting**

- **Any aspect from one definition.** The sphere is rotated first, and the projection is then applied in its plain form. A transverse or oblique version of any projection needs no formulas of its own.
- **Uniform.** The same six parts describe every projection.
- **Both directions.** GIPS supplies the mapping from longitude and latitude to pixel, and its inverse.
- **It is already a chain.** A scale and origin, a projection and a rotation are three steps in BIL's terms.

**Projection types**

| Type | Projection | Type | Projection |
| --- | --- | --- | --- |
| 0 | Tie-point polynomial | 12 | Albers, two parallels |
| 1 | Gnomonic | 13 | Lambert cylindrical equal-area |
| 2 | Stereographic | 14 | Lambert polar azimuthal equal-area |
| 3 | Orthographic | 15 | Bonne |
| 4 | Azimuthal equidistant | 16 | Sinusoidal |
| 5 | Azimuthal equal-area | 17 | Werner |
| 6 | Lambert conformal, one parallel | 18 | Defined by the user in code |
| 7 | Lambert conformal, two parallels | 19 | Aitoff |
| 8 | Mercator | 20 | Hammer |
| 9 | Polar stereographic | 21 | Briesemeister |
| 10 | Mollweide | 22 | Cylindrical |
| 11 | Albers, one parallel | | |

**How BIL represents it.** For an image projected this way, the chain from a Slice pixel to the world is:

1. `image_window`: from a Slice pixel to an Image pixel.
2. `affine`: undoes `origin`, `radius` and `xyrot`.
3. `reference_change`: the inverse of the projection of that `type`, on a sphere, with its special latitudes. PROJ computes it where PROJ has the projection, and BIL supplies the rest.
4. `sphere_rotation`: undoes the Euler rotation, giving longitude and latitude.

- **An unprojected image** needs only `image_window` and an `affine` built from the axis limits. The manual indicates that the limits are the outer edges of the first and last pixels, so BIL converts them to pixel centers (section 13.3).
- **`sphere_rotation` is a general step.** It joins the step library of section 13.2, and any format or caller may use it.
- **Two types cannot be located from the header.** A tie-point image (type 0) does not record its polynomial, and a user-defined projection (type 18) exists only in the user's code. For these the caller supplies the step.

**The world is a sphere with no name.** GIPS works on a sphere, and its header records neither the body nor its radius. The world space of a GIPS chain is longitude and latitude on a sphere. The caller names the body to go further.

**What BIL keeps from GIPS.** GIPS is an old package, and BIL takes its lessons without planning to read its files.

| Lesson | Where it went |
| --- | --- |
| Rotate the sphere first, then project | The `sphere_rotation` step (section 13.2) |
| A logarithmic scale for stored values | The `log10` normalization function (section 7) |
| A polynomial in the stored value | The `value_polynomial` normalization function (section 7) |

**If a reader were ever wanted.** The format is a text header of `name = value` items followed by flat binary rasters, which suits a scripted driver. Several conventions would need confirming first, from the "Guide to GIPS" (Ford, 1984) or from sample files: the meaning of `radius` for each type, the order and sign of the Euler angles, the sign of longitude, and whether the axis limits are pixel edges.

### 13.8 Validation

BIL implements rigorous sensor models itself, and proves each one by test against an established implementation.

- **Tested, not delegated.** An `external` step remains available for a sensor model that BIL does not have. It is not the default route for a model BIL can implement and test.
- **Every step has a reference.** A step is released only when its positions agree with its reference.
- **Every step runs both ways.** Image to world and back must return the starting pixel.

| Step | Reference to test against |
| --- | --- |
| `affine`, `projective`, `polynomial` | Exact arithmetic, and GDAL for map grids |
| `rational_polynomial` | GDAL's RPC transformer |
| `range_doppler` | SIX, for SICD |
| `camera` with `pose` | A Community Sensor Model plugin for the same sensor, where one exists |
| `coordinate_grid`, `tie_points` | GDAL's geolocation array and control point transformers |
| `sphere_rotation` | PROJ's oblique transformation |
| `reference_change` | None beyond PROJ's own tests, since PROJ computes it |
| `surface` with terrain | GDAL's RPC transformer with an elevation model |

**The tests**

| Test | A step passes when |
| --- | --- |
| Agreement | Its positions match the reference to within the tolerance, across the whole Image and not only near its center |
| Round trip | Image to world and back returns the starting pixel to within the tolerance |
| Window and Level | The position of a scene point is the same whichever Window, Level or stored level it is read through |
| Known points | Where a file comes with surveyed ground points, BIL's positions match them to the accuracy the file claims |
| Honest failure | Cases built to have no single answer return `NotConverged` or `Ambiguous`, and never a position |

**Tolerance.** The default is one hundredth of a pixel in the Image. A step may state a tighter one.

**A second, independent check.** GRDL (A.15) is a candidate second reference for the `range_doppler` step with SICD and for the `rational_polynomial` step. Its geolocation was written separately from SIX and GDAL, so agreement with all of them is stronger evidence than agreement with one.

**Where no reference exists.** A step with no established implementation to compare against is tested by round trip and by known points only, and its documentation says so.

### 13.9 Accuracy

Every position BIL computes can come with a full record of how accurate it is, carried forward through every step of the chain and never reduced to a single number until the caller asks for one.

**Where error estimates come from.** Formats state error in many forms. These are from knowledge of the formats, and each is to be confirmed when its driver is built.

| Source | What it states | Becomes |
| --- | --- | --- |
| RPC coefficients in NITF | A bias error and a random error, in meters | A bias and a random contribution of the `rational_polynomial` step |
| Replacement sensor model in NITF | A covariance of adjustable parameters, and a direct error covariance with correlation over the image | A parameter covariance and an output covariance of the step |
| SICD and SIDD | Error statistics: a composite covariance at the scene point, and component errors for the platform's position and velocity, the radar and the atmosphere | Parameter covariances of `pose` and `range_doppler`, or one output covariance |
| Community Sensor Model plugins | A covariance of sensor parameters, and partial derivatives | Passed through the `external` step |
| Video with KLV | Standard deviations and correlations of platform and sensor position and angles, where the stream carries them | A parameter covariance of `pose` for each frame |
| Terrain | A vertical accuracy, such as the absolute vertical accuracy in a DTED header or a stated accuracy for a model | A contribution of the `surface` step |
| The caller | A covariance for any step it makes or adds, such as a correction from a registration layer, and for the measured pixel positions | Contributions of those steps and of the pixels |

- **Converted once, kept as stated.** BIL converts each estimate to a covariance at one standard deviation, in meters or pixels. The value as the format stated it, such as a CE90, remains readable as a field.
- **An assumption is recorded as one.** Where BIL must assume something to convert, such as a circular error that is equal in every direction, the record says so.

**Kinds of error, kept apart**

| Kind | Meaning | Why it is kept apart |
| --- | --- | --- |
| Bias | The same error for every point of the Image or frame, such as an error in the platform's position | It cancels between two points of one Image, so it decides relative accuracy |
| Random | An error independent from point to point | It does not cancel |
| Correlated | An error that varies smoothly across the image or over time, with a stated correlation | Two nearby points share most of it |

**Carried through the chain**

- **Each step supplies its partial derivatives,** with respect to its input and to its parameters. BIL combines them, step by step, to carry every contribution to the final space. This is exact for linear steps, such as `affine` and `image_window`.
- **`image_window` adds no error.** A Window, Level or stored level changes the pixel units, and the covariance is scaled with them.
- **The surface contributes its own error.** An error in height moves the ground point along the line of sight, so terrain error is carried through the geometry of the intersection, not added as a vertical term alone.
- **A step with no estimate is marked unknown.** It is never treated as perfect.
- **First order by default, checked by sampling.** The `accuracy_method` argument is `first_order`, the default, or `sampled`. The sampled method propagates a set of points through the chain itself, as an unscented transform, and confirms the first-order result where the geometry is strongly curved, such as a line of sight near grazing.

**The record**

An `Accuracy` is returned for each position, as its `accuracy`, when the caller asks for it.

| Part | Content |
| --- | --- |
| `covariance` | The full covariance of the position: 3 × 3 in a local east, north and up space at the point, in meters, for the Earth. For any other world, in that world's own space. For `world_to_image`, 2 × 2 in pixels. |
| `contributions` | One `Contribution` for each source of error |
| `status` | `Complete` when every step has an estimate, `Partial` when some steps have none, and `Missing` when none has |

**A contribution**

| Part | Content |
| --- | --- |
| `name` | The step and the quantity in error, such as `rational_polynomial.error`, `pose.position` or `surface.height` |
| `kind` | `Bias`, `Random` or `Correlated`. A name and a kind together identify a contribution, so the bias and random error of RPC coefficients are two contributions. |
| `covariance` | Its own covariance, in the same space and units as the total |
| `provenance` | `Stated` by the format, `Derived` by BIL, `Given` by the caller, or `Assumed`, with the field it came from |

- **Never smaller than the truth by omission.** A total with unknown contributions is `Partial`, and the unknown steps are named.
- **Computed only when asked.** Positions without accuracy cost no more than before.

**Calls**

| Call | Result |
| --- | --- |
| `image_to_world(positions, surface, accuracy, accuracy_method)` | With `accuracy` true, each position with its `Accuracy`. `accuracy_method` is `first_order` by default. |
| `world_to_image(positions, accuracy, accuracy_method)` | With `accuracy` true, each pixel position with its `Accuracy` in pixels |
| `relative_accuracy(positions_a, positions_b)` | The covariance of the difference between paired positions, in which shared bias cancels and correlated error partly cancels |
| `list_error_sources()` | Each contribution the chain carries, with its kind and provenance |
| `circular_error(accuracy, probability)` | The horizontal radius holding the position with the given probability, such as CE90 |
| `linear_error(accuracy, probability)` | The vertical distance holding the position with the given probability, such as LE90 |
| `error_ellipse(accuracy, probability)` | The horizontal ellipse holding the position with the given probability: semi-axes and orientation |

- **Exact summary measures.** `circular_error` integrates the two-dimensional normal distribution of the covariance as it is, without assuming the error is circular and without a fixed conversion factor.
- **Steps the caller makes carry error too.** `make_step` takes an optional `covariance`, so a correction from a registration layer above BIL carries its own accuracy forward.

**Carried forward when written.** `write_geo_model` writes the error estimates into the output wherever the format has a place for them, such as the bias and random error fields of RPC coefficients in a GeoTIFF. Otherwise they go to the companion file, so the record is never lost by writing (sections 11 and 14.2).

**Tested**

| Test | Passes when |
| --- | --- |
| Agreement | Propagated covariances match a reference implementation where one exists, such as the SICD error propagation of SIX and SARkit |
| Coverage | Over many simulated collections with known errors, the true position falls inside the CE90 circle in 90 percent of cases, to within sampling error |
| Methods agree | `first_order` and `sampled` agree where the geometry is nearly linear, and their difference is reported where it is not |
| Window and Level | The accuracy of a scene point is the same, in meters, whichever Window, Level or stored level it is read through |

## 14. Metadata

Metadata is held as named fields with values. A field's name comes from the file itself or from the specification of its format.

Geocoding, normalization parameters and coordinates are built from metadata, and each has its own interface. This section covers everything else, and the fields behind those three.

### 14.1 Fields

| Part of a field | Meaning |
| --- | --- |
| Name | A namespace, then the native name, as a list of parts: `nitf.file_header.FTITLE`, `dicom.PatientName` |
| Value | Text, integer, 64-bit float, boolean, date and time, bytes, a list of one of these, or a group of further fields |
| Unit | The unit of the value, where the file or the specification states one |
| Origin | `SelfDescribed`, `Specified`, `Derived`, `Companion` or `Caller` |
| Attachment | A Source, an Image, or one index of an Image: row, column, component, frame or a declared additional index |
| Handling | `Ordinary`, `Marking` or `Identity`. It decides what a writer does with the field (section 11). |

**Where names come from**

| Origin | The name comes from | Examples |
| --- | --- | --- |
| `SelfDescribed` | The file, which writes a name beside each value | HDF5 and NetCDF attributes, ENVI header keys, FITS keywords, XML such as SICD and OME |
| `Specified` | The format's specification, because the file holds only a code or a fixed position | NITF header fields and tagged record extensions, DICOM tags, TIFF and Exif tags, MISB KLV keys |
| `Caller` | The caller, who added the field | A task identifier, or a result from a layer above BIL |

- **Native names are kept exactly.** The part of a name after the namespace is spelled as the file or the specification spells it, so it can be looked up in the format's own documents. BIL's own naming applies to the namespace only.
- **A name is a list of parts.** Each part is one level of the format's own structure: a group, a record or a field. Every call accepts the list, or its dotted form, in which the parts are joined by dots.
- **Dots inside a part are escaped.** HDF5 paths and XML names may contain dots. In the dotted form such a dot is written `\.`, as in `hdf5.calibration.band\.1`.
- **Repeats carry a number.** Where a native name occurs more than once, every instance carries its number, counted from 1, in square brackets: `nitf.RPC00B[2]`, `fits.HISTORY[3]`, `dicom.ReferencedImageSequence[1].ReferencedSOPInstanceUID`. Outside a counted run, a name that occurs once has no number.
- **TIFF's fields.** An Image's tags are the group `tiff.ifd`, named as TIFF 6.0, the GeoTIFF standard and GDAL name them (`tiff.ifd.ImageWidth`, `tiff.ifd.GDAL_NODATA`); its GeoTIFF keys are the group `geotiff.keys`, named as GeoTIFF 1.0 names them (`geotiff.keys.GTModelTypeGeoKey`). A tag or key with no known name is named by its number in decimal (`tiff.ifd.65000`), so nothing in the file is hidden. Text is read from its own bytes, as Latin-1, every string of a tag kept. An unsigned 64-bit value above the largest signed one is delivered as a float, and its stored bytes keep it exactly. A tag that cannot be read leaves `tiff.ifd` `Partial`; a key directory that cannot be read fails `geotiff.keys` as corrupt, since its keys are found only through it.
- **Runs carry their place.** A field in a counted run of a format's structure, such as one band's fields in a NITF image subheader, always carries its place in the run, even when the run has one entry, since the number says which band or segment it belongs to: `IREPBAND[1]`, `LISH[1]`. A field inside two runs carries one number for each, outermost first: `LUTD[2][1]` for the second band's first lookup table.
- **Text is Unicode.** Text is decoded from the character set the format declares. The result of `read_field` still carries the stored bytes.
- **A date and time states whether its zone is known.** With a known zone it is held in UTC, with the offset the file stated. Without one it is held as local time, marked as having no known zone, and is never silently taken as UTC.
- **Field definitions name the specified fields.** A definition gives a code or position its name, value type and unit. Each driver carries the definitions for its formats.
- **Definitions can be added.** A driver, a scripted driver or a caller can add definitions without rebuilding BIL, for example for a new NITF extension or a vendor's private DICOM tags.

**The form of a field definition.** Definitions are records with these parts, kept as JSON when they are in a file.

| Part | Meaning | Example |
| --- | --- | --- |
| `name` | The native name, as a list of parts | `["nitf", "VNDRTA", "GAIN"]` |
| `locator` | Where the value is, in the format's own terms | `{"extension": "VNDRTA", "offset": 12, "length": 6}`, or `{"tag": "0018,1150"}` for DICOM |
| `value_type` | Text, integer, float, boolean, date and time, bytes, or a list of one of these | `"float"` |
| `encoding` | How the stored bytes become the value | `"ascii_decimal"`, `"big_endian_u16"` |
| `unit` | The unit, where there is one | `"dB"` |
| `handling` | `Ordinary`, `Marking` or `Identity` | `"Ordinary"` |
| `provides` | The dictionary item it provides, where it is plainly the same (section 14.2) | `"common.exposure_time"` |

The `locator` and `encoding` each format accepts are listed by its driver.

- **Codes up to the format's length.** A locator's code matches the code the file stores with its padding removed, so where a format pads its codes, a code of any length up to the format's own is accepted: a NITF extension tag of one to six characters. Each driver states its formats' limit beside the locators it lists: a longest length where codes are padded, as NITF's tags are, or a fixed length where they are not, as a DICOM tag is. A definition whose code breaks the limit is refused with an error when it is added, rather than never matching.
- **A family of codes.** A locator's code may hold a wildcard, `*`, so that one definition serves a family of codes, such as `RPC00*` for RPC00A and RPC00B. A field's name keeps the code matched in each file, so native names stay exact: `nitf.RPC00B.LINE_OFF` in a file that holds RPC00B.
- **Wildcards.** `*` stands for any run of characters, including none, anywhere in a code; the other characters count against the format's limit, and a code needs at least one. Where several definitions match a code, one with no wildcard is preferred, then the one with the most characters fixed; a definition that ties with one already present is refused when added. There is no way to name a literal `*`, so a stored code that holds one is matched only by a family.
- **Ordered layouts.** Where a format's fields follow one another, with presence, lengths and counts that depend on earlier fields, as NITF's headers and most of its extensions do, a locator gives the field's place in an ordered layout rather than an offset. Each field has a `length`, a number or another field's value less a number, and may have `present_if`: another field's stored bytes among given values, not among them, or above zero. Runs of fields repeat by a `count`: a number, another field's value, or one field's value or another's where that is 0. For example: `{"layout": "nitf.image_subheader", "after": "ICORDS", "length": 60, "present_if": {"field": "ICORDS", "not": [" "]}}`.
- **Nothing is hidden.** A value with no definition is still listed, under its native code, with its value as bytes.

### 14.2 The metadata dictionary

BIL keeps a dictionary of the essential metadata that its users need, so that every format offers the same items under the same names, as far as its contents allow.

**Three ways an item gets its value.** For each Source, BIL fills a dictionary item from the first of these that works.

| Order | Source of the value | Origin reported | Example |
| --- | --- | --- | --- |
| 1 | **Provided.** The format holds the item directly, in a native field that is plainly the same thing | `SelfDescribed` or `Specified`, as for the native field | NITF `IDATIM` gives `common.collection_time` |
| 2 | **Derived.** The format does not hold it, but BIL computes it by a known formula from other values | `Derived` | `common.sun_elevation` from the collection time and the center position |
| 3 | **From a companion file.** Neither works, and a companion file beside the Source supplies it | `Companion` | A sun elevation written by the user for a format that cannot hold one |

A value the caller adds with `add_field` comes before all three, with the origin `Caller`.

**What the dictionary holds.** Each item has a name in the `common` namespace, a definition, a value type, a unit, and what it attaches to, as section 14.1 lists.

**Provided items.** A native field provides an item only when it is plainly the same thing.

1. **Same meaning.** Both the specification and the dictionary describe the same thing in plain terms.
2. **One to one.** One native field gives the item. Nothing is combined or chosen between.
3. **Conversion of form only.** At most a change of unit, or of how a date or number is written.
4. **If in doubt, not provided.** The item is then derived, or taken from a companion file.

The mapping lives in the native field's definition, which names the dictionary item it provides. An item that is provided is a second name for the native field, not a copy, and the native field stays readable.

**Derived items**

- **A stated formula.** Each derived item names the formula that computes it. `describe_field` shows the formula and the inputs that were used.
- **The same formula for every format.** Formulas belong to the core, not to drivers, so a derived item means the same thing whatever the format.
- **Computed when asked.** A derived item is computed the first time it is read, or when a function needs it, and then held like any other field.
- **Inputs from anywhere.** A formula may use provided values, other derived values, companion values or the caller's own.
- **Honest when inputs are missing.** If an input cannot be found, the item's status is `Missing` (section 3.3).

**First entries** (illustrative; the list is settled as drivers are built)

| Item | Value | Provided by, for example | Derived from |
| --- | --- | --- | --- |
| `common.sensor_name` | Text | TIFF and Exif `Model`, DICOM `ManufacturerModelName` | Not derived |
| `common.platform_name` | Text | A satellite or aircraft name, where the format states one | Not derived |
| `common.collection_time` | Date and time, in UTC where the zone is known | NITF `IDATIM`, the KLV time stamp, Exif `GPSDateStamp` with `GPSTimeStamp`, which are in UTC | Exif `DateTimeOriginal`, where the GPS time is absent, taken as local time with its zone found as below; or the time coordinate of frame 0 |
| `common.title` | Text | NITF `FTITLE`, the NetCDF `title` attribute | Not derived |
| `common.exposure_time` | Seconds | Exif `ExposureTime` | Not derived |
| `common.center_position` | Latitude, longitude and height | Formats that state a scene center | The geo model, at the center of the Image |
| `common.footprint` | The positions of the four corners | NITF `IGEOLO` | The geo model, at the corners of the Image |
| `common.ground_sample_distance` | Meters, along rows and along columns, on the ground. In the image's own plane it is `common.sample_spacing`. | Product formats that state it | The geo model, from neighboring pixels at the center |
| `common.sun_elevation`, `common.sun_azimuth` | Degrees | Optical satellite products | A solar position formula, from the collection time and the center position |
| `common.earth_sun_distance` | Astronomical units | Optical satellite products | A formula, from the collection time |
| `common.view_elevation`, `common.view_azimuth` | Degrees | Products that state the sensor's look direction | The sensor's position in the geo model, and the center position |
| `common.incidence_angle` | Degrees | SAR products | The sensor's position, the center position and the surface |
| `common.band_center` | Nanometers, for each component | ENVI `wavelength`, HDF5 attributes | The middle of each band's lower and upper edges |
| `common.radar_wavelength` | Meters | SAR products | The radar's center frequency |
| `common.gate_range` | Meters, for each frame | Range-gated sensors | Not derived |
| `common.projection_angle` | Degrees, for each frame | CT projections | Not derived |
| `common.color_primaries`, `common.transfer_characteristics`, `common.matrix_coefficients`, `common.video_full_range` | H.273 code points | A stream's own code points; format color tags that are plainly the same | Not derived; a stated default applies (section 9.3) |

**Local times.** Exif and some other formats record the time on the camera's clock, with no zone. BIL takes such a time as local time at the place of collection, and finds the zone in this order.

1. **A stated offset.** Exif `OffsetTimeOriginal`, where the file has it.
2. **The sensor location.** From the Exif GPS position, or from the sensor position in the geo model. Used only when a time-zone lookup is installed.
3. **The footprint.** From `common.center_position`, which is the center of the footprint. Used only when a time-zone lookup is installed.
4. **None found.** The time is taken as local time, marked as having no known zone (section 14.1).

- **Zone from position needs an optional plugin.** A time-zone lookup finds the zone from a map of time-zone boundaries, and the offset for that date, daylight saving included, comes from the IANA time zone database. BIL does not require one. Where one is installed, such as a plugin over the Rust crate tzf-rs, BIL uses it. Where none is, steps 2 and 3 are skipped.
- **Not in BIL's packages.** The boundary maps available, including the one tzf-rs carries, are under the Open Database License. A lookup is therefore an optional plugin that the user installs, as Grok is (section 10).
- **Loaded only when needed.** The boundary map is large, so a lookup reads it the first time a zone has to be found.
- **Stated.** `describe_field` reports which of the four ways gave the zone, so a time taken as local because no lookup was installed is visible as such.

**More first entries,** for the domain profiles below (illustrative, as above)

| Item | Value | Provided by, for example | Derived from |
| --- | --- | --- | --- |
| `common.processing_level` | Text, as the producer names it | Product formats that state it | Not derived |
| `common.reference_system` | The coordinate reference system of the geo model, as an EPSG code or a full description | GeoTIFF keys, NITF `ICORDS` | The geo model |
| `common.horizontal_accuracy`, `common.vertical_accuracy` | Meters, at 90 percent | Products that state CE90 and LE90 | The accuracy record at the center of the Image (section 13.9) |
| `common.band_width` | Nanometers, for each component | ENVI `fwhm` | The difference of each band's upper and lower edges |
| `common.radiometric_gain`, `common.radiometric_offset` | For each component, in the units of `common.units` per stored unit | Product calibration fields | The Image's normalization parameters, where they are a linear scale (section 7) |
| `common.units` | The unit of the calibration that `common.radiometric_gain` and `common.radiometric_offset` produce, for each component, and nothing else. A Slice's own unit is part of its definition (section 5). | Product formats, HDF5 and NetCDF `units` | The Image's normalization |
| `common.solar_irradiance` | Watts per square meter per micrometer, for each component | Optical satellite products | Not derived |
| `common.thermal_constants` | The two constants that turn radiance into brightness temperature, for each component | Thermal products | Not derived |
| `common.polarization` | The transmit and receive polarization of each component | SAR products | Not derived |
| `common.look_side` | `left` or `right` | SAR products | The sensor's position and velocity in the geo model, and the center position |
| `common.sample_spacing` | The distance between samples along rows and along columns, with its unit, in the image's own plane, such as slant range for SAR. On the ground it is `common.ground_sample_distance`. | SAR products, DICOM `PixelSpacing` | The geo model, where it has one |
| `common.calibration_constant` | The radar calibration constant, for each component | SAR products | Not derived |
| `common.frame_rate` | Frames per second | Video streams | The time coordinate of the frames |
| `common.sensor_position`, `common.sensor_attitude` | Position, and roll, pitch and yaw, for each frame | MISB KLV, camera models | The geo model's camera step |
| `common.field_of_view` | Degrees, horizontal and vertical | MISB KLV | The geo model's camera step |
| `common.camera_intrinsics` | Focal length, principal point and distortion, in pixels | Depth and camera formats | The geo model's camera step |
| `common.modality` | Text | DICOM `Modality` | Not derived |
| `common.slice_thickness` | Millimeters | DICOM `SliceThickness` | Not derived |
| `common.image_orientation`, `common.image_position` | Direction cosines, and millimeters | DICOM `ImageOrientationPatient` and `ImagePositionPatient` | Not derived |
| `common.depth_scale` | Meters per stored unit | Depth camera formats | Not derived |
| `common.f_number`, `common.focal_length`, `common.sensitivity` | A ratio, millimeters, and ISO speed | Exif `FNumber`, `FocalLength` and `PhotographicSensitivity` | Not derived |

**Domain profiles.** Each problem domain needs a known set of metadata, whatever sensor or format the data came from. A profile names that set.

- **Dictionary items only.** A profile lists dictionary items, never native fields, so one profile applies to every format. Each format fills the items through the three ways above.
- **Required or recommended.** Each item of a profile is, as a `ProfileNeed`, `Required`, without which the domain's work cannot be done, or `Recommended`, which improves it.
- **Profiles build on profiles.** A profile's required or recommended list may name another profile, whose items then count as required or recommended in it. `optical_radiometry` requires `geolocation` rather than repeating its items, and `thermal` recommends it.
- **Checked, not enforced.** `check_profile` reports what a Source holds against a profile. It never refuses a read or holds back a function. What a function itself needs remains its own declaration, above.
- **Added by layers above.** An algorithm above BIL can state its needs once as a profile, with `add_profile`, and check any Source against it before reading a pixel.
- **Gaps can be filled.** A missing item can be supplied by `add_field` or in a companion file, as for any dictionary item. `check_profile` names each one, with whether it could be derived and which inputs are missing.

**A profile report.** `check_profile(profile_name, index)` returns a `ProfileReport`. For fields attached to an index, such as those of each frame, the caller names the index; otherwise it is left out.

| Part | Content |
| --- | --- |
| `status` | `Complete` when every required item has a value, `Partial` when some do, and `Missing` when none does (section 3.3). Recommended items do not change it. |
| `items` | For each item: its `name`; its `need`, `Required` or `Recommended`; its `status`; its `origin`; and its `basis`, the native field or the formula and inputs behind it, as `describe_field` gives them (section 14.1) |

**First profiles** (illustrative, settled with the dictionary as drivers are built)

| Profile | Required | Recommended |
| --- | --- | --- |
| `provenance` | `sensor_name`, `collection_time` | `platform_name`, `processing_level`, `title` |
| `geolocation` | `footprint`, `reference_system` | `center_position`, `ground_sample_distance`, `horizontal_accuracy`, `vertical_accuracy` |
| `optical_radiometry` | `provenance`, `geolocation`, `band_center`, `radiometric_gain`, `radiometric_offset`, `units`, `sun_elevation`, `earth_sun_distance` | `band_width`, `solar_irradiance`, `sun_azimuth`, `view_elevation`, `view_azimuth` |
| `thermal` | `provenance`, `band_center`, `radiometric_gain`, `radiometric_offset`, `units`, `thermal_constants` | `geolocation`, `view_elevation` |
| `sar` | `provenance`, `geolocation`, `radar_wavelength`, `polarization`, `look_side`, `incidence_angle`, `sample_spacing` | `calibration_constant`, `view_azimuth` |
| `motion_imagery` | `provenance`, `frame_rate`, `sensor_position`, `sensor_attitude`, `field_of_view` | `geolocation` |
| `photography` | `collection_time`, `exposure_time`, `f_number`, `focal_length` | `sensor_name`, `sensitivity`, `color_primaries`, `transfer_characteristics`, `center_position` |
| `medical` | `modality`, `sample_spacing`, `image_orientation`, `image_position` | `slice_thickness`, `units`, `sensor_name` |
| `depth_sensing` | `depth_scale`, `camera_intrinsics`, `units` | `collection_time`, `sensor_position`, `sensor_attitude` |

Every item above is in the `common` namespace, and a profile named in a list stands for its items. Where an item is reached twice, the stronger need applies. A profile that names itself, directly or through others, is refused by `add_profile` and `load_profiles`. A profile holds no identity field and no marking: `medical` says what the data is, never whom it is of.

**The form of a profile.** Profiles are records with these parts, kept as JSON when they are in a file, as field definitions are.

| Part | Meaning | Example |
| --- | --- | --- |
| `name` | The profile's name | `"sar"` |
| `description` | What the domain's work is | `"Calibrated SAR backscatter and geometry"` |
| `required` | Dictionary items, and profiles, without which the work cannot be done | `["provenance", "geolocation", "common.radar_wavelength"]` |
| `recommended` | Dictionary items, and profiles, that improve it | `["common.calibration_constant"]` |

**Left out on purpose.** Security classification is not a dictionary item. Classification schemes differ between formats and nations, and a wrong mapping does harm, so markings are always read and written in their native form.

**What a function needs.** Slice functions, normalization functions and geocoding steps each declare every input they need: dictionary items and normalization parameters alike. `spectral.toa_reflectance`, for example, needs the sun elevation and the Earth–Sun distance.

- `list_required_fields(function_name)` lists every input of either kind, with whether each is available for this Source and its origin.
- `list_slice_functions` offers a function only when everything it needs is available. A function that is held back can be enabled by supplying the missing items in a companion file.

**Companion files.** A companion file holds metadata that a file does not, beside the file it describes.

- **Named for its file.** It is the file's name with `.metadata` added. A caller may also name one in `OpenOptions`.
- **Plain text.** One `name = value` item to a line, using the field names of this section, with values in the dictionary's units.
- **A fallback.** A companion value is used only where the file neither provides the item nor allows it to be derived.
- **Written by the user or by BIL.** `write_companion(fields)` writes one for a Source without changing the Source. BIL's writers also use a companion file for what the output format cannot hold, including markings (section 11).
- **Read back.** When BIL opens a file that has a companion file, its items are attached to the Source with the origin `Companion`.
- **Not trusted blindly.** For a Source opened as untrusted, a companion file's values are used only when `trust_companion` is set, since a companion beside an outside file comes from outside too. Markings in it are always reported, whether or not its values are used.

### 14.3 Extracted on demand, then held

1. **Opening extracts little.** Opening a Source extracts only what describes its Images: size, pixel type, component names and frame count.
2. **Fields come in groups.** A group is one unit of the format: a NITF file header, one tagged record extension, an Exif block, a DICOM data set, or the KLV of one frame.
3. **A group is extracted when first asked for.** Asking for any field extracts its whole group.
4. **Values are decoded when needed.** Extracting a group reads its bytes. Each value is decoded from text, codes or packed bits into its typed value only when it is first read, or when a function needs it: a normalization, a geocoding step, a slice function's parameters, a derivation, or a writer carrying markings.
5. **Extracted groups are held.** They stay with the Source, so later questions are answered from memory. They are freed when the Source is closed or the caller releases them.
6. **Values that vary by index are extracted for the indexes asked for.** Opening a video with 100,000 frames does not parse 100,000 KLV packets.
    - **Held up to a limit.** Fields attached to an index are held for at most `max_held_positions` positions, set in `OpenOptions`, with a default of 1,000. The position used longest ago is released first, and extracted again if asked for. A live stream therefore stays within bounded memory.
    - **Values at their own times.** Some fields arrive at times of their own, as KLV packets do, not aligned to frames and often less often. The value at a frame is the latest at or before the frame's time, returned with its own time.
7. **Markings are always carried.** A definition can flag a field as a `Marking`, such as a security classification, NITF's security fields among them, or as `Identity`, such as a patient's name or NITF's originator name and telephone (ONAME, OPHONE). NITF's titles and comments (FTITLE, IID2, ICOM), which can hold anything, stay `Ordinary`. A writer extracts both kinds whether or not the caller has asked for them, writes the markings and leaves the identity fields empty (section 11).

This is the design for now. BIL does not yet index metadata across many Sources, or keep it after the Source is closed.

### 14.4 Calls

The calls exist on a Source and on each of its Images.

| Call | Result |
| --- | --- |
| `list_field_groups()` | The names of the groups present. Extracts nothing. |
| `list_fields(group_name)` | The names of the fields in a group |
| `read_field(field_name)` | The value, with its type, unit and origin. An unknown name is an error; a known field with no value is delivered as `Missing` (section 3.3). |
| `read_field_at(field_name, index)` | The value at one position along the index the field attaches to, such as one frame. For a field with its own times, the latest value at or before that position, with its time. |
| `describe_field(field_name)` | The field's definition, without its value. For a dictionary item, also its origin, and the native field or the formula and inputs behind it. |
| `list_required_fields(function_name)` | The dictionary items a function needs, with whether each is available and its origin |
| `check_profile(profile_name, index)` | A `ProfileReport` for this Source or Image: each item with its status and origin. `index` is needed only for items attached to an index. |
| `write_companion(fields)` | Writes a companion file for this Source, without changing the Source |
| `list_markings()` | The fields flagged as markings. Also available on a Fragment and a Slice, for the Source they came from. |
| `find_fields(pattern, extract)` | The names that match a pattern, among the defined names and the groups already extracted. With `extract` true, every group is extracted first. |
| `add_field(field_name, value)` | Adds a caller's field. It is held with the Source and never changes the file. |
| `add_field_definitions(definitions)` | Adds names, value types and units for coded fields, and any dictionary item each provides |
| `load_field_definitions(location)` | Reads field definitions from a file in the form below |
| `release_metadata(group_name)` | Frees a held group. It is extracted again if asked for. |

Profiles belong to no one Source, so their other calls are BIL's own, beside `list_drivers`.

| Call | Result |
| --- | --- |
| `list_profiles()` | The names of the domain profiles BIL knows, its own and those added |
| `describe_profile(profile_name)` | A profile's description, and its required and recommended items and profiles |
| `add_profile(profile)` | Adds a profile in the form of section 14.2, until the process ends |
| `load_profiles(location)` | Reads profiles from a file in that form |

```python
source = bil.open_source("scene.ntf")
image  = source.image(0)

image.list_field_groups()                         # ["nitf.image_subheader", "nitf.RPC00B", ...]
title = source.read_field("nitf.file_header.FTITLE")
title.value, title.origin                         # the title text, Origin.Specified

source.read_field("common.title").value           # the same text, by its dictionary name

source.add_field("caller.task_id", "T-0417")

report = image.check_profile("sar")
report.status                                     # Status.Partial
[i.name for i in report.items if i.status != bil.Status.Complete]   # ["common.look_side"]
```

### 14.5 How metadata relates to the rest of BIL

| Part of BIL | Relationship to fields |
| --- | --- |
| Geocoding (section 13) | The driver builds the geo models from fields. The fields themselves stay readable here. |
| Normalization (section 7) | The driver maps fields to normalization parameters. The fields stay readable here. |
| Coordinates (section 3.1) | A coordinate is a field attached to an index. `read_coordinate` returns such a field along its whole index, and `read_field_at` returns it at one position. Both work on any field attached to an index. `find_index` needs one whose values are numbers. |
| Additional data (section 10.1) | Tables and arrays too large to be field values are read with `read_auxiliary`. |
| Writing (section 11) | `write_metadata` takes fields, and writes those the format can hold. Markings are always written, to the companion file if the format has no place for them. Identity fields are left empty unless the caller keeps them. |

## 15. Outside the model

Anything that needs a wide or data-dependent neighborhood, more than one frame, or data that is not a 2D array is outside Fragment and Slice.

| Category | Examples | Where it belongs |
| --- | --- | --- |
| Other neighbor-dependent operations | Pan-sharpening, speckle filtering, slope and hillshade | The image processing code BIL feeds |
| Multi-frame operations | Frame differencing, background estimation, stabilization | The image processing code BIL feeds |
| Registration and other higher-level processing | Registration, segmentation, orthorectification | A layer above BIL; not considered in this specification |
| Image formation | Reconstructing an image from MRI k-space, CT projections, SAR phase history, interferograms or coded-aperture measurements | A layer above BIL. BIL reads the measurements themselves as arrays. |
| Whole-image statistics | Computing the statistics for a display stretch | Above BIL. Applying a stretch with statistics passed in is a slice function of the kind Present (section 9). |
| Non-raster data | Event cameras, photon-counting and list-mode PET event lists, moving-target radar reports, localization microscopy, unorganized point clouds, vector overlays | Not covered by BIL until binned into a 2D array |

Four things inside BIL do read neighboring pixels: demosaicing, chroma upsampling, the interpolation of mosaic sites, polarizer angles and video fields, and filtered downsampling. Each uses a small fixed neighborhood tied to the Image grid, so results stay window-independent. None changes the size of a Slice: at a given Level, every Slice of an Image has the same size.

## 16. Open decisions

Two points are not yet settled, and ninety-three have been decided. Each lists the current proposal.

- [x] **Naming.** Decided: types and enum values are UpperCamelCase, as Rust and Python require, and every other name is snake\_case. That includes the names passed as text: slice functions, Slice names, level names, geocoding steps and normalization functions. An earlier decision to use UpperCamelCase everywhere was reversed so that Rust developers would accept the library (section 2).
- [x] **Spelling.** Decided: American spelling in every identifier and throughout this document.
- [x] **Full-resolution demosaicing.** Decided: BIL provides it at full resolution through the `cfa.demosaic` slice function.
- [x] **Deferred extraction for video.** Decided: reading a Fragment only seeks and extracts compressed data, and the video is decoded when the Slice is formed (section 4).
- [x] **Indexes and coordinates.** Decided: data is addressed by index and described by coordinate. Frame is an index, and time is a coordinate linked to it (section 3.1).
- [x] **External decoding.** Decided: an optional, built-in layer in which a separate process decodes into BIL's memory-mapped format and a mapped driver reads it (section 10.2).
- [x] **Fragment cache.** Decided: decoded blocks and video frames are kept in a cache for each Source, sized by the number of Fragments kept. Decoder state is held in a Reader for each thread (section 4).
- [x] **Access patterns.** Decided: the caller declares `Random` or `Sequential`, for a Source or for one Reader. Under `Sequential`, BIL reads and decodes ahead in the storage order each driver reports, merges adjacent reads and drops passed blocks first. The pattern never changes a result (section 4).
- [x] **Access pattern details.** Decided: `Random` and `Sequential`, which do not repeat their type's name. `Random` is the default, since reading ahead wrongly costs more than not reading ahead. `read_ahead_bytes` is 64 MB. Read-ahead runs forward only, in storage order, and in band-sequential storage follows each component the last read used. Blocks read ahead count against `set_cache_size` as well as the byte ceiling. There is no automatic pattern. The default sizes are revisited when benchmarks on real files are run (section 4).
- [x] **Video chroma.** Decided: luma and reduced chroma are two related Images. Chroma upsampling is part of the local slice functions `video.planes` and `video.rgb`, which read both, because every Slice at one Level shares one grid (sections 3, 5 and 9.3).
- [x] **One resolution for every Slice.** Decided: the Level sets the grid. At a given Level every Slice of an Image has the same size, whichever slice function made it. Data stored at lower resolution, such as video chroma, mosaic sites, polarizer angles and video fields, is interpolated onto that grid (section 5).
- [x] **Frame count and index.** Decided: a frame count is `Known`, `AtLeast` or `Unknown`, and the frame index is built on first need, never when a Source is opened (section 3.2).
- [x] **Mapped format details.** Decided: BIL's own layout, a JSON header with a block table and page-aligned blocks, used as a bounded set of slots on request. The decoder process is spawned on first use, or attached to where one is running. Readiness comes by a reply on the control channel, or by a `complete` flag for a whole copy. Readers mark the slots they hold, and shared memory is passed without a name (section 10.2).
- [x] **Test data.** Decided: two layers. Synthetic reference files with known contents are generated by BIL's own test suite. Real files are listed in `testdata/manifest.toml` with their source, purpose and checksum, and fetched by `testdata/fetch.py`. The first come from GDAL's download cache, which mirrors the NITF Test Bed files; the author supplies others. CLIF 2007 remains a candidate for wide area motion imagery (Appendix B).
- [x] **Interface details from the user manual.** Decided: `ImageInfo` states `may_have_missing`; a Source closes when the last reference goes, or at `close()`, after which borrowed arrays keep only their memory; `level_count` counts halvings to one pixel along the shorter side; Python builds requests with keywords, indexes results by Slice and level name, passes times as `datetime` and gives positions `lat`, `lon`, `height`, `status` and `accuracy`; a live stream ignores the access pattern; `common.units` and `common.sample_spacing` are defined against a Slice's unit and `common.ground_sample_distance` (sections 3, 3.1, 3.2, 4, 5, 13.4 and 14.2).
- [x] **The configuration.** Decided: one file in a stated place, which an environment variable can point elsewhere, read by a call. Its form and names are settled when Tier 2 starts (section 10.3).
- [x] **Missing data and coarse Levels, from experiment 1b.** Decided: a Level averages the present values over the whole 2D filter at once; a stored NaN at a valid pixel is missing; `pixel.mask.valid` follows the mask averaged by the level method, beside any kind of Slice, and `valid_fraction` is that average; labels at coarse Levels alone read only their grid points under any method (section 5).
- [x] **Borrowed data and the Python interface, from experiment 5.** Decided: under `Private` mapping, arrays from one Source share their pages, and concurrent writes are the caller's data race; borrowed arrays are read-only under `Shared` mapping only; the Python interface is built on PyO3 (sections 2, 4 and 12).
- [x] **Libraries' corrupt data and limits, from experiment 3.** Decided: data a library recovers with warnings is corrupt data, an error by default, and its whole block is missing under `corrupt_as_missing`; `max_scans` is a fourth limit, 500 for untrusted Sources; a library's working memory is capped at twice `max_decoded_bytes` where it allows; a refusal is an error; a codec does not expand JPEG 2000 palettes, and the driver applies or reports `cmap` and `cdef` (sections 3.3, 3.4 and 10.3).
- [x] **Stored NaN and closing in C.** Decided: a stored NaN in float data is always missing, mask or not, and float drivers set `may_have_missing` unless the format rules NaN out; in C, `bil_close_source` frees the Source's handle, which must not be used again, while Fragments and Slices already read stay valid until each is freed, and in Rust dropping the Source closes it (sections 4 and 5).
- [x] **Codes in locators.** Decided: a locator's code matches the stored code with its padding removed, up to a length each driver states for its formats, and a definition that breaks it is refused when added; a code may hold a wildcard, `*`, for a family of codes, and a field's name keeps the code matched (section 14.1).
- [x] **Ordered layouts and runs.** Decided: a locator may give a field's place in an ordered layout, with lengths, presence and counts taken from earlier fields; a field in a counted run always carries its place, one number for each run, outermost first (section 14.1).
- [x] **Wildcards.** Decided: `*` stands for any run of characters, including none, and a code needs at least one other character; an exact code is preferred, then the most characters fixed, and a tie is refused when added; no literal `*` (section 14.1).
- [x] **Working memory where a library has no cap.** Decided: the driver refuses a block whose declared size over every component exceeds twice `max_decoded_bytes`, before reading it (section 10.3).
- [x] **Registering a shared library's drivers.** Decided: one at a time, only those BIL uses, each Source opened with only its chosen driver allowed; never all at once (section 10.5).
- [x] **NITF pad pixels, identity fields and codec names.** Decided: a pixel is missing when every sample holds the pad value, anywhere in the Image; ONAME and OPHONE are `Identity`, NITF's titles and comments `Ordinary`; codecs are named by what they decode, with NITF's codes mapped (sections 4, 10 and 14.3).
- [x] **What stops an open.** Decided: damage to the structure that locates the Images fails the open; damage within an extension area leaves its header's group `Partial` and the pixels readable (section 3.3).
- [x] **OpenJPEG's source and threads.** Decided: BIL's own bindings link the shared copy wherever one is found, and a private copy through openjpeg-sys otherwise, its symbols not exported; OpenJPEG stays on the calling thread, and the residue of `OPJ_NUM_THREADS` is accepted (section 10, 10.5).
- [x] **Compressed NITF Images.** Decided: integers aligned to the right, typed from NBPP, `significant_bits` from ABPP; a JPEG 2000 Image is one codestream; an Image no codec accepts is listed and its reads fail (sections 4 and 10).
- [x] **TIFF's fields and no-data value.** Decided: groups `tiff.ifd` and `geotiff.keys`, unknown numbers in decimal, text as Latin-1; `GDAL_NODATA` by the every-sample rule, rounded to float16 and float32 (sections 4 and 14.1).
- [x] **TIFF samples and overviews.** Decided: the `tiff` crate reads the directories, BIL the chunks, so samples are as stored and reads run in parallel; other widths widen; overviews and SubIFDs are stored levels of unknown alignment, covering their Image's extent, with its bands and, lacking their own, its no-data value (sections 4, 5 and 10).
- [x] **TIFF compression, limits and damage.** Decided: codec names for TIFF's codes; LZW and PackBits join Deflate in Tier 1; `max_directories` and `max_subdirectories`, and refusals by a limit reported as unsupported; damage to the IFD chain or chunk tables fails the open (sections 1, 3.3, 10 and 10.3).
- [x] **One error type.** Decided: every driver reports through one error type, and an unrecognized file names the formats tried (section 3.3).
- [ ] **Drivers for the added sensor cases.** The medical, security screening and sonar cases in section 8 name the data but not yet the libraries that read it.
- [x] **Metadata model.** Decided, for now: metadata is held as named fields, with names taken from self-describing files or from specifications. Fields are extracted on demand and held to be read when needed (section 14).
- [x] **Markings and identity.** Decided: markings are always written, to the companion file where the format cannot hold them. Identity fields are left empty by default. The caller can override both (section 11).
- [x] **Metadata dictionary.** Decided: BIL keeps a dictionary of essential metadata, so that formats offer the same items as far as they can. Each item is provided by the format, derived by a stated formula, or taken from a companion file, in that order (section 14.2).
- [x] **Lazy metadata decoding.** Decided: each value is decoded only when it is read or when a function needs it (section 14.3).
- [x] **Companion files.** Decided: a plain text file beside a Source supplies metadata the format lacks, and carries markings for outputs that cannot hold them. It replaces the separate markings side file (sections 11 and 14.2).
- [x] **Borrowed data.** Decided: zero-copy also needs aligned samples. Borrowed data is read-only under `Shared` mapping, and mapping is `Shared`, `Private` or `Off` (section 4).
- [x] **Threads and plugins.** Decided: a Source and an Image may be shared between threads, and a Reader belongs to one thread at a time. Each driver declares its concurrency as `Parallel`, `OneHandlePerThread` or `Serial`. `bil_register_driver` carries an interface version (sections 4 and 10).
- [x] **Codec choice and probing.** Decided: the driver and codec are chosen when a Source is opened and never switched. Both are reported and recorded in what BIL writes. Probing looks at the container, then its content, and ties follow a configured order (section 10).
- [x] **Field names and time.** Decided: a field name is a list of parts, with escaped dots and numbered repeats. Text is Unicode, and a date and time states whether its zone is known. Local times take their zone from a stated offset, then, where an optional time-zone lookup is installed, the sensor location or the footprint. Otherwise they are taken as local time (section 14).
- [x] **Performance claims.** Decided: specialized loops cover the six common sample types and the rest convert through a float32 row buffer. Copy counts exclude the decoder's own buffer. Benchmarks run on dedicated machines with thresholds, outside the build. Each function aims to be as fast as its arithmetic allows, measured against a copy and against that arithmetic alone (sections 6 and 12).
- [x] **Color description.** Decided: color is described by the four H.273 properties, from metadata or a stated default, and computed by formula in BIL's own loops. Color profiles are exposed as fields and not applied; color management is a layer above BIL (section 9.3).
- [x] **Values for `color.hsv`.** Decided: hue, saturation and value are computed from the encoded values as stored, as most software does (section 9.3).
- [x] **What the catalog holds.** Decided: a slice function belongs in BIL when it undoes the storage, calibrates, or re-expresses the measurement in a standard form that analysis starts from. Analysis goes to the `bil-extras` package. BIL is not for diagnostic use (sections 1.2 and 9).
- [x] **Consistency, second review.** Decided: one rule for repeated names; one name and one meaning for each quantity; decibels beside their linear Slices; color converted only by the color functions; every input of a function a field; coordinates are dictionary items where the same; access classes describe the data only; Levels named in requests and geocoding built from the Slice; unknown names are errors and absent values `Missing`; one registration call; every option and held limit in section 3.4; one list of attachments; a display stretch as a kind of slice function; and packaging rules that match the FFmpeg decision (U1 to U16 of the second review).
- [x] **The calling interface.** Decided: the gaps found by BIL by Example are closed. A caller lists Images and reads an `ImageInfo`; reads Fragments by Level name; passes selectors with parameters to `read_slices`; sends results to buffers or a callback; reads through a chosen Reader; finds the newest frame, with a timeout and cancellation; names geo models by their main step and makes steps; leaves out a surface where none is needed; registers slice functions and level methods by a stated description; writes field definitions in a stated form; writes into named components and writes geo models; and calls C by five conventions (sections 2, 3, 3.2, 3.4, 4, 5, 9.7, 10, 11, 13.3, 13.4, 14.1 and 14.2).
- [x] **Red team, second review.** Decided: isolation decodes on request, and a whole copy is a separate choice; Window independence and agreement between machines hold to a stated tolerance, not to the bit; markings are written inside files where they fit, a missing companion is noticed, and refusal is available but off by default; writers write the geo model, hold stated sample types and write Slices on their own grid; deferred Fragments decode themselves; the cache has a byte ceiling and untrusted Sources get limits; remote Sources run on a private runtime with counted, merged requests and timeouts; a live stream has one position and past frames are `Missing`; a static-linking exception; arithmetic never in integers; masks reduced as masks; checked frame indexes and companions not trusted blindly; a stability policy; more conformance tests; and Exif GPS time used first (R26 to R41 except R29).
- [x] **Stored levels and the Level grid.** Decided: a stored level is used to make a Level only when it is aligned with the Level grid. A misaligned one can be read as it stands, with its true origin and scale, but never makes a Level. A stored level's position is its first pixel's center, a Fragment from it is in its own rows and columns, and JPEG 2000 resolution levels are aligned (sections 4 and 5).
- [x] **The Source interface in Rust.** Decided: `OpenOptions` is a struct opened like `std::fs::OpenOptions`, an unrecognized file names the drivers tried, `describe_storage` takes the level, and a container with one driver is chosen by its leading bytes alone (sections 3.3, 3.4, 4 and 10).
- [x] **Representations from NITF and TIFF.** Decided: representations and component names follow IREP, IREPBAND and Photometric by one table, which covers every band count and ExtraSamples; `pixel_type` may be none beside `component_count`; `storage` is a `Storage` without blocks; a relation's reduction is per axis (section 3).
- [x] **The Fragment cache's entries.** Decided: one entry for each block `describe_storage` lists, JPEG 2000 tiles among them; every block not borrowed is cached; the ceiling in bytes gives way for one block a read decodes, with a warning; a block several Readers need is decoded once; `describe_cache` counts blocks (section 4).
- [x] **Grid alignment.** Decided: Level n holds the grid points at multiples of 2ⁿ inside the Window. Every Slice carries its origin and scale. `step` and a Level are not combined. Range is reduced over a centered 3×3 block, so it shares the grid (section 5).
- [ ] **Dictionary contents.** Which items the metadata dictionary holds, the formula for each derived one, and the items of each domain profile. The first entries and first profiles are in section 14.2, and both are settled as drivers are built.
- [x] **Domain profiles.** Decided: the dictionary knows the metadata each problem domain needs, independent of sensor and format, as named profiles of dictionary items, each required or recommended. Profiles name one another in those lists, and a profile that names itself, directly or through others, is refused. They are checked against a Source by `check_profile`, never enforced, and can be added by layers above BIL (section 14.2).
- [x] **Sensor geometry.** Decided: geocoding is a chain of steps between named coordinate spaces, covering affine, RPC, SAR and camera models, with Community Sensor Model plugins as one kind of step (section 13).
- [x] **Geocoding validation.** Decided: BIL implements rigorous sensor models itself and tests each step against an established implementation, in both directions (section 13.8).
- [x] **Geocoding without a clean answer.** Decided: a line of sight stops at the first surface it meets, every position carries a status, and longitude wraps (section 13.3).
- [x] **Mixed sample types.** Decided: a Fragment keeps one sample type. Mixed data is held as separate Images that share a grid, and a slice function may read from several of them (sections 3 and 5).
- [x] **Slice selection.** Decided: a Slice is always named in three parts, with no aliases and no short forms. Parameters are passed in a `SliceSelector`, never inside the name. Names taken from metadata follow one stated rule (section 5).
- [x] **Uniform design.** Decided: each thing is done one way for every format and sensor, and a difference between sources is a reported property, never a special case (section 1.6).
- [x] **One read request.** Decided: every read is one request for any Slices, at any Levels, delivered any way. The other read calls are short forms of it (section 5).
- [x] **Results and status.** Decided: a call delivers or returns an error, and everything delivered carries one status. Corrupt data is an error by default (section 3.3).
- [x] **Reads across frames.** Decided: outside BIL. Everything beyond 2D is selected, never ranged, so a caller reads frame by frame (section 1.6).
- [x] **GIPS geocoding model.** Decided: BIL adopts the model, as the `sphere_rotation` step, and adds `log10` and `value_polynomial` to the normalization library. A reader for GIPS files is not planned (sections 7 and 13.7).
- [x] **Geocoding accuracy.** Decided: every error estimate a format, terrain or caller supplies is kept as a full covariance, split by source and by kind (bias, random, correlated), with its provenance, and carried through every step of the chain by partial derivatives, checked by sampling. Unknown contributions are named, never taken as zero. Summary measures such as CE90 are computed exactly from the covariance, relative accuracy is available, and the estimates are written with the geo model (section 13.9).
- [x] **Default terrain.** Decided: none is fetched. With no surface given, a constant height is used, and remote terrain is read only when the caller opens it (section 13.6).
- [x] **Containment.** Decided: BIL reaches the network only for a location the caller gave it, and writes data only where the caller named a place. Mapped data is private to its owner and removed after use (sections 1.3 and 10.2).
- [x] **Untrusted files.** Decided: caller-set resource limits, plugins loaded only from configured locations, and fuzz testing of BIL's own readers. Isolation of C and C++ drivers in a separate process is offered as a setting and is off by default (section 10.3).
- [x] **Build order.** Decided: BIL is built in tiers, and Tier 1 is GeoTIFF and NITF. Every driver must pass one conformance suite before its tier is finished (sections 1.4 and 10.4).
- [x] **Tier 2.** Decided: ENVI, video with KLV, and SICD and SIDD, with Readers, scripted drivers and the C interface for external plugins (section 1.4).
- [x] **Sample types.** Decided: a Fragment can hold fifteen sample types and delivers samples as stored, recording any unused low bits. A Slice is always float32 and works from the sample value (sections 4 and 5).
- [x] **Packaging.** Decided: BIL works on installation and does not duplicate what standard package management supplies. Each library comes from an existing crate or from the installing package manager where that loses nothing, and BIL carries a private copy only as a last resort (section 10.5).
- [x] **Pixel positions.** Decided: whole-number positions refer to pixel centers (section 13.3).
- [x] **Terrain access.** Decided: BIL reads external terrain through its own `Terrain` interface, using its own drivers for the pixels and PROJ for the geoid (section 13.6).
- [x] **Backing libraries and licenses.** Decided: Tier 1 reads GeoTIFF and NITF with Rust readers, and keeps GDAL and NITRO as test references, so Tier 1 needs no LGPL 3.0 library and no C++ build. Each library of a later tier is assessed when its tier starts, for license, maintenance, availability from package managers and conformance (section 10). Every library in Appendix A stays in the plugin design until then.
- [x] **Python-authored drivers.** Decided: allowed, as the scripted drivers of section 10.1.
- [x] **Scripted drivers outside Python.** Decided: Rust, C and C++ callers can use scripted drivers, through CPython in a helper process (section 10.1).
- [x] **Interpreter packaging.** Decided: the interpreter is CPython, taken from the Python installed on the machine, and BIL's packages contain none. Inside Python, BIL uses that interpreter. Without an installed Python, only scripted drivers are unavailable (section 10.1).
- [x] **Interpreter process.** Decided: for Rust, C and C++ callers, CPython runs in a helper process. The cost of the round trip is benchmarked and watched, and the choice is reviewed if it becomes prohibitive (section 10.1).
- [x] **NaN for missing data.** Decided: a Slice value with no data behind it is NaN by default. `missing_value` in `SliceOptions` fills such positions with a number the caller chooses. `missing_weights` chooses whether a Level weights the values below it equally, the default, or by the present full-resolution pixels behind each (section 5).
- [x] **Registry names.** Decided: hold the names with genuine first releases rather than empty placeholders. `bil` 0.0.1 on crates.io holds the vector layer and kernels from experiment 1, `bil-sys` 0.0.1 holds the name for native bindings, and `bil` 0.0.1 on PyPI holds the Python name. The project's owner publishes them through the repository's Publish workflow. If `bil` is taken on PyPI, the package is `blazing-imaging` and still imports as `bil`.
- [x] **Default demosaicing method.** Decided: `bilinear`, the same method used for the resolution levels, so both come from a single pass. `malvar_he_cutler` remains available.
- [x] **Downsampling method.** Decided: bilinear interpolation, with weights 1 2 1 at each halving, built into the interface as resolution levels. Its aliasing is recorded, and `binomial5`, with weights 1 4 6 4 1, is the first added method. Other named methods can be added as extensions (section 5).
- [x] **Reduced resolution at lower cost.** Decided: a computed Level reads all the data under its Window. `decimate` is built in as a level method that strides, and a file's own reduced-resolution data is read as named stored levels, used only when the caller asks (section 5).
- [x] **JPEG 2000 codecs.** Decided: OpenJPEG is the fallback, and decodes every file until a Rust codec is proven. j2k-native is on trial as the Rust fallback. emuella-j2k is preferred for its declared profiles once it passes BIL's tests. hayro-jpeg2000 is a test reference. Grok stays an optional plugin (section 10).
- [x] **LGPL version.** Decided: BIL is licensed under LGPL 2.1 or later.
- [x] **License and responsibility.** Decided: BIL is a Rust library with a C-callable interface over it, delivered as one shared library. BIL answers for what its own packages contain, which is only LGPL-compatible builds. Users answer for what they install and pass on. `list_drivers` reports each library's license as built (section 10.6).
- [x] **License review.** Closed by the author, with no legal review planned. The static-linking exception stands as worded in `LINKING-EXCEPTION.md`. What ships stays compatible with LGPL 2.1, with Apache-2.0 accepted, and LGPL 3 code is used only in tests or as an optional plugin packaged separately. Licenses are checked in the build by cargo-deny (section 10.6).
- [x] **Writable formats.** Decided: ENVI, TIFF and GeoTIFF (uncompressed or Deflate), and NumPy `.npy`, each through BIL's own writer (section 11).
- [x] **Wrapper crates.** Decided: an existing wrapper is considered first for every C or C++ library, and used when it passes the four tests of maintenance, performance, capability and packaging. Otherwise an existing bindings crate is used where one passes, and BIL's own bindings only where none does (section 10).
- [x] **Reaching FFmpeg.** Decided: through the existing rusty\_ffmpeg bindings, with a thin safe layer of BIL's own. BIL does not generate bindings or build FFmpeg (section 10).
- [x] **Where FFmpeg comes from.** Decided: FFmpeg 6 or later, always the shared copy from conda or the system package manager. BIL never carries it, and for FFmpeg the one-step test means one step through such a package manager. The rule applies to FFmpeg only (section 10.5).

## Appendix A. Candidate libraries

Every library in this appendix is part of the plugin design for now, to be assessed case by case later. A.13 carries a recommendation for each of its 40 libraries. Those in A.1 to A.13 were checked against their own project pages on 10 October 2026, and license and status are as those pages state them. Two things recur: several of the strongest libraries are LGPL or AGPL, and most Rust candidates are below version 1.0.

### A.1 JPEG 2000

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [emuella-j2k](https://github.com/emuella/emuella-j2k) | Rust | Apache-2.0, with BSD-2-Clause parts derived from OpenJPH | Preferred once proven by BIL's tests. Decodes selected Part 1 and HTJ2K profiles, with region and reduced-resolution output for some of them, and claims no general conformance. Version 0.1.0 is unreleased, it has one owner, and its C interface is experimental. |
| [Grok](https://github.com/GrokImageCompression/grok) | C++ | AGPL-3.0 | Optional plugin only: BIL is LGPL and Grok is AGPL, so installing it makes that installation AGPL. Faster than Kakadu and OpenJPEG in its own benchmarks. Random-access region decoding, 1 to 16-bit data and cloud reads. |
| [OpenJPH](https://github.com/aous72/OpenJPH) | C++ | BSD-2-Clause | An HTJ2K (Part 15) codec. Not a general Part 1 decoder. |
| [j2k-native](https://docs.rs/j2k-native) | Rust | MIT or Apache-2.0 | On trial as the Rust fallback. Part 1 and HTJ2K, from codestreams, JP2 and JPH, with a native path for 8, 12 and 16-bit data. Version 0.12.0, 6 October 2026, with one owner. Its page claims no conformance or region decoding, so BIL's tests decide. |
| [hayro-jpeg2000](https://docs.rs/hayro-jpeg2000/latest/hayro_jpeg2000/) | Rust | Apache-2.0 or MIT | Test reference only. A Part 1 decoder that forbids unsafe code, tested on over 20,000 images. Version 0.4.1. Built for PDF rendering: 8-bit output, no HTJ2K, no region decoding, and slower than OpenJPEG by its author's account. |

### A.2 NITF and SAR

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [SIX](https://github.com/ngageoint/six-library) | C++ | LGPL-3.0 | To be assessed when Tier 2 starts: as LGPL 3, an optional plugin or test reference only (section 10.6). Reads and writes SICD, SIDD and CPHD, and is the reference implementation for SIDD. |
| [nitf-rs](https://github.com/holmesv3/nitf-rs) with [sicd-rs](https://github.com/holmesv3/sicd-rs) | Rust | MIT (nitf-rs) | A minimal NITF reader and a SICD crate in Rust. A small project. |
| [SARkit](https://pypi.org/project/sarkit/) | Python | MIT | Reads and writes SICD, SIDD, CPHD and CRSD and checks files for inconsistencies. Version 1.10.1, July 2026. Useful as a reference and test oracle, and usable from a scripted driver. |

### A.3 TIFF and GeoTIFF

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [async-tiff](https://github.com/developmentseed/async-tiff) | Rust | Not checked | Cloud-optimized GeoTIFF read by byte range from object stores, decoded on a thread pool and handed to NumPy without copying. [Described here](https://developmentseed.org/async-geotiff/latest/blog/2026/02/03/introducing-async-geotiff/). |
| [oxigdal-geotiff](https://docs.rs/oxigdal-geotiff) | Rust | Apache-2.0 | The GeoTIFF and COG driver of OxiGDAL, now named OxiGeo; see A.11. Version 0.1.7, September 2026. |

### A.4 Raw camera files

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [RawSpeed](https://github.com/darktable-org/rawspeed) | C++ | LGPL-2.1 | Built for the fastest possible decode. Returns unmodified raw data with the color filter layout and black levels, and does no demosaicing, which matches a BIL Fragment exactly. Reads little metadata. |
| [rawler](https://lib.rs/crates/rawler) | Rust | LGPL-2.1 | Raw pixels plus metadata, built on rawloader. Version 0.7.1, October 2025. |

### A.5 Video and motion imagery metadata

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [ts-transformer](https://github.com/aklofas/ts-transformer) | Rust, with C, Python and JVM bindings | MIT or Apache-2.0 | MPEG-TS demultiplexing with typed MISB KLV, including ST 0601, 0102 and 0903, over UDP, RTP, SRT and others. It has no video decoder. Pre-1.0. |
| [libmisbklv](https://github.com/nitsuga/libmisbklv) | C++20 | Apache-2.0 | Reads and writes MISB ST 0601 and ST 0903. The core has no dependencies. A new project with no releases. |
| [klv-uas](https://lib.rs/crates/klv-uas) | Rust | MIT | ST 0601 parsing from transport stream payloads. Version 0.1.1 from 2024, with some data types unfinished. |
| [rav1d](https://github.com/memorysafety/rav1d) | Rust | BSD-2-Clause | An AV1 decoder ported from dav1d, with the same C interface. |
| ffmpeg-next, rsmpeg, gstreamer-rs | Rust bindings | Not checked | Ways to reach FFmpeg and GStreamer from Rust. A [2026 survey](https://dev.to/yeauty/the-state-of-media-processing-in-rust-2026-what-each-crate-actually-covers-2k2c) reports ffmpeg-next as the most used but in maintenance mode, rsmpeg as tracking FFmpeg 6 to 8, and gstreamer-rs as actively maintained. BIL uses rusty\_ffmpeg, the bindings under rsmpeg, with a thin safe layer of its own (section 10). |

### A.6 GIMI and HEIF

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [libheif](https://github.com/strukturag/libheif) | C++ with a C interface | LGPL-3.0, so an optional plugin only (section 10.6) | Reads GIMI (GEOINT Imagery Media for ISR), a HEIF-based imagery format. The [OGC Testbed-20 report](https://docs.ogc.org/per/24-042r1.html) records support for uncompressed ISO/IEC 23001-17 data, tiling, pyramids, image sequences, integers to 64 bits, floats and complex numbers. |

### A.7 Scientific arrays

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [zarrs](https://github.com/zarrs/zarrs) | Rust | Apache-2.0 or MIT | Zarr version 3 arrays, the usual layout for cloud-hosted multidimensional data. |
| [hidefix](https://docs.rs/hidefix/latest/hidefix/) | Rust | LGPL-3.0-or-later, so an optional plugin or test reference only (section 10.6) | Concurrent HDF5 and NetCDF4 reading through a prebuilt chunk index. Experimental, and used alongside the HDF5 C library. Version 0.12.0, September 2026. |

### A.8 Live cameras

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [Aravis](https://github.com/AravisProject/aravis) | C | LGPL-2.1-or-later | Acquisition from GenICam cameras over Gigabit Ethernet and USB3. |
| [cameleon](https://github.com/tonarino/cameleon) | Rust | MPL-2.0 | GenICam cameras over USB3 Vision. Gigabit Ethernet support is planned, not present. |

### A.9 Still-image codecs

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [libjpeg-turbo](https://github.com/libjpeg-turbo/libjpeg-turbo) | C | BSD-style | A JPEG codec accelerated with vector instructions, for JPEG blocks inside NITF, TIFF and raw files. |
| [jxl-oxide](https://github.com/tirr-c/jxl-oxide) | Rust | Apache-2.0 or MIT | A spec-conforming JPEG XL decoder. Version 0.12.6, May 2026. |

### A.10 Sensor geometry

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [usgscsm](https://github.com/DOI-USGS/usgscsm) | C++ | Not checked | Community Sensor Model plugins for frame, line-scanner and SAR sensors. Relevant to the open sensor geometry decision. |

### A.11 OxiGeo, formerly OxiGDAL

[OxiGeo](https://github.com/cool-japan/oxigeo) is a pure Rust reimplementation of GDAL under Apache-2.0, at version 0.2.4 (August 2026). Its drivers are separate crates, so BIL can take them one at a time.

| Part | What it could give BIL | What to watch |
| --- | --- | --- |
| GeoTIFF and COG driver | BigTIFF, overviews, HTTP range reads, and DEFLATE, LZW, ZSTD and JPEG compression, with no C library | The most useful part: the common GeoTIFF path without GDAL |
| Zarr driver | Versions 2 and 3, with sharding | Overlaps zarrs |
| NetCDF and HDF5 drivers | Reading without the C libraries | HDF5 is complete only for the oldest file layout. Newer files may open as an empty tree, which rules it out for most current products |
| JPEG 2000 driver | A read-only decoder | Tier-2 decoding was added only recently. Not a rival to emuella-j2k |
| Cloud storage | S3, Google Cloud Storage and Azure Blob with range requests | A storage backend any BIL driver could use |
| Projections | A Rust subset of PROJ: more than 20 projections and 211 EPSG codes | Far fewer than PROJ itself |
| Not covered | NITF, SAR, ENVI, raw camera files and video | BIL's other plugins remain necessary |

**Maturity.** The project describes itself as production-ready. Its first release was in February 2026, it reports about 814,000 lines of Rust, and its own size figures disagree with each other. None of its claims were tested in this survey. Each OxiGeo driver should be accepted only after it passes BIL's own test files.

**Upstream only.** BIL tracks upstream OxiGeo. The fork at boxerab/oxigdal is not used, because it is three months and one minor version behind.

### A.12 Medical imaging

| Library | Language | License | What it adds, and what to watch |
| --- | --- | --- | --- |
| [ITK](https://github.com/InsightSoftwareConsortium/ITK) | C++, with Python packages | Apache-2.0 | The Insight Toolkit for N-dimensional scientific image processing, segmentation and registration. BIL uses its image readers for medical formats such as DICOM, NIfTI, NRRD and MetaImage. Its registration and segmentation are image processing, so they stay outside BIL. |

Registration is a layer above BIL and is not considered in this specification.

### A.13 Second survey, with recommendations

These 40 libraries were each opened on 10 October 2026 and assessed for BIL. All remain in the plugin design. The recommendation is input to the case-by-case review, not a decision. Licenses are as each project states them.

| Recommendation | Meaning | Count |
| --- | --- | --- |
| Adopt | A good fit for a plugin in the default build | 14 |
| Adopt on trial | Promising, to be proven on real files before it is relied on | 4 |
| Optional plugin only | Its license is stricter than the LGPL, so it ships separately, as Grok does | 2 |
| Use indirectly | Reached through another plugin, so it needs no plugin of its own | 5 |
| Fallback, with caution | Usable, but with a stated risk | 1 |
| Reference | Worth studying or testing against, but not loaded by BIL | 8 |
| Defer | Narrow, stale or outside current needs | 6 |

**JPEG 2000 and TIFF**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [OpenJPEG](https://github.com/uclouvain/openjpeg) | C; BSD-2-Clause | Fallback, with caution | Adopted as the JPEG 2000 fallback (section 10). JPEG 2000 Parts 1 and 2, and the ISO reference software. Its own page says the repository "should be considered unmaintained", with no committer regularly reviewing tickets or pull requests. |
| [libtiff](https://libtiff.gitlab.io/libtiff/) | C; permissive LibTIFF license | Adopt | The reference TIFF library, at version 4.7.2: BigTIFF, tiles, strips and multi-page files. The fallback for unusual TIFF variants that a Rust reader rejects. |
| [libgeotiff](https://github.com/OSGeo/libgeotiff) | C; MIT or public domain | Adopt | Reads and writes GeoTIFF keys. Small, but it brings in libtiff, PROJ and SQLite. |

**SAR**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [SarPy](https://github.com/ngageoint/sarpy) | Python; MIT | Adopt, through scripted drivers | NGA's library. Beyond SICD, SIDD, CPHD and CRSD, it reads Capella, COSMO-SkyMed, GFF, ICEYE, NISAR, PALSAR2, RADARSAT-2, RCM, Sentinel-1 and TerraSAR-X as SICD-like data. Version 2.0 merges SARkit. It is the quickest route to many vendor SAR formats: the script interprets the metadata and the compiled core reads the pixels. |
| [ISCE3](https://github.com/isce-framework/isce3) | C++ and Python; license not stated on its page | Reference | An InSAR processing library funded under NISAR, not a reader. Its page says it is in early development with a changing interface. NISAR products are HDF5, which BIL reads directly. |
| [xarray-safe-s1](https://github.com/umr-lops/xarray-safe-s1) | Python; MIT | Reference | A small Sentinel-1 SAFE reader that decodes the calibration tables from the product XML. A guide to those tables; SarPy covers the format. |

**Raw camera files**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [rawloader](https://github.com/pedrocr/rawloader) | Rust; LGPL-2.1 | Use indirectly | Decodes the raw formats of most camera makers and returns raw pixels, the color filter pattern, black and white points, white balance and the color matrix, with no demosaicing. Reached through rawler, which is built on it. Its DNG support is partial. |
| [quickraw](https://github.com/RawLabo/quickraw) | Rust; LGPL-2.1 | Defer | Last released in September 2022, at version 0.1.6. Its page says its integer processing "may not get a very precise result". |

**Video**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [dav1d](https://github.com/videolan/dav1d) | C and assembly; BSD-2-Clause | Adopt | The AV1 decoder, covering every AV1 feature, subsampling and bit depth, with vector code for x86, ARM and other processors. FFmpeg can use it, and rav1d is its Rust port with the same interface. |
| [GStreamer](https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html) | C; LGPL | Adopt | The second stream source beside FFmpeg, strongest for live streams and pipelines. libmisbklv already builds on it for KLV. Its "ugly" plugin set holds codecs with known patent claims, so that set should not ship by default. |
| [libde265](https://github.com/strukturag/libde265) | C; LGPL-3.0 | Use indirectly, through the optional libheif plugin (section 10.6) | An HEVC decoder with tile and wavefront threading. It is the decoder behind HEVC images in libheif, so it arrives with the GIMI and HEIF plugin. |
| [OpenH264](https://github.com/cisco/openh264) | C++; BSD-2-Clause | Defer | Its decoder covers the Constrained Baseline profile only and fails on compressed frames over 1 MB. Too narrow for sensor video, so FFmpeg's H.264 decoder is used. |

**Scientific arrays**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [hdf5-metno](https://github.com/metno/hdf5-rust) | Rust; MIT or Apache-2.0 | Adopt | The maintained Rust bindings to the HDF5 C library, which is how the Rust core reaches that library. It makes HDF5 thread-safe with a lock, so reads through it do not run in parallel. |
| [hdf5-pure](https://docs.rs/hdf5-pure) | Rust; MIT or Apache-2.0 | Adopt on trial | HDF5 with no C library, at version 0.47.0 (September 2026). Reads contiguous, compact and all chunked layouts, with Deflate, scale-offset and ZFP filters. It would remove the lock that hdf5-metno needs. |
| [HDF4](https://github.com/HDFGroup/hdf4) | C and Fortran; BSD | Use indirectly | In maintenance mode: its page says no new development occurs and HDF4 is deprecated. Needed only for older archives, which GDAL already reaches. |
| [CFITSIO](https://github.com/HEASARC/cfitsio) | C and Fortran; permissive NASA notice | Adopt, low priority | The standard FITS library for astronomy. Worth a plugin only when FITS data is needed. |

**Medical and microscopy**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [dicom-rs](https://github.com/Enet4/dicom-rs) | Rust; MIT or Apache-2.0 | Adopt on trial | A pure Rust DICOM implementation under active development, with a crate that decodes pixel data. A light way to read DICOM without bringing in all of ITK. |
| [DCMTK](https://github.com/DCMTK/dcmtk) | C++; BSD-3-Clause | Defer | A full DICOM toolkit with JPEG and JPEG-LS decoding. It overlaps ITK, which is already included. |
| [OpenSlide](https://github.com/openslide/openslide) | C; LGPL-2.1 | Defer | Reads whole-slide microscope images from Aperio, Hamamatsu, Leica, Zeiss and others. Worth a plugin only when microscopy is needed. |

**Still-image codecs**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [LERC](https://github.com/Esri/lerc) | C++; Apache-2.0 | Adopt | Compresses every numeric type, lossless or to a stated maximum error, with a per-pixel validity mask. That mask maps directly onto BIL's NaN rule for missing data. Used inside GeoTIFF and MRF. |
| [CharLS](https://github.com/team-charls/charls) | C++; BSD-3-Clause | Adopt | JPEG-LS lossless and near-lossless, as found in DICOM. It does not support subsampled scans or the Part 2 extensions. |
| [image-rs](https://github.com/image-rs/image) | Rust; MIT or Apache-2.0 | Adopt | One Rust dependency for the everyday formats: PNG, JPEG, BMP, GIF, TIFF, WebP, PNM, TGA, QOI, HDR and EXR. It makes separate plugins for several of them unnecessary. |
| [zune-image](https://github.com/etemesi254/zune-image) | Rust; MIT, Apache-2.0 or Zlib | Adopt on trial | Its JPEG decoder aims to match libjpeg-turbo, it keeps 8-bit, 16-bit and float data at source depth, and its decoders are fuzzed daily. To be compared with libjpeg-turbo on the JPEG blocks inside NITF and TIFF. |
| [OpenEXR](https://github.com/AcademySoftwareFoundation/openexr) | C++; BSD-3-Clause | Adopt, low priority | Multi-part, multi-channel floating point images. EXR commonly stores 16-bit floats, which are not among BIL's sample types, so adopting it means adding one. |
| [libpng](https://github.com/pnggroup/libpng) | C; PNG Reference Library License 2 | Use indirectly | The reference PNG library, with vector code for many processors. image-rs and zune-image read PNG without it. |
| [libavif](https://github.com/AOMediaCodec/libavif) | C; BSD-2-Clause | Use indirectly | AVIF in every AV1 format and bit depth, decoding through dav1d. libheif already reads AVIF. |
| [libjxl](https://github.com/libjxl/libjxl) | C++; BSD-3-Clause | Defer | The JPEG XL reference implementation. jxl-oxide in A.9 already decodes JPEG XL in Rust. |
| [libwebp](https://github.com/webmproject/libwebp) | C; BSD-3-Clause | Defer | WebP is a display format with little use for sensor data. image-rs reads it if it is ever needed. |

**Geometry and coordinates**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [PROJ](https://github.com/OSGeo/PROJ) | C and C++; MIT | Adopt | The standard library for coordinate transformations, with datum grids that can be fetched on demand. GDAL and libgeotiff already require it. |
| [Community Sensor Model](https://github.com/USGS-Astrogeology/csm) | C++; Unlicense | Adopt | An interface for sensor models, with plugin loading and model types such as `RasterGM`. Adopting it would let BIL load usgscsm and any other CSM plugin, which answers part of the open sensor geometry decision. |
| [OSSIM](https://github.com/ossimlabs/ossim) | C++; MIT | Reference | Mostly processing: orthorectification, reprojection and terrain analysis, which sit above BIL. |
| [Orfeo ToolBox](https://github.com/orfeotoolbox/OTB) | C++; Apache-2.0 | Reference | A processing toolbox for optical and radar imagery, not a reader. A layer above BIL. |

**Metadata and color**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [Little CMS](https://github.com/mm2/Little-CMS) | C; MIT | Reference | The standard color management engine, at version 2.19.1 (May 2026), implementing ICC 4.4. BIL does not apply color profiles, so this is a library for the layer above, which reads the profile from BIL's fields. |
| [moxcms](https://github.com/awxkee/moxcms) | Rust; BSD-3-Clause or Apache-2.0 | Adopt on trial | Color management in pure Rust. A possible replacement for Little CMS that keeps the core free of C. |
| [Exiv2](https://github.com/Exiv2/exiv2) | C++; GPL-2.0-or-later | Optional plugin only | Reads Exif, IPTC, XMP and ICC metadata, including from CR3, HEIF, AVIF and JPEG XL. It is GPL, so like Grok it can only be an optional plugin that changes the license of the installation. |
| [lensfun](https://github.com/lensfun/lensfun) | C++; library LGPL-3.0, database CC BY-SA 3.0 | Optional plugin only | A lens database with vignetting, distortion and chromatic aberration correction. Vignetting fits BIL as a normalization. Distortion correction moves pixels, so it sits above BIL. |
| [ExifTool](https://github.com/exiftool/exiftool) | Perl; GPL or Artistic | Reference | Knows about 225 file types and the maker notes of most camera brands. It is a Perl program, so it cannot be linked, though a scripted driver could run it as a separate program. |

**Storage and unifying libraries**

| Library | Language and license | Recommendation | Why |
| --- | --- | --- | --- |
| [object_store](https://github.com/apache/arrow-rs-object-store) | Rust; Apache-2.0 | Adopt | One asynchronous interface to S3, Azure Blob, Google Cloud Storage, HTTP, local files and memory. The natural storage backend. |
| [OpenImageIO](https://github.com/AcademySoftwareFoundation/OpenImageIO) | C++; Apache-2.0 | Reference | The closest existing design to BIL: one reading interface and one plugin per format, loaded at run time. Its cache for very large image sets is worth studying. It is aimed at film and animation, so it lacks the sensor formats. |
| [libvips](https://github.com/libvips/libvips) | C; LGPL-2.1-or-later | Reference | Demand-driven, threaded processing that keeps memory low, with pixel types up to 128-bit complex. The model for `stream_slice`. As a processing library it sits above BIL. |

### A.14 Not open source

Kakadu, the ECW and MrSID kits and the Blackmagic RAW kit are proprietary. BIL can reach them only through an external driver supplied by someone who holds a license.

### A.15 Related projects

These projects overlap with BIL as a whole, rather than supplying one format. Section 1.7 compares them with BIL. GRDL and OxiGeo were checked against their own pages on 12 October 2026. The other rows are from earlier knowledge and were not rechecked.

| Project | Language and license | Relation to BIL |
| --- | --- | --- |
| [GRDL](https://pypi.org/project/grdl/) | Python; MIT | The nearest in purpose: building blocks for intelligence imagery. It reads GeoTIFF, NITF, HDF5 and JPEG 2000, and for SAR it reads SICD, SIDD, CPHD, CRSD, Sentinel-1, TerraSAR-X, NISAR and BIOMASS. For EO it reads Sentinel-2 and EO NITF. It locates pixels by affine, SICD, RPC, RSM and control point models, with an optional elevation model, and it reads chips by row and column. Version 0.7.4, September 2026, alpha, with one maintainer. Pure Python, with SARkit, SarPy and rasterio as optional backends. No video. A candidate second validation reference (section 13.8), and a possible user of BIL as its reader. |
| [OxiGeo](https://github.com/cool-japan/oxigeo) | Rust; Apache-2.0 | A Rust alternative to GDAL for map and cloud data. Its drivers are candidates for BIL (A.11). It has bindings for Python, Node.js and WASM but no C interface. |
| [GDAL](https://gdal.org) | C and C++; MIT | The incumbent for map rasters, and a BIL backing library (section 10). |
| [OpenImageIO](https://github.com/AcademySoftwareFoundation/OpenImageIO) | C++; Apache-2.0 | The closest design (A.13), aimed at film and animation. |
| [imageio](https://github.com/imageio/imageio) | Python; BSD-2-Clause | One Python front end over many readers, video included. No windowed reads, levels or geocoding. |
| [Bio-Formats](https://github.com/ome/bioformats) | Java; GPL-2.0 | One interface to most microscope formats. Reached only through external decoding (section 10.2). |

### A.16 Color

Checked on 12 October 2026, except colour-science, which is from earlier knowledge. BIL computes color itself (section 9.3), so none of these is loaded by BIL.

| Library | Language and license | Role |
| --- | --- | --- |
| [colour-science](https://github.com/colour-science/colour) | Python; BSD-3-Clause | The reference the color functions are tested against |
| [palette](https://docs.rs/crate/palette/0.7.7) | Rust; MIT or Apache-2.0 | Color-space arithmetic with white points as parameters. A second reference. It has no YCbCr. |
| [Little CMS](https://github.com/mm2/Little-CMS) | C; MIT | ICC color management, for the layer above (A.13) |
| [moxcms](https://docs.rs/crate/moxcms/0.9.0) | Rust; BSD-3-Clause or Apache-2.0 | Pure Rust ICC conversion, for the layer above. Young: first release February 2025, with one maintainer. |
| [OpenColorIO](https://opencolorio.readthedocs.io/en/latest/releases/ocio_2_5.html) | C++; BSD-3-Clause | The film industry's color pipeline, version 2.5. Aimed at displays. |

## Appendix B. Candidate test data

BIL needs real files to prove its drivers, its speed and its bounded-memory rule. One candidate has been identified so far.

| Dataset | What it is | Why it suits BIL |
| --- | --- | --- |
| [CLIF 2007](https://github.com/AFRL-RY/data-clif-2007) | Wide area motion imagery collected by the US Air Force Research Laboratory over a university campus in October 2007. About 5 TB, in the public domain, with over 3.5 million human-made object labels. Six monochrome cameras of 4,016 × 2,672 pixels each give a stitched frame of about 66 megapixels, at roughly 2 frames a second. | It is large enough that it cannot be loaded whole. It comes both as raw 8-bit camera frames and as NITF with JPEG-compressed 128 × 128 blocks and six resolution sets per frame. Blocks exercise Windows, resolution sets exercise Levels, and the frame sequence exercises deferred reads. |

Its details are as stated on the dataset's page, checked on 11 October 2026. The data sits in a requester-pays store, so downloading it has a cost.
