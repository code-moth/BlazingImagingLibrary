## How to use this document

Each finding is a weakness in the specification, with a proposed resolution and the decision it needs from you.

- **Work through them in any order.** Give the finding's number and your ruling, and both this document and the specification are updated.
- **Every finding carries a status.** It starts as Open, and moves to Accepted, Rejected or Deferred. An accepted finding is marked Resolved once the specification has changed, with the version that changed it.
- **The resolutions are proposals.** They are my suggestions, not decisions, and several offer more than one option.
- **This was a paper review.** The specification was read end to end and checked against itself and against what is known of the formats and libraries. Nothing was run.

## Summary

{{SUMMARY}}

## Critical findings

These break one of the specification's own five rules, or leave a core mechanism with no place in the design.

### R1. Stored pyramids have no home

**Status:** Resolved in version 24 · **Where:** sections 4, 5 and 8.4

**Problem.** A Level is defined as bilinear downsampling from full resolution. The Window `step` is defined as every nth stored pixel, with no filtering. Data that a file stores at reduced resolution is neither: overviews, whole-slide pyramid levels, NITF reduced-resolution sets and JPEG 2000 resolution levels are all filtered by the file's own method. Section 5 says `step` "can use overviews stored in the file", which contradicts the Fragment definition in section 4.

**Why it matters.** A Level 8 view of a 100,000 × 100,000 slide returns about 150,000 pixels, but BIL must decode all 10 gigapixels to make it. The cost follows the collection, which breaks the Proportional rule for exactly the data section 1.1 is written about.

**Proposed resolution.**

- Name stored reduced-resolution data as its own thing, a stored level. `list_stored_levels` reports each one with its scale and, where the file states it, the method that made it.
- Add a level source to `SliceOptions`. `Exact` computes from full resolution, as now. `Stored` starts from the nearest stored level and applies the level method from there.
- `plan_slice` reports which was used. `step` stays strictly unfiltered.

**Decision needed.** May a Level read start from a stored level, accepting that its values then depend on how the file was made?

**Ruling.** Accepted, with two additions.

- A computed Level must always read the data under it. An averaged view of a whole image means processing the whole image, and that cost is accepted.
- For large reductions a decimation strategy is needed that strides across the data.
- Stored reduced-resolution data is to be named as such, and the stored solution implemented.

**What changed in the specification.** Version 24, section 5.

- The cost of each route to a reduced read is stated: computed with a filter, computed with `decimate`, and stored.
- `decimate` is a built-in level method that reads only the pixels it keeps. `decimate_first` takes the first halvings by striding and leaves the rest to the level method.
- Stored levels are listed by `list_stored_levels` and named `stored1`, `stored2` and so on. They are read directly with `stored_level`, or used to make a Level with `level_source` set to `Stored`. They are never used unasked.
- Every Slice reports a level name, such as `level3`, `stored2` or `stored2.level1`.
- `step` no longer claims to use stored overviews, which removes the contradiction with the Fragment definition.

### R2. There is no cache of decoded tiles

**Status:** Resolved in version 25 · **Where:** sections 5 and 12, and the open decision on a frame cache

**Problem.** Compressed formats decode whole tiles or blocks. Adjacent Windows that share a tile each decode it again. BIL's own margins make this worse: a Window aligned to a tile boundary reads a margin of one or two pixels, and decodes every neighboring tile to get it.

**Why it matters.** An algorithm that steps through a tiled JPEG 2000 image one tile at a time, using a local slice function, decodes each tile up to nine times: once for itself and once for each of its eight neighbors.

**Proposed resolution.**

- A decoded-block cache with a size the caller sets in bytes.
- `plan_slice` reports how many blocks a read will decode.
- Merge this with the open decision on the decoded frame cache, since a video frame is one more kind of block.

**Decision needed.** The default cache size, and whether one cache serves every Source or each Source has its own.

**Ruling.** Accepted. The cache size is to be set as a number of Fragments kept, not in bytes.

**What changed in the specification.** Version 25, section 4.

- A Fragment cache keeps recently decoded blocks and video frames, each as a Fragment. `set_cache_size` sets how many are kept, and zero turns it off.
- `describe_cache` reports the bytes held, because a count does not fix the memory used. `plan_slice` reports the blocks a read will decode.
- The open decision on a decoded frame cache is closed, since a video frame is one more kind of block.
- Two details were not ruled on and are my choice: each Source has its own cache, and the default is 16 Fragments.

### R3. Video does not fit the model

**Status:** Resolved in version 25 · **Where:** sections 4, 8.3, 9.3 and 15

**Problem.** There are four separate faults.

1. **The Fragment shape.** A deferred Fragment holds compressed packets, but `data` and `strides` describe pixels. Decoder state kept between reads is changing state behind a call declared as read-only.
2. **Frame count and frame times.** A transport stream has no index, so both need a scan of the whole stream. That contradicts "opening extracts little".
3. **A Window saves nothing.** Decoding a small Window still decodes whole frames, from the preceding key frame.
4. **Chroma.** Section 8.3 makes the half-size chroma planes separate Images. Section 9.3 makes them Slices of one Image. Section 15 puts chroma upsampling outside the model, so the most common video in the world cannot give `color.rgb`.

**Why it matters.** Video is one of the five format groups BIL must read, and wide area motion imagery is the worked example that opens the specification.

**Proposed resolution.**

- Make deferred a distinct state in the Fragment type, either pixels or packets.
- Move decoder state out of the Image and into a reader object that each thread owns (see R13).
- Let the frame count be `Known`, `AtLeast` or `Unknown`. Build the frame index on first need, and allow it to be saved beside the file.
- `plan_slice` reports how many frames a read will decode.
- Treat 4:2:0 video as one Image. Add `video.rgb` as a local slice function with a fixed, stated chroma upsampling. It uses a small fixed neighborhood tied to the Image grid, so it meets the window-independence rule.

**Decision needed.** Whether chroma upsampling comes into BIL as a local slice function.

**Ruling.** Accepted. Chroma upsampling is part of the local slice function, because a Slice is a representation of the input data at a set resolution, and every Slice maps its coordinates in the same way.

**What changed in the specification.** Version 25, sections 3.2, 4, 5, 8.3 and 9.3.

- A Fragment is in one of three states: `Pixels`, `Planes` or `Packets`. A deferred Fragment is in the `Packets` state.
- Decoder state is held in a Reader, one for each thread, and not in the Image.
- `count_frames` returns a count that is `Known`, `AtLeast` or `Unknown`. The frame index is built on first need, never when a Source is opened, and can be saved.
- `plan_slice` reports how many frames a read will decode. The specification now says plainly that a Window does not reduce video decoding.
- Video with reduced chroma is one Image. `video.Planes` and the new `video.rgb` upsample chroma to the luma grid, by a stated bilinear method.
- Section 5 gains the rule "one grid for every Slice".

**Follow-up ruling.** A Slice is the caller's chosen representation of the Fragment, and its Level is part of it. For 4:2:0 and similar uneven representations of the vector pixel, the resolution at the requested Level is to be the same across all Slices.

**What changed in the specification.** Version 26, sections 5, 9.3, 13 and 15.

- The rule is now "the Level sets the grid". At a given Level, every Slice of an Image has the same size, whichever slice function made it.
- The half-size Slices are gone. `cfa.sites`, `cfa.binned`, the `polcam` family and `video.fields` interpolate their samples onto the full grid, as `video.Planes` does for chroma.
- The stored samples stay available, unaltered, in the Fragment.

**Revised in version 44.** The symmetry review removed the `Planes` state. Luma and reduced chroma are now two related Images, and a Fragment always has one shape. Deferred decoding is a reported property, `is_deferred`.

### R4. The markings rule contradicts itself

**Status:** Resolved in version 27 · **Where:** sections 11 and 14

**Problem.** Section 14 says markings are always written. The same table says `write_metadata` writes the fields the format can hold. A NumPy `.npy` file holds none, and ENVI and TIFF have no defined place for NITF security fields. A Slice handed to NumPy carries no marking at all.

The rule also treats two different things as one. A security classification must never be dropped. A patient's identity usually should be removed, unless the caller asks to keep it.

**Why it matters.** As written, reading a classified NITF and writing `.npy` strips the classification silently, and copying a medical image carries the patient's name into every output by default.

**Proposed resolution.**

- Split the flag in two: `Marking`, which must be kept, and `Identity`, which is personal data and is not written unless the caller asks.
- When a writer cannot hold a `Marking` field, it either refuses or writes the markings to a file beside the output.
- A Slice and a Fragment keep a reference to the markings of the Source they came from.

**Decision needed.** Refuse, write a side file, or let the caller override. And the default for identity fields.

**Ruling.** Accepted. Write a side file where necessary, and let the caller override. The default for identity fields is to leave them empty. Medical imaging stays in scope.

**What changed in the specification.** Version 27, sections 11 and 14.

- A field's definition gives it a handling: `Ordinary`, `Marking` or `Identity`.
- Markings are always written. Where the output format has no place for them, they go to a side file named for the output, with `.markings` added. BIL reads that file back when it opens the output.
- Identity fields are written empty. The caller keeps them with `keep_identity`, and can turn markings off with `keep_markings`.
- Every Fragment and Slice keeps a reference to the markings of its Source, and `write_fragment` and `write_slice` write them without being asked.
- The limits are stated: BIL empties only flagged fields and is not a tool for removing identity, and a plain array that leaves BIL no longer carries its markings.

**Revised in version 45.** The markings side file became the companion file, named with `.metadata`. It carries markings and any other metadata an output format cannot hold.

### R5. Decoded data can spill, and BIL may reach the network unasked

**Status:** Resolved in version 28 · **Where:** sections 10.2 and 13.6, and the open decision on default terrain

**Problem.** The mapped format holds decoded pixels uncompressed, for other processes to read. The specification says nothing about where it is put, who may read it, how long it lives or how it is removed. Decoding a classified or patient Source therefore leaves a clear copy on disk.

The candidate default terrain is fetched from public cloud storage. Each tile request tells an outside server where the user is looking.

**Why it matters.** Either behavior would stop BIL being approved for a closed network, and the second discloses the area of interest.

**Proposed resolution.**

- **No network by default.** BIL makes no network request unless the caller gave it a remote location. The default surface is a constant height, and remote terrain is chosen explicitly.
- **Mapped data is controlled.** The caller names its location. It is created readable by its owner only, carries the Source's markings, and is removed when the decoder process ends unless the caller asks to keep it. Shared memory with no file is offered as an alternative.

**Decision needed.** Confirm "no network unless asked" as a rule of the specification.

**Ruling.** Accepted. Implement the recommendations.

**What changed in the specification.** Version 28, sections 1.2, 1.3, 10.2 and 13.6.

- A sixth design rule, Contained, with a new section 1.3 that sets out two rules: no network unless asked, and no copy unless asked.
- Backing libraries are held to the same rules. PROJ does not download grids, and XML parsing does not fetch external references.
- No terrain is fetched by default. With no surface given, a constant height is used and the result says so. The open decision on default terrain is closed.
- Mapped data has fixed handling: the caller chooses the place, only its owner can read it, it carries markings, and it is removed after use unless the caller keeps it. Shared memory with no file is an option.
- The specification states that BIL does not encrypt mapped data, and that it cannot hold plugins and scripted drivers to these rules.

### R6. Sixty parsers of untrusted files run in the caller's process

**Status:** Resolved in version 29 · **Where:** section 10 and Appendix A

**Problem.** BIL brings about sixty C and C++ parsers into the caller's process, and imagery files arrive from outside. A file crafted to exploit any one of them takes over the caller. A Rust core gives no protection for code it only calls.

There are no resource limits either. A header can claim any size, and a small compressed block can expand without bound. Plugins found at run time and scripted drivers loaded from a folder are both ways to load code.

**Why it matters.** The more formats BIL reads, the more ways in it offers. That is the direct cost of the library's main selling point.

**Proposed resolution.**

- Resource limits in the options to `open_source`: largest image size, most decoded bytes, largest metadata group.
- Present external decoding (section 10.2) as isolation as well as convenience. In an isolated mode, any C or C++ driver runs in the decoder process with reduced privileges.
- Plugins load only from locations the caller has configured, never from a search path.
- BIL's own Rust drivers and parsers are fuzz tested as a requirement.

**Decision needed.** Whether isolation is the default for Sources the caller has not marked as trusted.

**The case against isolation by default.** It was weighed before the ruling.

- An isolated read cannot borrow from the original file, and always makes one decoded copy.
- Every read becomes a round trip between processes, which hurts small Windows most.
- It needs a default place for mapped data, against the containment rules of R5.
- It makes the unfinished mapped format part of the first tier.
- Reduced privileges work differently on every platform, and some environments forbid them.
- If a trusted flag restores speed, most callers will set it on everything.

**Ruling.** The middle course. Adopt the limits, the configured plugin locations and the fuzz testing. Offer isolation as a setting, off by default.

**What changed in the specification.** Version 29, new section 10.3.

- `OpenOptions` carries three limits the caller sets: `max_image_pixels`, `max_decoded_bytes` and `max_field_group_bytes`. None is set by default.
- Plugins load only from configured locations, with no searching. An installed optional package records its location, so it is still found.
- `isolation` is `Off`, `Untrusted` or `Always`, and defaults to `Off`. A deployment can set it once as policy.
- When isolation is called for and cannot be provided, the open fails. BIL does not fall back to the caller's process.
- BIL's own Rust readers are fuzz tested as a condition of release. The C and C++ libraries are not covered by that.

### R7. The scale has no phasing

**Status:** Resolved in version 30 · **Where:** the whole specification

**Problem.** The specification holds about sixty slice functions, thirteen geocoding steps, sixty candidate libraries, an embedded interpreter, a new file format, writers and terrain access. Nothing says what is built first, or when a driver counts as finished.

**Why it matters.** Without an order, the core concepts are not proven until much of the breadth exists, and a flaw found late is expensive.

**Proposed resolution.**

- **Tiers.** Tier 1 proves the core with the fewest dependencies: the native ENVI driver, TIFF and GeoTIFF, NITF with JPEG 2000, and one video path. It carries `raw`, `complex`, `color`, `spectral.bands` and `cfa.demosaic`, with `affine` and `rational_polynomial` geocoding.
- **A driver conformance suite.** Every driver must pass the same tests: window independence, round trip, planned against actual cost, missing data, and metadata on demand.

**Decision needed.** Which formats are in Tier 1.

**Ruling.** Accepted. GeoTIFF and NITF are Tier 1.

**What changed in the specification.** Version 30, new sections 1.4 and 10.4.

- BIL is built in tiers, and each is finished before the next is started.
- Tier 1 is GeoTIFF and NITF, with the core concepts, four slice functions, `affine` and `rational_polynomial` geocoding, metadata with markings, a TIFF writer, and the Rust and Python interfaces.
- Tier 1 leaves out video, scripted drivers, external decoding and every other format. ENVI and the video path, which the proposal had in Tier 1, move to a proposed Tier 2.
- A driver conformance suite of eleven tests is the same for every driver, and a failed test blocks release.
- What Tier 2 holds is a new open decision.

**Additions confirmed.** The details added when this was written into the specification have been accepted.

- Compression in Tier 1: uncompressed in both formats, Deflate in GeoTIFF and JPEG 2000 in NITF.
- A TIFF and GeoTIFF writer in Tier 1, for the round-trip test.
- Rust and Python interfaces in Tier 1, with the C interface for external plugins in Tier 2.
- `image_window`, `affine` and `rational_polynomial` geocoding in Tier 1, with a constant-height surface.
- The eleven conformance tests.
- `cfa.demosaic` moved out of Tier 1 with the formats that need it.

**Tier 2 decided.** Version 31 of the specification settles Tier 2 as proposed: ENVI, video with KLV, and SICD and SIDD. The order of Tier 3 is not yet set.

## Serious findings

These are gaps or contradictions that would surface as soon as the affected part is built.

### R8. The zero-copy rule is missing conditions

**Status:** Resolved in version 46 · **Where:** sections 4, 6 and 12

**Problem.**

1. **Alignment.** Pixel data starts wherever the file puts it. A float32 view at an odd offset is undefined behavior in Rust, and slow or refused elsewhere.
2. **Files that change.** If another process shortens a mapped file, the reader crashes. Nothing can catch it.
3. **Storage that cannot be mapped.** Object stores and streams never meet the fourth condition, so zero-copy does not exist for them.
4. **Writing to borrowed data.** Borrowed arrays must be read-only, and algorithms that work in place will fail on them.
5. **Lifetime in Python.** A borrowed NumPy array must keep its Source open.

**Proposed resolution.**

- Add a fifth condition: samples are aligned for their type.
- Borrowed data is read-only, and is exposed to NumPy as a read-only array that holds a reference to its Source.
- Offer a private mapping for callers who modify in place, and an option to turn mapping off for network file systems.
- State that mapping a file while it is being changed is not supported.

**Decision needed.** Accept these as additions to the rule.

**Ruling.** Accepted.

**What changed in the specification.** Version 46, sections 4 and 12.

- The zero-copy rule gains a fifth condition: each sample starts at an address that is a multiple of its own size.
- Borrowed data is read-only, and in Python is a read-only NumPy array that holds a reference to its Source.
- `mapping` in `OpenOptions` is `Shared` by default, `Private` for copy-on-write, or `Off` for network file systems. The two options in the proposal became one setting with three values, in keeping with the uniform rule.
- Mapping a file while another process changes it is stated as unsupported.

### R9. Grid alignment is not specified

**Status:** Resolved in version 46 · **Where:** sections 5, 9.3 and 13.2

**Problem.**

1. **Levels.** Level n values sit on Image pixels at multiples of 2ⁿ. Nothing says what a read returns, or how large it is, when the Window starts elsewhere.
2. **Block functions.** The same holds for half-size Slices. For a Window starting on an odd row, which site is `red`?
3. **Step with level.** Nothing says what happens when both are set.
4. **Range.** The range rule takes a 2×2 block to the right and below, while bilinear values are centered on a pixel. `range` and `intensity` at the same Level are therefore offset from each other.
5. **Mirroring.** At an Image edge, mirroring must keep the color filter pattern in step.

**Why it matters.** Window independence is the specification's central guarantee, and it cannot be tested until these are fixed.

**Proposed resolution.**

- Level n of a Window returns the grid points at multiples of 2ⁿ that fall inside it. `plan_slice` reports the size.
- Every Slice carries its origin and scale in Image pixels, as 64-bit values, and geocoding reads them per Slice.
- `step` above 1 and `level` above 0 may not be combined.
- Block functions are anchored to the Image grid in the same way. Mirroring reflects by whole pattern periods.
- For range, either accept the stated offset and record it in the Slice, or take the minimum of a 3×3 neighborhood centered on the grid point, which keeps `range` aligned with `intensity`.

**Decision needed.** The range option, and whether to forbid `step` with `level`.

**Note.** Version 26 of the specification removed half-size Slices, so item 2 no longer applies. Item 4, the offset of `range`, is now the one remaining exception to "every Slice maps positions the same way". The rest of this finding is still open.

**Ruling.** Accepted. No range option was named, so the centered 3×3 minimum was chosen, because it keeps `range` on the same grid as every other Slice.

**What changed in the specification.** Version 46, sections 5 and 13.2.

- Level n of a Window holds the grid points at multiples of 2ⁿ that fall inside it, and `plan_slice` reports the size.
- Every Slice carries its `origin` and `scale` in Image pixels, as 64-bit values, and the `image_window` step of geocoding reads them.
- A `step` above 1 and a Level above 0 cannot be combined.
- At an Image edge, pixels are mirrored by whole periods of any pattern in the data.
- Range is reduced by the minimum of a 3×3 block centered on each grid point.

### R10. The sample types cannot hold the specification's own cases

**Status:** Resolved in version 32 · **Where:** sections 4 and 8

**Problem.** Section 8.4 lists 1-bit masks, and CPHD stores complex samples with one byte per part. Neither has a sample type. Also missing are 16-bit float, signed 8-bit, 64-bit integers and complex with 64-bit parts, all of which ENVI, HDF5 or OpenEXR can hold.

Justification is not covered. Ten-bit video is commonly stored in the high bits of 16, so "exactly the stored integers" returns values 64 times too large.

**Proposed resolution.**

- Add `I8`, `F16`, `U64`, `I64`, `ComplexI8` and `ComplexF64`.
- Deliver 1-bit data unpacked as `U8` with `significant_bits` set to 1.
- State that a Fragment delivers values right-justified.

**Decision needed.** Which types to add. The 64-bit integers lose precision in a Slice, as 32-bit ones already do.

**Ruling.** Add all six types to the Fragment. A Slice is always the same float representation, so that algorithms applied to the output use one representation. A Fragment delivers data in its usual stored form, and the Slice delivers it appropriately. This replaces the proposal that a Fragment should right-justify.

**What changed in the specification.** Version 32, sections 4, 5, 6, 8 and 12.

- `I8`, `F16`, `U64`, `I64`, `ComplexI8` and `ComplexF64` are added, making fifteen sample types.
- A Fragment does not shift samples. `bit_shift` records unused low bits, as hardware video decoders and some NITF files produce.
- A Slice works from the sample value, so 10-bit video gives 0 to 1023 whichever decoder produced it.
- One-bit data is unpacked to `U8` with one significant bit.
- A table states which sample types are exact as float32 and which are rounded. Arithmetic is done at the source's precision and narrowed once.
- Only the six common sample types get their own compiled loop. The rest go through a general loop, which also answers the first item of R25.

### R11. A Fragment has one sample type, and a slice function has one Image

**Status:** Resolved in version 42 · **Where:** sections 4, 9.2 and 9.5

**Problem.** A pixel is several samples of one type. Range in float32 beside intensity in 16-bit integers cannot be one pixel, yet `depth.sensor` yields both from one Fragment. Color with depth has the same problem. An interferogram's coherence is "its own Image", and no slice function can read two Images.

**Proposed resolution.**

- Keep the Fragment to one sample type, which keeps it a simple strided view.
- Hold mixed data as separate Images that declare they share one grid.
- Let a slice function declare more than one input Image on the same grid. `pixel_gain_offset` already reads a second Image, so the precedent exists. The function stays pointwise, so it stays window-independent.

**Decision needed.** Accept slice functions with more than one input Image.

**Ruling.** Accepted.

**What changed in the specification.** Version 42, sections 3, 4 and 5.

- A Fragment keeps one sample type, which keeps it a simple strided view.
- A driver can declare that Images share a grid, and mixed data is held that way.
- A slice function may declare several input Images on one grid. BIL reads the same Window from each, and the function stays pointwise or local.
- `insar.interferogram` gains a `coherence` Slice, read from the second Image.

### R12. Slice selection is a string with no grammar

**Status:** Resolved in version 42 · **Where:** sections 5 and 9

**Problem.**

1. **Ambiguity.** `raw.intensity` is a function and a Slice name. `color.hsv` is a family and a function. They cannot be told apart without looking both up.
2. **Parameters.** `complex.polar(hv)` puts a parameter in the string. `spectral.response(curves)` takes arrays, which cannot travel that way.
3. **Names from metadata.** Band names in files contain spaces and punctuation, repeat, and are not UpperCamelCase.
4. **Names from wavelength.** `nm550` collides when two bands are less than a nanometer apart.

**Proposed resolution.**

- Give every slice function a family, so a full name always has three parts. Short forms such as `raw` become registered aliases.
- Make selection structured: a function name, named parameter values and a Slice name. The single string remains as shorthand for functions without parameters.
- State the rule for names taken from metadata: convert to UpperCamelCase, add a number on collision, and keep the original as a label.
- Wavelength names carry enough digits to be unique.

**Decision needed.** The family name for the general functions of section 9.1.

**Ruling.** Accepted. No family name was given, so `pixel` was chosen.

**What changed in the specification.** Version 42, sections 5 and 9.

- Every slice function has a family, so a full name has three parts. The general functions are in the `pixel` family, as in `pixel.raw`.
- Two parts select a function that yields one Slice, and one part is a registered alias, such as `raw`.
- Parameters are passed in a `SliceSelector` as named values, never inside the name.
- Names taken from metadata are converted to snake\_case, numbered when they repeat, and kept in their original form as a `label`.
- Wavelength names take as many decimal places as are needed to be unique.

**Revised in version 44.** The symmetry review removed the aliases and the two-part short form. A Slice is always named in three parts.

### R13. There is no threading model and no error model

**Status:** Resolved in version 48 · **Where:** sections 2, 10 and 12

**Problem.** Nothing says which objects may be used from several threads. Decoder state and held metadata both change during calls declared as read-only.

Section 12 requires parallel rows and tiles. A GDAL dataset handle cannot be shared between threads, and the HDF5 C library runs one call at a time when built for threads. The specification faults a wrapper crate for a lock that the library underneath it also has.

Errors are not covered. Nothing says whether a corrupt tile is an error or missing data. The C example returns no status. A Rust panic must not cross the C interface. The plugin interface has no version check.

**Proposed resolution.**

- A Source and an Image may be shared between threads. A reader object owned by each thread holds library handles and decoder state.
- A driver declares its concurrency: `Parallel`, `OneHandlePerThread` or `Serial`. `plan_slice` reports it.
- Every call returns a result. A corrupt block is an error by default, with an option to return it as missing data and report it.
- C calls return a status code. Panics are caught at the interface. `bil_register_driver` carries an interface version.

**Decision needed.** Whether a corrupt block is an error or missing data by default.

**Note.** Version 25 of the specification introduced the Reader for R3. The rest of this finding is still open.

**Part settled in version 44.** The error model is now in section 3.3 of the specification: a call delivers or returns an error, everything delivered carries one status, corrupt data is an error by default, a failure is caught at the interface, and C functions return a status code. Still open: which objects may be shared between threads, how a driver declares its concurrency, and a version check on the plugin interface. The details added in version 44 were confirmed, including corrupt data as an error by default.

**Ruling.** Accepted as proposed.

**What changed in the specification.** Version 48, sections 2, 4, 10 and 12.

- A Source, an Image, a Fragment and a Slice may be shared between threads. A Reader belongs to one thread at a time and may be handed to another.
- Each driver and codec declares `concurrency` as `Parallel`, `OneHandlePerThread` or `Serial`, and `plan_slice` reports it. Under `Serial` only the library call is locked, and conversion stays parallel.
- The test for wrappers no longer counts a lock the library itself requires, so hdf5-metno is not faulted for HDF5's own limit.
- `bil_register_driver` carries the plugin interface version. A different major version is refused and shown by `list_drivers` as not loaded.
- The C example now returns a status from every call.

### R14. Results depend on what is installed

**Status:** Resolved in version 48 · **Where:** section 10

**Problem.** BIL falls back to another codec when a file is outside the first one's profiles. Lossy decoders round differently, so the same file gives different pixel values on different machines. The failure may come part-way through an image, leaving a mix.

`probe` works from the file name and leading bytes. DNG, GeoTIFF, whole-slide and many raw camera files are all TIFF underneath. NetCDF-4 and several SAR products are HDF5. DICOS is DICOM. Ties are certain, and the specification does not break them.

**Proposed resolution.**

- Choose the codec when the Source is opened, and never switch during its life.
- Report the driver, codec and their versions for every Source, and record them in anything BIL writes.
- Require exact agreement between codecs for lossless data, as a conformance test. State a tolerance for lossy data.
- Probe in two stages: the container first, then its content. Break ties by a configured order.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 48, sections 10 and 11.

- A codec accepts a Source only after checking the coding parameters the file declares. The driver and codec are chosen when the Source is opened and kept for its life. A block the codec cannot decode is corrupt data, never a switch to another codec.
- `describe_decoding()` reports the driver and codec with their versions, and everything BIL writes records them as `bil.driver` and `bil.codec`.
- A conformance test, Codec agreement: identical values for lossless data, and at most one unit of the stored value from the reference decoder for lossy data.
- Probing runs in two stages, the container and then its content, and ties follow the installation's configured order, never load order.

### R15. BIL's libraries will meet other copies in the same process

**Status:** Resolved in version 34 · **Where:** sections 2 and 10

**Problem.** A Python process that imports BIL will often already hold GDAL, HDF5 or FFmpeg, loaded by rasterio, h5py or a video package. Two versions of one C library in a process can crash or corrupt data. HDF5 is the best-known case.

**Proposed resolution.** Three options, which can be combined.

1. Build the libraries into BIL with their symbols hidden.
2. Make the default build mostly Rust (async-tiff, hidefix, nitf-rs, emuella-j2k), with the C libraries as optional plugins.
3. Run C library drivers in the decoder process, as in R6.

**Decision needed.** Whether the default build should avoid C libraries where a Rust reader exists.

**Ruling.** Keep the libraries. BIL is to work for its formats without the user having to set things up, and clashes with outside use are to be minimized. This is option 1, with option 3 as the fallback. Option 2, a mostly Rust default build, is not taken.

**What changed in the specification.** Version 33, new section 10.5.

- The standard package carries its backing libraries already built, so installing BIL is enough.
- Each library is a private copy with hidden symbols, its own data files and no use of environment variables.
- BIL makes no changes to process-wide settings, and its libraries load no plugins of their own.
- Each release is tested in one process beside the common packages that carry the same libraries.
- A library that cannot be kept private runs in the decoder process.
- The costs are stated: a large package, and a build of every library for every supported platform.

**Further ruling.** Where a Rust build of a library is easily available, BIL is not to duplicate it. Where a library can come from outside through standard package management with no loss, BIL uses that.

**What changed in the specification.** Version 34, section 10.5 rewritten.

- Each library is taken from the first source that works without loss: an existing Rust crate, then the installing package manager's shared copy, then a build of BIL's own as the last resort.
- "Without loss" is defined: the user installs in one step, the library cannot clash, and the four wrapper tests pass.
- Clashes are avoided in one of two ways: one shared copy, or a private copy with hidden symbols.
- The rules on environment variables, process-wide settings and plugin loading hold whichever source is used.
- BIL no longer takes on a build of every library. This also removes the conflict with the earlier decision that BIL does not build FFmpeg.

### R16. Geocoding re-implements rigorous sensor models, with no validation target

**Status:** Resolved in version 41 · **Where:** section 13

**Problem.** `camera` with a `pose` per row, and `range_doppler`, mean BIL computes what SIX, Community Sensor Model plugins and OSSIM already compute and have validated. Orbit and attitude interpolation and each supplier's conventions are easy to get slightly wrong. Nothing says how BIL's answers are checked.

Inverting by iteration can fail. A line of sight can meet steep terrain more than once. Coordinate grids that fold, or cross the antimeridian or a pole, have no single inverse.

**Proposed resolution.**

- Name a reference implementation and a tolerance for each step, as conformance tests: GDAL for `rational_polynomial`, SIX for `range_doppler`.
- Prefer `external` steps for rigorous models at first, and keep BIL's own steps to the simple forms.
- `intersect` returns the first intersection from the sensor.
- An inverse that does not converge, or is ambiguous, says so instead of returning a position.

**Decision needed.** Whether rigorous models go through `external` steps in the first tier.

**Ruling so far.** Prefer testing the rigorous models over handing them to `external` steps.

**What changed in the specification.** Version 40, new section 13.8.

- BIL implements rigorous sensor models itself. An `external` step stays available but is not the default route.
- Each step has a named reference to test against: GDAL for `rational_polynomial`, SIX for `range_doppler`, and others.
- Four tests apply to every step: agreement with the reference, round trip, the same position through any Window or Level, and known ground points.
- The driver conformance suite gains a geocoding test.

**Still to decide.** This finding stays open for three points.

1. Whether `intersect` returns the first intersection from the sensor when a line of sight meets the terrain more than once.
2. What an inverse returns when it does not converge or has more than one answer.
3. What BIL does with coordinate grids that fold, or that cross the antimeridian or a pole.

**Final ruling.** All three accepted as suggested.

**What changed in the specification.** Version 41, section 13.3.

- A line of sight stops at the first surface it meets, counting from the sensor.
- Every position carries a status: `Resolved`, `Unresolved`, `NotConverged` or `Ambiguous`. An inverse with no single answer returns a status and no position.
- Longitude differences are taken the short way round, and a coordinate grid is interpolated as positions on the sphere, so chains work across the antimeridian and over a pole. A fold in a grid reports `Ambiguous`.
- The validation tests gain one for honest failure.

**Revised in version 44.** The statuses are now the single set of section 3.3, so `Resolved` became `Complete` and `Unresolved` became `Missing`.

### R17. Metadata has six holes

**Status:** Resolved in version 48 · **Where:** section 14

**Problem.**

1. **Names.** Native names contain dots, as HDF5 paths and XML do. They also repeat: several instances of one NITF extension, the items of a DICOM sequence, FITS `HISTORY` lines.
2. **Searching.** `find_fields` across all groups forces the full extraction that on-demand reading exists to avoid.
3. **Holding.** Per-frame fields held for a live stream grow without bound, against the bounded memory rule.
4. **KLV timing.** KLV packets are not aligned to frames, and often arrive less often.
5. **Text and dates.** Formats declare their own character sets, and many dates carry no time zone.
6. **My own example.** `common.collection_time` maps Exif time beside NITF time. Exif time has no time zone, so the mapping fails the "obviously the same" test.

**Proposed resolution.**

- A name is a list of parts, not a dotted string. The dotted form escapes dots, and repeats carry a number.
- `find_fields` searches defined names and groups already extracted, unless the caller asks it to extract.
- Fields attached to an index are held up to a limit the caller sets.
- `read_field_at` for a frame returns the nearest packet at or before the frame's time, with the packet's own time.
- Text is decoded to Unicode by the format's declared character set, with the raw bytes still available. A date and time states whether its zone is known.
- Remove Exif from the `common.collection_time` row.

**Note.** Version 45 of the specification lists Exif time as a way to derive `common.collection_time`, with its time-zone offset, and not as a value the format provides. The rest of this finding is still open.

**Decision needed.** Accept, or rule on each part.

**Ruling.** Accepted, except the last part: Exif stays in the `common.collection_time` row. Its time is taken as local time, and where there is a sensor location or a footprint, the time zone is computed from it.

**What changed in the specification.** Version 48, sections 14.1 to 14.4.

- A field name is a list of parts. In the dotted form a dot inside a part is written `\.`. Repeats carry a number from 1 in square brackets, as in `nitf.RPC00B[2]`.
- `find_fields(pattern, extract)` searches the defined names and the groups already extracted, and extracts everything first only when asked.
- Fields attached to an index are held for at most `max_held_positions` positions, 1,000 by default, oldest released first.
- `read_field_at` returns, for a field with its own times such as KLV, the latest value at or before the frame's time, with that value's time.
- Text is decoded to Unicode from the declared character set, with the stored bytes still available. A date and time states whether its zone is known, and a time with no zone is never taken as UTC.
- Exif `DateTimeOriginal` takes its zone from `OffsetTimeOriginal`, then the sensor location, then the footprint center. The zone comes from a time-zone boundary map, and the offset for the date from the IANA time zone database. With none of these, the time stays local and marked as having no known zone.

**Follow-up in version 49.** tzf-rs is not a requirement. Where a time-zone lookup plugin over it is installed, BIL uses it to find the zone from the sensor location or footprint. Where none is installed, the time is taken as local time. The lookup is an optional plugin outside BIL's packages, because its boundary map is under the Open Database License.

### R18. The license position needs legal review

**Status:** Resolved in version 35 · **Where:** sections 1.2 and 10

**Problem.** I am not a lawyer, and these are questions to put to one.

1. **Static linking.** Rust links statically by default. LGPL 2.1 requires that a recipient can relink the application against a changed copy of the library.
2. **Apache-2.0 dependencies.** emuella-j2k and object\_store are Apache-2.0 only. The Free Software Foundation holds that license to be incompatible with GPL version 2, and whether such code can be built into a library that is LGPL "2.1 only" is unsettled. "2.1 or later" would avoid the question.
3. **FFmpeg.** It is LGPL or GPL depending on how it was built. A wrapper that links the system's FFmpeg may link a GPL build.
4. **Bio-Formats.** It is GPL. Running it as a separate process is the usual way to stay clear, but that should be confirmed.

**Proposed resolution.**

- Decide between "2.1 only" and "2.1 or later".
- Ship BIL as a shared library with the C interface, and a thin Rust crate over it, so relinking is always possible.
- `list_drivers` reports the license each linked library was actually built under.

**Decision needed.** "2.1 only" or "2.1 or later".

**Ruling.** LGPL 2.1 or later. Any clash with another license that a user installs is the user's, not BIL's. The principle is to leave users free to make their own decisions and installations, while BIL itself is shared. BIL is a Rust library with a C-callable interface over it, delivered as a shared library. BIL's own packages hold only LGPL-compatible builds, and any move away from that is to be flagged. `list_drivers` reports the license each library was built under.

**What changed in the specification.** Version 35, new section 10.6.

- The license is LGPL 2.1 or later.
- BIL is delivered as one shared library: the Rust library with a C-callable interface over all of it. The C-callable interface is now part of Tier 1.
- A table states who answers for what: the BIL project for its own packages, and the user for what they install and pass on.
- BIL's packages contain no GPL or AGPL code, use the permissive build of each library, carry notices, and are checked at every release.
- `list_drivers` reports each library's name, version, source and license as built, or `Unknown`.
- A legal review before the first release is a new open decision. It covers whether "or later" allows Apache-2.0 and LGPL 3 components inside a package.

**What stays open from this finding.** Patents on video codecs are untouched by any of this and remain the user's concern.

## Lesser findings

These are worth fixing, but none blocks the design.

### R19. Some slice functions are image processing

**Status:** Resolved in version 53 · **Where:** sections 1.2 and 9

**Problem.** Section 1.2 puts image processing out of scope. `color.lab`, `spectral.indices`, `spectral.angle` and `pol.pauli` are image processing. `nuclear.uptake` computes a clinical quantity that has several definitions, from fields that are often wrong in practice.

**Proposed resolution.** Split the catalog into a core set and an extras package. Name the definition `nuclear.uptake` uses, or remove it. State that BIL is not for diagnostic use.

**Decision needed.** Where the line between core and extras falls.

**Ruling.** BIL may do the processing that prepares Slices, so that every sensor's data can be treated uniformly. Sensors store data in a way that suits the sensor, and BIL prepares it in a way that supports analysis. Decompositions are included, because color, polarimetry and spectral peaks are almost never analyzed in their sensed form. `insar.interferogram` is kept. Functions that go out go to an extras package.

**What changed in the specification.** Version 53, sections 1.2, 8.5, 9 and 10.

- The rule: a slice function belongs in BIL when it undoes the storage, calibrates, or re-expresses the measurement. Re-expressing must use the data's own values, have a standard definition, be general to the data type, and be local.
- `color.hsv`, `color.lab`, `color.ycbcr`, `polcam.linear`, `pol.pauli`, `pixel.peak` and `class.members` stay in the core.
- `insar.interferogram` stays, as a stated allowance, where its two Images are declared related in one Source.
- `spectral.indices`, `spectral.angle` and `nuclear.uptake` move to `bil-extras`, with the same names and the same mechanism.
- `nuclear.activity`, in becquerels per milliliter, joins the core as the calibration step. `nuclear.uptake` states its definition: SUV normalized by body weight.
- Section 1.2 states that BIL is not for diagnostic use.

### R20. Color is under-specified

**Status:** Resolved in version 52 · **Where:** section 9.3

**Problem.** `color.ycbcr` does not say which matrix or range. `color.lab` needs a white point and linear values. `color.hsv` does not say whether it works on encoded or linear values. Color profiles are not mentioned.

**Proposed resolution.** Each color function states its transfer function, matrix, range and white point, taken from metadata with a stated default. A color profile is exposed as a field.

**Decision needed.** Accept.

**Part settled in version 51.** Color is described by the four properties of ITU-T H.273: primaries, transfer curve, matrix and range. Each comes from metadata or a stated default, and is computed by formula in BIL's own loops, with no per-pixel lookups. Color profiles are exposed as fields and not applied, because color management is a layer above BIL. Little CMS moves from Adopt to Reference. Still open: whether `color.hsv` works on encoded or linear values.

**Ruling.** Accepted in part, as set out above. Color profiles are not applied: color management is the layer above BIL, or not dealt with at all, because the imaging systems BIL serves rarely describe their color to that fidelity. No per-pixel lookups. `color.hsv` works on the encoded values as stored.

**What changed in the specification.** Versions 51 and 52, sections 9.3, 14.2 and Appendix A.

- Every color function uses the four H.273 properties, from metadata or a stated default, and `describe_field` reports which.
- Matrices and transfer curves are computed by formula in BIL's own loops.
- An embedded ICC profile is a field, unaltered and not applied.
- `color.hsv` is computed from the encoded values as stored.

### R21. NaN spreads through downstream code

**Status:** Resolved in version 50 · **Where:** section 5

**Problem.** One NaN in the input makes the whole output of a Fourier transform NaN, and spreads through every convolution. The specification says float32 is what such code expects, but that code does not expect NaN.

**Proposed resolution.** Keep NaN as the default. Add a `missing_value` to `SliceOptions` that fills missing positions with a number the caller chooses.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 50, section 5.

- NaN stays the default for missing positions.
- `missing_value` in `SliceOptions` fills them with a number the caller chooses.
- The fill is applied to the finished Slice, after the slice function and the Level, so averaging still leaves missing values out.
- The status stays `Partial`, and `pixel.mask.valid` in the same request says which positions were filled.

### R22. A cut across frames costs one read per frame

**Status:** Resolved in version 44 · **Where:** sections 3.2 and 8.4

**Problem.** A Window lies in one frame. A cut through a medical volume along another axis, or one pixel's history through a video, needs a separate read for every frame.

**Proposed resolution.** Either add a read that takes a run of frames and returns a stack, or state that such cuts are outside BIL.

**Decision needed.** Add the read, or rule it out.

**Ruling.** Ruled out, as part of the symmetry review. Everything beyond 2D is selected, never ranged, so a read across frames is outside BIL and a caller reads frame by frame.

**What changed in the specification.** Version 44, section 1.6. The Fragment cache and the per-thread Reader keep frame-by-frame reading cheap.

### R23. The bilinear filter lets through aliasing

**Status:** Resolved in version 49 · **Where:** section 5

**Problem.** The 1 2 1 weights still pass half the amplitude at the highest frequency the reduced grid can hold, and content above that folds back into the result. Wider filters do better.

**Proposed resolution.** Keep `bilinear` as the default, as decided, since it gives every level in one pass. Record the limitation, and make a five-tap binomial the first extension method.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 49, sections 5 and 10.

- The limit of `bilinear` is recorded: it passes half the amplitude at the highest frequency the halved grid holds, so finer detail aliases. It stays the default.
- `binomial5`, with weights 1 4 6 4 1 divided by 16 and a radius of 2 pixels, is built in as the first added method. It passes a quarter of the amplitude at that frequency and still gives every level in one pass.

### R24. The naming rule is broken by its own calls

**Status:** Resolved in version 39 · **Where:** section 2

**Problem.** Section 2 says functions start with a verb. `Image(0)`, `LevelCount`, `DefaultSlice`, `ForWindow`, `HeightAt`, `Coverage` and `ImageToWorld` do not. The interface also mixes spellings: `Color` beside `ReadNormalisation`.

**Proposed resolution.** Either rename the calls (`OpenImage`, `CountLevels`) or relax the rule. Choose one spelling for identifiers.

**Decision needed.** Rename or relax, and British or American spelling in identifiers.

**Note.** Version 25 of the specification made fields, parameters and options strictly UpperCamelCase. The verb rule and the spelling are still open.

**Ruling.** Change everything to snake\_case, for the sake of acceptance by Rust developers, and use American spelling throughout.

**What changed in the specification.** Version 38, section 2 rewritten.

- Names follow each language's own convention. Functions, fields and parameters are snake\_case, and types and enum values stay UpperCamelCase, which is what Rust and Python both expect.
- C uses a `bil_` prefix on functions and a `Bil` prefix on types.
- Names that are data keep their form in every language: slice functions, Slice names, level names and metadata field names.
- The rule that functions start with a verb is gone. Its purpose was to keep functions apart from types when both were UpperCamelCase, and the casing now does that. A call that returns a property is named for what it returns.
- Spelling is American in every identifier and throughout the specification, so `color` and `read_normalization` now agree.
- The names in the problem statement above are left as they were when the finding was written.

**Further ruling.** A mixture of snake\_case and UpperCamelCase in different areas is not acceptable. Make it all consistent.

**What changed in the specification.** Version 39.

- There is now one rule: types and enum values are UpperCamelCase, and every other name is snake\_case.
- The names passed as text, which version 38 had left in UpperCamelCase, are now snake\_case: slice functions and Slice names such as `color.hsv.hue`, level names such as `stored2`, level methods such as `bilinear`, geocoding steps such as `affine`, and normalization functions such as `gain_offset`.
- Types and enum values stay capitalized because Rust and Python require it, and the Rust compiler warns otherwise.
- The native part of a metadata field name is still kept as its format spells it.

### R25. Some performance claims overreach

**Status:** Resolved in version 49 · **Where:** sections 6 and 12

**Problem.**

1. **One loop per pair.** Sample types, slice functions, instruction sets, byte orders and normalizations multiply into thousands of loops.
2. **One copy.** A decoder always holds its own output buffer, so "one full-size copy" for compressed data leaves that out.
3. **Benchmarks that fail the build.** Timing on shared build machines varies, so this gate will fail at random.

**Proposed resolution.**

- Specialized loops for the common pairs. The rest convert to a float32 row buffer and then map, and the specification says so.
- State the claim as one copy beyond the decoder's own buffer.
- Track benchmarks on dedicated hardware, with thresholds, outside the build.

**Decision needed.** Accept.

**Note.** Version 32 of the specification limits specialized loops to the six common sample types, which answers item 1. Items 2 and 3 are still open.

**Ruling.** Accepted.

**What changed in the specification.** Version 49, sections 6 and 12.

- Copy counts are stated as copies BIL makes beyond the decoder's own buffer. A decoded block kept in the Fragment cache is that buffer, not a further copy.
- Benchmarks run on dedicated machines, outside the build. Each has a threshold, and a regression past it must be fixed or explained before a release.

## What held up

These parts were attacked and stood. Version 43 of the specification states them as its foundations, in section 1.5.

- **Index and coordinate.** Addressing by index and describing by coordinate is consistent everywhere it is used.
- **Window independence for pointwise functions.** The rule is sound. The gaps in R9 concern levels and block functions only.
- **Containers and codecs.** Splitting them is right, and it is what lets one JPEG 2000 codec serve every container.
- **Geocoding as a chain.** Every format's model examined fits the form. The risk in R16 is in the content of the steps, not the chain.
- **Scripted drivers that describe pixels.** Keeping the interpreter off the pixel path holds together.
