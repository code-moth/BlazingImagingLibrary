This is the second review of the Blazing Imaging Library specification, made against version 56. It has two parts.

- **Symmetry and consistency (U1 to U16).** Places where the design does one thing in two ways, names one thing twice, or says something an earlier decision has since overtaken. These test rule 7, Uniform, and the two tests of section 1.6.
- **Red team (R26 to R41).** Weaknesses that would cause wrong results, unsafe behavior or failed adoption. Numbering continues from the first review, which ended at R25.

Each finding gives the problem, a proposed resolution and the decision it needs. The review was made on paper: nothing was run. Claims about outside formats and libraries are from knowledge of them, and say so where it matters.

## Summary

{{SUMMARY}}

## Symmetry and consistency

### U1. Repeated names are numbered three different ways

**Status:** Resolved in version 57 · **Where:** sections 5 and 14.1

**Problem.** Three rules number things that repeat, and no two agree.

| Where | Rule | Example |
| --- | --- | --- |
| Slice names from metadata (section 5) | The first is unnumbered, later ones gain `_2`, `_3` | `red`, `red_2` |
| Field names (section 14.1) | Every instance is numbered, from 1, in brackets | `nitf.RPC00B[1]`, `nitf.RPC00B[2]` |
| Slice positions (section 5) | Counted from 0 | Position 0 is `red` |

A caller who learns one rule guesses wrong for the others. A Slice name also changes meaning when a second band of the same name appears: `red` stops being the only red band but keeps its name, while a field in the same situation becomes `[1]`.

**Proposed resolution.** One rule for repeats, everywhere: when a name repeats, every instance carries its number, counted from 1. Slice names take the form `red_1`, `red_2`, and field names keep their brackets, because a suffix could clash with a native name. Positions stay counted from 0, as indexes are everywhere else in BIL, and the specification says that positions are indexes and repeat numbers are not.

**Decision needed.** Whether the first instance of a repeated Slice name is numbered.

**Ruling.** Accepted. Every instance of a repeated Slice name is numbered.

**What changed in the specification.** Version 57. Section 5: repeated names become `red_1`, `red_2`, as field names already are in brackets. Positions are indexes and count from 0; repeat numbers count from 1.

### U2. One quantity has two names, and one name has several meanings

**Status:** Resolved in version 57 · **Where:** sections 9.3, 9.4 and 9.5

**Problem.**

- **Chroma is named twice.** `color.ycbcr` yields `chroma_blue` and `chroma_red`. `video.planes` yields `chroma_u` and `chroma_v`. They are the same quantities.
- **`luminance` means four things.** In `color.luminance` it is linear luminance from the declared primaries. In `cfa.demosaic` and `cfa.binned` it comes from camera color with no primaries known. In `palette.lookup` the specification does not say. A caller who mixes them compares unlike values under one name.
- **Units appear in one name only.** `thermal.calibrated` yields `temperature_k`. Every other Slice leaves the unit out: `height` in meters, `range` in meters, `activity` in becquerels per milliliter, and `spectral.brightness_temperature` in kelvin.

**Proposed resolution.**

- Use `chroma_blue` and `chroma_red` in both functions.
- Give `luminance` one definition: linear luminance from the declared primaries, using the defaults of section 9.3 where none is declared. Every function that yields `luminance` follows it.
- Rename `temperature_k` to `temperature`, and state every Slice's unit in its definition, never in its name.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. Sections 5, 9.3 and 9.4: `video.planes` yields `chroma_blue` and `chroma_red`; every `luminance` is linear luminance from the declared primaries; `temperature_k` becomes `temperature`, and units are stated in definitions, never in names.

### U3. Decibels are reached three ways

**Status:** Resolved in version 57 · **Where:** sections 7 and 9.2

**Problem.** `complex.power` yields `db` as a Slice beside `power`. `sar.calibrated_db` is a separate function, and it has no `rcs` although `sar.calibrated` does. The `log10` normalization, with a `gain_offset` after it, makes decibels from anything. A caller cannot predict which form a quantity in decibels takes.

**Proposed resolution.** One form: a decibel Slice sits beside its linear Slice in the same function, named with `_db`. `complex.power` yields `power` and `power_db`. `sar.calibrated` yields `sigma0`, `sigma0_db` and the rest, including `rcs_db`, and `sar.calibrated_db` is removed. `log10` stays as a normalization for formats that store values on a log scale.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. Sections 5, 9 and 9.2: `complex.power` yields `power` and `power_db`; `sar.calibrated` yields each quantity and its `_db` form, including `rcs_db`; `sar.calibrated_db` is removed. `log10` stays for formats that store values on a log scale.

### U4. Color conversion has two routes, and one uses lookups

**Status:** Resolved in version 57 · **Where:** sections 7 and 9.3

**Problem.** Section 9.3 now says color is computed by formula from the four H.273 properties, with no per-pixel lookup tables. Section 7 still lists the `matrix` normalization for "color space conversion, luminance", with "RGB to luminance" taking its matrix from "color space in metadata", and `lookup_table` for tone curves. That is a second route to the same result, with its own source of parameters, and one of them is a per-pixel lookup.

**Proposed resolution.**

- Color conversion is done only by the color functions of section 9.3. Remove "color space conversion" and "luminance" from the uses of `matrix`, and the "RGB to luminance" motivating case. `matrix` remains for band synthesis.
- `lookup_table` stays only where the format stores values through a table: palettes and amplitude tables. Those are the stored values, not a color conversion, and section 9.3 says so.

**Decision needed.** Whether `lookup_table` stays for tone curves.

**Ruling.** Accepted. `lookup_table` stays only for values the format stores through a table.

**What changed in the specification.** Version 57. Sections 7 and 9.3: color is converted only by the color functions. `matrix` is for band synthesis, the RGB-to-luminance case is removed, and tone curves leave `lookup_table`.

### U5. "What a function needs" has two mechanisms

**Status:** Resolved in version 57 · **Where:** sections 7, 10 and 14.2

**Problem.** A function's inputs reach it two ways.

| Inputs | How they arrive | Precedence | Companion file |
| --- | --- | --- | --- |
| Normalization parameters | `read_normalization`, a driver duty | Caller, then metadata | Cannot supply them |
| Dictionary items | `list_required_fields` | Caller, provided, derived, companion | Can supply them |

A SAR calibration polynomial missing from a file therefore cannot be supplied in a companion file, while a sun elevation can. Two lists say why a function is held back, and they are reported by different calls.

**Proposed resolution.** Normalization parameters become fields like any other input. A driver maps them from native fields, as it maps dictionary items, and they follow the same order: caller, provided, derived, companion. `read_normalization` remains as the driver's way of supplying them. `list_required_fields` lists every input a function needs, of either kind.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. Sections 7, 10 and 14.2: normalization parameters are fields, filled in the dictionary's order, so a companion file can supply them. `list_required_fields` lists every input.

### U6. Wavelength is held twice

**Status:** Resolved in version 57 · **Where:** sections 3.1, 9.1 and 14.2

**Problem.** A band's wavelength is a coordinate attached to the component index (section 3.1), which `pixel.nearest` and `pixel.integrate` read. It is also the dictionary item `common.band_center` (section 14.2). Nothing says they are the same, so a driver could fill one and not the other.

**Proposed resolution.** State the general rule: a coordinate is the view along its index of a field, and where a dictionary item describes the same thing, the coordinate's field is that item. `common.band_center` is the wavelength coordinate of the component index. The same holds for frame time and `common.collection_time` at frame 0.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. Section 3.1: where a dictionary item describes the same thing as a coordinate, the coordinate's field is that item: `common.band_center` for wavelength, `common.collection_time` for frame time at frame 0.

### U7. "Borrowed" and "Streamed" each mean two things

**Status:** Resolved in version 57 · **Where:** sections 4, 5, 6 and 8.3

**Problem.**

- **Borrowed.** Section 4 defines a borrowed Fragment as "the stored bytes themselves". Section 8.3 says video Fragments are "borrowed from the decoder", while section 4 says they are deferred.
- **Read-only.** Borrowed pixels "cannot be changed", yet `Private` mapping exists "so a caller can change the array without touching the file".
- **Streamed.** `Streamed` is an access class in section 6 and a `Delivery` in section 5. The access class depends on what the caller chose to do, not on the data.

**Proposed resolution.**

- Keep `Borrowed` for stored bytes only. Change the column in section 8 to say "Deferred; the decoder's buffer is used without a copy".
- Under `Private` mapping, borrowed arrays are writable, and writes stay in the caller's memory. Under `Shared`, they are read-only. The read-only rule names `Shared`.
- Make the access classes describe the source only: `Borrowed`, `Converted` and `Decoded`. Delivery stays a separate choice, and `plan` reports both.

**Decision needed.** The names of the access classes.

**Ruling.** Accepted, with the access classes `Borrowed`, `Converted` and `Decoded`.

**What changed in the specification.** Version 57. Sections 4, 6 and 8: `Borrowed` means stored bytes only, and video is shown as deferred; borrowed arrays are read-only under `Shared` mapping and writable under `Private`; access classes describe the data, and delivery is a separate choice.

### U8. Resolution is chosen three ways, and geocoding takes a fourth

**Status:** Resolved in version 57 · **Where:** sections 5 and 13.4

**Problem.** A request chooses its resolution through `levels`, a list of whole numbers, through the `stored_level` option, and through `level_source`. The level names a Slice reports, such as `stored2.level1`, cannot be asked for by name. Geocoding then takes a Window and a Level number in `for_window(window, level)`, which cannot express a stored level or a `step`, although every Slice now carries the `origin` and `scale` that say exactly where it sits.

**Proposed resolution.**

- `levels` in a `SliceRequest` takes level names: `level3`, `stored2`, `stored2.level1`. A whole number is accepted as a short form of `levelN`. `stored_level` is removed, and `level_source` stays only to let BIL choose a stored level itself.
- Replace `for_window(window, level)` with `for_slice(slice)`, which builds the `image_window` step from the Slice's `origin` and `scale`. `for_window` remains as a short form for computed Levels.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. Sections 5 and 13.4: a request's `levels` takes names such as `stored2.level1`, with a number as short form; the `stored_level` option is removed; `for_slice(slice)` builds the geocoding chain from the Slice, and `for_window` remains as a short form.

### U9. "No value" is said two ways

**Status:** Resolved in version 57 · **Where:** sections 3.3 and 14

**Problem.** Section 3.3 says a call either delivers or returns an error. It also gives fields a `Missing` status, "the field has no value at that index", and a derived item with a missing input is `Missing`. Nothing says what `read_field` does with a name the file does not hold: an error, or a delivered value with status `Missing`.

**Proposed resolution.** One rule. A name that is not defined for this Source is an error. A defined field, or a dictionary item, that has no value here is delivered with status `Missing` and no value. The same applies along an index with `read_field_at`.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. Sections 3.3 and 14.4: an unknown name is an error; a known field with no value is delivered as `Missing`.

### U10. Some definitions still describe an earlier design

**Status:** Resolved in version 57 · **Where:** sections 3, 5 and 7

**Problem.**

- **Slice function.** Defined as a mapping "from a Fragment", and section 5 says "from a single Fragment". Several functions now read Fragments from more than one Image.
- **Normalization.** "Uses only the pixel's values and its absolute row and column." It also uses frame coordinates and calibration Images.
- **Representation.** A slice function is "defined for one sensor representation", and `list_slice_functions` offers what is valid for an Image's representation. Nothing defines a representation or says how a driver declares one: color, Bayer pattern, complex, spectral or polarimetric.
- **Level.** Section 3 says each Level "halves the rows and columns". Since version 46 a Level is the grid points at multiples of 2ⁿ.

**Proposed resolution.** Rewrite the four definitions to match the design. For representation, add it to what `list_images` describes: a named representation for each Image, with its parameters, such as the mosaic pattern, the polarizer angles or the band coordinates. A slice function declares the representations it accepts.

**Decision needed.** Accept, including how a representation is declared.

**Ruling.** Accepted, with the representation declared by the driver for each Image.

**What changed in the specification.** Version 57. Sections 3, 5, 7 and 10: the definitions of Slice function, Level and normalization match the design, and Representation is defined and listed by `list_images`.

### U11. Plugins register in more than one way, under different rules

**Status:** Resolved in version 57 · **Where:** sections 9.6 and 10

**Problem.**

- **Families.** `register_slice_function` adds functions "under the driver's own family". `bil-extras` uses the same call to add `spectral.indices`, `spectral.angle` and `nuclear.uptake` to the core families.
- **A bypass.** "A driver does not implement slice functions", yet `read_slice_direct` lets a decoder emit float32 rows itself. Nothing requires those rows to equal the core's, or to stay window-independent.
- **Several registrations.** Section 10 says all plugin kinds "register through the same mechanism". There are at least four calls: `bil_register_driver`, `register_slice_function`, `register_level_method` and the scripted `register_driver`.

**Proposed resolution.**

- A family belongs to whoever defines it. The core families are open to packages from the BIL project, and other plugins use their own families.
- `read_slice_direct` is allowed only for a slice function whose result it reproduces exactly, and the Codec agreement test checks it against the core's own path.
- One registration call that takes a description of the plugin and its kind. The specific calls become short forms of it.

**Decision needed.** Whether `read_slice_direct` is kept.

**Ruling.** Accepted. `read_slice_direct` is kept, only where it reproduces the core's result exactly.

**What changed in the specification.** Version 57. Sections 9, 9.6 and 10: families belong to whoever defines them, with the core open to packages of the BIL project; one call, `register_plugin`, with the others as short forms; `read_slice_direct` checked by the Codec agreement test.

### U12. Memory is bounded by several unrelated limits

**Status:** Resolved in version 57 · **Where:** sections 4, 10.3, 12, 13.6 and 14.3

**Problem.** "Bounded memory" is a rule of section 12, but the limits that enforce it are counted in different units, and some are missing.

| What is held | Limit |
| --- | --- |
| Decoded Fragments | A count, 16 by default, with no limit on bytes |
| Metadata attached to an index | A count of positions, 1,000 by default |
| Extracted metadata groups | None |
| Terrain tiles | "Recently used tiles are kept", with no limit |
| Frame index | None |

The options that set them are scattered: `OpenOptions` gathers limits, mapping, isolation, the companion file and held positions, but is never listed in one place.

**Proposed resolution.**

- Each kind of held data has a stated limit and a report of what it holds, in the manner of `describe_cache`.
- Terrain tiles are read through BIL's own drivers, so they use the Fragment cache of their Sources rather than a cache of their own.
- `OpenOptions` and `WriteOptions` are each listed in full, in one place, with every default.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. New section 3.4 lists `OpenOptions`, `SliceOptions` and `WriteOptions` with every default, and gives each kind of held data a limit and a report through `describe_held`. Terrain tiles use the Fragment cache.

### U13. Where a field can attach is listed three ways

**Status:** Resolved in version 57 · **Where:** sections 3.1, 14.1 and 14.2

**Problem.** Section 14.1 says a field attaches to "a Source, an Image, or an index of an Image, such as frame". Section 14.2 adds "a component" for dictionary items. Section 3.1 attaches coordinates to row, column, component and frame. The three lists do not agree.

**Proposed resolution.** One list in section 14.1: a field attaches to a Source, an Image, or one index of an Image, whether row, column, component, frame or a declared additional index. The other sections refer to it.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. Section 14.1 holds the one list of attachments: a Source, an Image, or one index, whether row, column, component, frame or an additional index.

### U14. The catalog rule and the catalog disagree

**Status:** Resolved in version 57 · **Where:** sections 9 and 15

**Problem.**

- **Caller-supplied values.** The first check bans anything from outside the collection, including an "analyst's threshold". `pixel.weighted(weights)` and `spectral.response(curves)` take values from the caller, and stay in the core.
- **A generic index.** `pixel.pair` yields `normalized_difference`, which is NDVI for any two bands, while `spectral.indices` moved to the extras package as analysis.
- **Display stretch.** Section 15 allows a display stretch as a slice function when its statistics are passed in. It neither undoes storage, calibrates nor re-expresses.
- **The interferogram.** The allowance says `insar.interferogram` "combines two collections". Section 8.2 describes the interferogram as already formed, one complex Image with its coherence, and section 15 puts the forming of interferograms outside BIL. The function only re-expresses a product already made.
- **Points from range.** `depth.points` computes positions "from its range and the camera geometry", outside the geocoding chain that section 13 says locates every pixel.

**Proposed resolution.**

- Add to the first check: parameters the caller names explicitly, such as weights or response curves, are allowed. They are part of the request, not information brought in by BIL.
- Keep `pixel.pair` as a general tool of the `pixel` family, and say that the named indices stay in the extras package.
- Remove the display-stretch row from section 15, or name it as a fourth kind, "present", with its own check.
- Replace the interferogram allowance with the plain statement that the function re-expresses a formed interferogram.
- `depth.points` takes its geometry from the Image's own `camera` step.

**Decision needed.** Whether a display stretch belongs in BIL.

**Ruling.** Accepted, keeping the display stretch.

**What changed in the specification.** Version 57. Sections 9, 9.1, 9.2, 9.5 and 15: caller-named parameters pass the first check; `pixel.pair` is a general tool; a fourth kind, Present, holds a display stretch, with `pixel.stretch(low, high)`; the interferogram allowance becomes a plain re-expression of a formed interferogram; `depth.points` uses the `camera` step.

### U15. The packaging rules contradict the FFmpeg decision

**Status:** Resolved in version 57 · **Where:** sections 10, 10.5 and 16

**Problem.**

- **The Packaging test.** A wrapper fails Packaging when "it cannot build the library into BIL as a private copy". Section 10 then says rsmpeg and ffmpeg-next fail it because they link an installed FFmpeg. Yet section 10.5 now requires FFmpeg to be the installed copy, and never a private one.
- **Bindings.** "Where none passes, BIL writes its own bindings in `bil-sys`." For FFmpeg it uses the rusty\_ffmpeg bindings instead.
- **The decision text.** "Wrapper crates" in section 16 lists three tests, and section 10 has four.

**Proposed resolution.** Rewrite the Packaging test as "it cannot supply the library from the source that section 10.5 sets for it". Make the fallback "BIL depends on an existing bindings crate where one passes, and writes its own bindings only where none does". Bring the decision text into line.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. Sections 10 and 16: the Packaging test asks for the source section 10.5 sets; existing bindings crates come before BIL's own; the FFmpeg reasoning is restated, since its wrappers no longer fail Packaging.

### U16. Some statements were left behind by later decisions

**Status:** Resolved in version 57 · **Where:** sections 1.6, 1.7 and 10.4

**Problem.**

- **Section 1.6** says missing pixels are "NaN in a Slice". Since version 50, `missing_value` can fill them.
- **Section 1.7** says image processing "sits above BIL in every case". Since version 53, BIL does the processing that prepares data.
- **Section 10.4.** The Missing data test requires NaN "in every Slice", and the One grid test checks only size, not `origin` and `scale`.

**Proposed resolution.** Update each statement. Section 1.6: "NaN in a Slice, or the caller's `missing_value`". Section 1.7: "Analysis sits above BIL". Section 10.4: the Missing data test holds under the default, and One grid requires the same size, origin and scale.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 57. Sections 1.6, 1.7 and 10.4: missing pixels may take `missing_value`; analysis, not image processing, sits above BIL; the Missing data test holds under the defaults, and One grid checks origin and scale.

## Critical findings

### R26. Isolation decodes the whole Source

**Status:** Resolved in version 59 · **Where:** sections 10.2 and 10.3

**Problem.** Isolation runs a driver in the decoder process of section 10.2, which "opens the original Source and decodes it" into the mapped format, "frame by frame for video". That is a decoding of the whole Source. A caller reading one small Window from a 100,000 × 100,000 NITF pays to decode 10 billion pixels first, and holds a 20 GB decoded copy.

- **It breaks rule 1, Proportional.** Cost follows the Source, not the request.
- **It breaks bounded memory and storage.** A live stream decoded this way grows without end.
- **It undermines a recent decision.** Version 56 bounds OpenJPEG's risk by recommending `isolation` set to `Untrusted` for hostile files. Under the current design, that makes large NITF files impractical to read.

**Proposed resolution.** The decoder process decodes on request. The caller's process sends each read, a Window and its blocks, and the decoder process decodes only those blocks into shared memory, which is reused as a bounded cache. The mapped format then serves two uses: decoded copies made deliberately, as now, and the exchange area for isolated reads, which is never a whole copy. For a live stream it is a ring of recent frames.

**Decision needed.** Accept, and whether the whole-copy form of external decoding stays as a separate choice.

**Ruling.** Accepted. A whole copy stays as a separate choice.

**What changed in the specification.** Version 59. Section 10.2: the decoder process works on request, decoding only the blocks each read needs into bounded shared memory, with a ring of frames for a live stream. Isolation always uses on request. A whole copy is configured separately, for many readers or for keeping the decoded data.

### R27. Window independence is not defined to the bit

**Status:** Resolved in version 59 · **Where:** sections 5, 10.4 and 12

**Problem.** The Window independence test requires a Slice of a Window to "equal" the same region cut from the whole. Section 5 says computed Levels give the same values "on every machine". Neither holds automatically.

- **Within one machine.** Vector loops handle the last few pixels of a row in a scalar tail. If the vector body uses fused multiply-add and the tail does not, a pixel computed in the body for one Window and in the tail for another differs in its last bit. Where a Window starts decides which path a pixel takes.
- **Across machines.** Loops are chosen at run time among AVX2, AVX-512 and NEON. Their fused operations, order of sums, and approximations of the arctangent, logarithm and power differ.
- **The test cannot pass reliably** until "equal" is defined.

**Proposed resolution.**

- **Bitwise on one machine.** The Window independence test requires identical bits. Every loop computes a pixel by the same sequence of operations whether it falls in the vector body or the tail, using masked vector tails or a scalar path that mirrors the vector one exactly.
- **Stated tolerance across machines.** Results from different instruction sets agree within a stated bound: exact for integer-to-float conversion, within one unit in the last place for arithmetic, and a bound stated for each transcendental function. The claim "on every machine" is restated this way.
- **Tested on each instruction set.** The conformance suite runs on each instruction set BIL ships.

**Decision needed.** Accept, and whether bitwise agreement across machines is wanted at a cost in speed.

**Ruling.** Not accepted as proposed: bitwise agreement is not a concern. Window independence and agreement between machines hold to a tolerance.

**What changed in the specification.** Version 59. Sections 5 and 10.4: equal means within one unit in the last place of float32 for arithmetic, and within each transcendental function's stated bound. Experiment 1's bitwise result is kept where it costs nothing, but is not required. The Window independence test runs on every instruction set BIL ships.

### R28. Markings can be separated from the data

**Status:** Resolved in version 59 · **Where:** sections 11 and 14.2

**Problem.** None of BIL's writable formats has a native field for a security marking, so for every file BIL writes, markings go to the companion file `<file>.metadata`. Copying the file without its companion loses the marking silently. Reading such a copy later gives data with no markings and no sign that any are missing. On closed networks this is a spill risk, and it falls on exactly the data the marking rule exists to protect.

**Proposed resolution.**

- **Inside the file wherever there is room.** BIL writes markings inside the file wherever the format has a place for text, as well as in the companion file. An ENVI header can hold additional keys, and a TIFF can hold text metadata, for example in the GDAL metadata tag. `.npy` has none.
- **A missing companion is detected.** A file that BIL wrote records that a companion file belongs to it. Reading it without the companion gives status `Partial` and a reported warning.
- **A refusal on request.** A `WriteOptions` setting refuses to write a marked Source to a format with no internal place for markings.

**Decision needed.** Accept, and whether writing `.npy` from a marked Source is refused by default.

## Serious findings

**Ruling.** Accepted, with the refusal off by default.

**What changed in the specification.** Version 59. Section 11: markings are written inside ENVI headers and TIFF text metadata as well as the companion file; a file BIL writes records its companion, and reading it without one gives `Partial` and a warning; `require_internal_markings` in `WriteOptions` refuses formats with no internal place, and is off by default.

### R29. Stored levels are not on the Level grid

**Status:** Resolved in version 60 · **Where:** section 5

**Problem.** A computed Level n sits on Image pixels at multiples of 2ⁿ. A stored overview usually does not. In a typical TIFF overview, pixel i of the first overview averages full-resolution pixels 2i and 2i+1, so its center lies at 2i + 0.5. With `level_source` set to `Stored`, a Level made from that overview sits half a pixel away from the same Level computed from full resolution. That breaks "every Slice of an Image at a given Level shares one grid" and makes results depend on which source was used. Version 46 fixed alignment for computed Levels only.

**Proposed resolution.**

- **Reported.** A stored level reports its exact `origin` and `scale`, which is already required.
- **Used to make a Level only when it aligns.** `level_source` = `Stored` uses a stored level only when its grid falls on the Level's grid. Otherwise the Level is computed from full resolution, and `plan` says so.
- **Read as it stands, freely.** Reading a stored level directly is always allowed, and its Slices carry their true origin.
- **Tested.** The One grid test checks origin and scale as well as size.

**Decision needed.** Whether a misaligned stored level is refused, or resampled onto the grid by a local function.

**Ruling.** Misaligned stored levels are not allowed to make a Level.

**What changed in the specification.** Version 60, section 5. `list_stored_levels` reports whether each stored level is aligned with the Level grid. `level_source` = `Stored` uses only aligned stored levels, and otherwise computes the Level from full resolution, as `plan` reports. A name such as `stored2.level1` is an error when `stored2` is misaligned. A misaligned stored level can still be read as it stands, with its true origin and scale.

### R30. Writing loses geocoding, and ENVI cannot hold every sample type

**Status:** Resolved in version 59 · **Where:** section 11

**Problem.**

- **Geocoding.** Section 11 writes pixels, markings and fields, but never the geo model. A GeoTIFF written by BIL from an RPC-located NITF chip has no georeferencing unless the caller builds it.
- **Sample types.** The specification says ENVI "holds every sample type, including complex". By my knowledge of the ENVI header, it has no type for signed 8-bit, 16-bit float, 8-bit complex or 16-bit complex integers. A SICD in `ComplexI16` cannot be written to ENVI in its native type, so the round trip fails or is silently widened.
- **Slices at reduced Levels.** Writing a Slice from a Level or a stored level is not described: which Window it fills, and what geo model it carries.

**Proposed resolution.**

- **Writers write the geo model.** Each writer writes the geo model as far as the format holds it: GeoTIFF the affine, tie-point or RPC tags; ENVI its map information. The chain includes the `image_window` step of the Slice written. What the format cannot hold goes to the companion file.
- **Each writer lists its sample types.** Writing a type a format cannot hold is an error, never a silent conversion.
- **A Slice is written at its own grid.** The output Image has the Slice's size, and its geo model starts from the Slice's `origin` and `scale`.

**Decision needed.** Accept.

**Note.** Version 58 adds `write_slice(window, slice, component)` and `write_geo_model(model)`, so a caller can write into named components and write a geo model. Still open: whether writers write the geo model without being asked, the sample types each writer refuses, and how a Slice from a reduced Level is written.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Section 11: writers write the geo model unasked, as far as the format holds it, with the rest in the companion file; a Slice is written on its own grid; each writer lists its sample types, and any other is an error. ENVI's list no longer claims every sample type.

### R31. A deferred Fragment has no decoder of its own

**Status:** Resolved in version 59 · **Where:** section 4

**Problem.** A video Fragment is deferred: it holds compressed packets, and its pixels are decoded "on demand". Decoder state lives in a Reader, and a Reader belongs to one thread. Fragments, however, may be passed between threads. Nothing says which decoder decodes a deferred Fragment that another thread asks for, or one asked for after its Reader has moved on to other frames. The cache, meanwhile, holds decoded frames "each as a Fragment", so the same frame can exist in deferred and decoded form at once.

**Proposed resolution.**

- **Self-contained.** A deferred Fragment holds everything needed to decode it: its packets from the preceding key frame.
- **Its own Reader if it can.** It is decoded by the Reader that made it, when that Reader is free and in place. Otherwise BIL decodes it with a decoder made for the purpose, at the cost `plan` reported.
- **Decoded once.** Once decoded, the Fragment and the cache entry are the same object, so a frame is never decoded twice for one Source.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Section 4: a deferred Fragment holds its packets from the key frame, is decoded by its own Reader when free and in place or else by a decoder made for it, and once decoded is the same object as its cache entry.

### R32. Untrusted files get no limits by default, and the cache is limited only by count

**Status:** Resolved in version 59 · **Where:** sections 4 and 10.3

**Problem.**

- **No limits.** No resource limit is set by default, and opening a Source as untrusted changes only isolation. A compressed block that expands without bound is stopped only if the caller thought to set `max_decoded_bytes`.
- **A count is not a memory bound.** One ARGUS-IS frame is 1.8 gigapixels, about 3.6 GB as 16-bit samples. The default cache of 16 Fragments could hold about 58 GB. A count bounds the memory only if block sizes are known.

**Proposed resolution.**

- **Untrusted sets limits.** Opening a Source as untrusted applies default limits: decoded bytes per block, pixels per Image, and bytes per metadata group, each scaled to the file's size where that makes sense. The caller can raise them.
- **A byte ceiling as well as a count.** The cache keeps its count, as decided, and adds a byte ceiling as a second limit. The default is a stated fraction of physical memory, and whichever limit is reached first applies.

**Decision needed.** The default byte ceiling.

**Ruling.** Accepted. The default byte ceiling is a quarter of physical memory.

**What changed in the specification.** Version 59. Sections 3.4, 4 and 10.3: `set_cache_bytes` adds a byte ceiling beside the count, and an untrusted Source gets default limits scaled to the file.

### R33. Remote Sources are not designed for

**Status:** Resolved in version 59 · **Where:** sections 1.1, 10 and 12

**Problem.** Remote reading is named as a source of data, through object\_store and the OxiGeo storage backend, but the design treats it like a local file.

- **Async inside a sync library.** object\_store's interface is asynchronous. BIL's interface is not. Running an async runtime inside a shared library that is called from Python, from C, and from Rust programs that have their own runtime causes conflicts, and nothing says how BIL avoids them.
- **Latency is not in the cost model.** Rule 1 counts bytes. Over a network, the number of requests dominates: a Window that touches 400 small tiles costs 400 round trips unless they are merged.
- **No timeouts or retries.** A stalled request blocks a read without end.

**Proposed resolution.**

- **A private runtime.** BIL runs remote reads on a small runtime of its own, hidden from the caller, and offers a blocking interface as now. An async interface can follow later.
- **Requests are counted and merged.** `plan` reports the number of requests and the bytes for a read. Adjacent byte ranges are merged into one request, up to a stated gap.
- **Timeouts and retries.** `OpenOptions` sets them for remote Sources, and a read that exceeds its timeout is an error.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Sections 3.4 and 10.3: remote reads run on a private runtime behind the blocking interface; `plan` reports requests and bytes; ranges within `remote_merge_gap` are merged; `remote_timeout` and `remote_retries` bound each request.

### R34. Live streams cannot be stopped, timed out or read twice

**Status:** Resolved in version 59 · **Where:** sections 3.2 and 4

**Problem.**

- **No way to stop.** A read on a live stream waits for the next frame. There is no timeout and no way to cancel it, so a stalled stream holds the caller's thread forever.
- **No single position.** A live stream moves forward only, but each Reader "keeps its place". Two Readers on one live stream would each need their own copy of the stream, and nothing says what happens.
- **Past frames.** Nothing says what a request for a frame older than those still held returns.

**Proposed resolution.**

- **One position.** A live Source has one shared position. All Readers see the same frames.
- **Past frames are `Missing`.** A frame older than those held is delivered with status `Missing`, never by waiting or seeking.
- **Timeouts and cancellation.** Every read on a stream takes a timeout, and `SliceRequest` and `stream_slice` take a cancellation handle the caller can trigger from another thread.

**Decision needed.** Accept.

**Note.** Version 58 adds `latest_frame()`, and `timeout` and `cancel` in `SliceOptions`. Still open: one shared position for a live stream, and `Missing` for a frame no longer held.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Section 3.2: a live Source has one position for all Readers, and a frame no longer held is `Missing`. Version 58 had already added `latest_frame`, `timeout` and `cancel`.

### R35. The LGPL may keep Rust developers away

**Status:** Resolved in version 59 · **Where:** section 10.6

**Problem.** The naming rule was changed so that Rust developers would accept BIL. The license may undo that. Rust programs normally link their dependencies statically, and the LGPL then requires that users can relink the program with a modified BIL. Rust has no stable binary interface, so in practice that means shipping object files built with the same compiler version, or the program's source. Section 10.6 notes the duty, but many Rust projects will not take on an LGPL dependency for this reason.

**Proposed resolution.** Add a static-linking exception to the license, as some LGPL libraries do: the OCaml runtime's linking exception is one example. A program may then link BIL statically without the relinking duty, while changes to BIL itself stay under the LGPL. This goes to the legal review already open in section 16.

**Decision needed.** Whether to add a static-linking exception.

## Lesser findings

**Ruling.** A static-linking exception is added.

**What changed in the specification.** Version 59. Section 10.6: a program may link BIL statically without the duty to allow relinking; changes to BIL itself stay under the LGPL. The wording goes to the license review, now noted in section 16.

### R36. The precision rule is wrong for integer data

**Status:** Resolved in version 59 · **Where:** section 5

**Problem.** "A slice function computes at the precision of the Fragment." For integer data that would mean integer arithmetic: the power of a `ComplexI16` sample in 32-bit integers can overflow, since twice 32,768 squared exceeds the largest signed 32-bit value.

**Proposed resolution.** Restate it: arithmetic is done in float32 for sample types that float32 holds exactly, and in float64 for `U32`, `I32`, `U64`, `I64`, `F64` and `ComplexF64`. It is never done in integers. The result is narrowed to float32 once, at the end.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Section 5: arithmetic is in float32 for types float32 holds exactly and in float64 otherwise, never in integers.

### R37. A validity mask at reduced Levels is undefined

**Status:** Resolved in version 59 · **Where:** section 5

**Problem.** `pixel.mask.valid` is 1.0 or 0.0. Reduced by `bilinear`, it becomes a fraction. Under the NaN rule a reduced value is valid when any contributing pixel is, so a mask reduced as a linear quantity disagrees with the data it describes.

**Proposed resolution.** Masks are their own kind of Slice. At a reduced Level, `valid` is 1.0 where the reduced data value exists, which is wherever any contributor was valid. A further Slice, `valid_fraction`, gives the share of contributors that were valid.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Section 5 and 9.1: masks are their own kind of Slice, and `pixel.mask` gains `valid_fraction`.

### R38. Saved frame indexes and companion files are trusted blindly

**Status:** Resolved in version 59 · **Where:** sections 3.2, 10.3 and 14.2

**Problem.**

- **Stale indexes.** A saved frame index records where each frame is. Nothing checks that it still matches the stream, so after the file changes it silently points at the wrong frames.
- **Trusted companions.** A companion file beside the data is read automatically and can supply calibration values and markings. For a file from outside, the companion comes from outside too. Section 10.3 refuses plugins found beside the data, but companion files are taken from there.

**Proposed resolution.**

- **An index checks its stream.** A saved index records the size and a checksum of the start of the stream, and is refused when they do not match.
- **Untrusted Sources skip companions.** For a Source opened as untrusted, a companion file is read only if the caller allows it. Markings in it are always reported, whether or not its values are used.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Sections 3.2, 3.4 and 14.2: a saved frame index records the stream's size and a checksum and is refused on a mismatch; an untrusted Source uses its companion's values only with `trust_companion`, and always reports markings in it.

### R39. BIL has no stability policy for its own interfaces

**Status:** Resolved in version 59 · **Where:** sections 2 and 10

**Problem.** The plugin interface has a version. BIL's own C interface, Rust crate and Python package do not. Callers of a shared library need to know when an update can break them.

**Proposed resolution.** Semantic versioning for the crate and the shared library, with the C interface changed incompatibly only at a major version. The Python package is built against Python's stable interface, so one build serves several Python versions.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Section 2: semantic versioning for the crate and shared library, incompatible C changes only at a major version, and the Python package built against Python's stable interface.

### R40. The conformance suite misses some of the design

**Status:** Resolved in version 59 · **Where:** section 10.4

**Problem.** Several things the specification now requires have no test.

- **Declared concurrency.** A driver that declares `Parallel` wrongly corrupts data.
- **Deferred decoding.** Native pixels read from a deferred Fragment, and those read from a decoded one, must match.
- **Status.** Results must report `Partial` and `Missing` correctly.
- **Field names.** Escaped dots and numbered repeats must parse.
- **One grid.** The test checks size only.

**Proposed resolution.** Add a test for each: parallel reads compared with serial ones, deferred against decoded Fragments, files built with known gaps for status, names with dots and repeats, and origin and scale in One grid.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Section 10.4: tests for declared concurrency, deferred decoding, status and field names, and One grid checks origin and scale (version 57).

### R41. Exif already has a time in UTC

**Status:** Resolved in version 59 · **Where:** section 14.2

**Problem.** Section 14.2 takes Exif `DateTimeOriginal` as local time and works out its zone. Exif files with a GPS position usually also carry `GPSDateStamp` and `GPSTimeStamp`, which are in UTC. Those give `common.collection_time` directly, with no zone to find.

**Proposed resolution.** Add the Exif GPS date and time as a provided source of `common.collection_time`, in UTC, ahead of the local-time rule. `DateTimeOriginal` is used only where the GPS time is absent.

**Decision needed.** Accept.

**Ruling.** Accepted.

**What changed in the specification.** Version 59. Section 14.2: Exif `GPSDateStamp` with `GPSTimeStamp` provides `common.collection_time` in UTC, ahead of the local-time rule.

## What held up

These parts were attacked again and stood.

- **The five foundations of section 1.5.** No finding touches index and coordinate, containers apart from codecs, geocoding as a chain or pixels by description. R27 tightens the meaning of window independence without changing it.
- **One read request.** Every read path examined is a form of `SliceRequest`. U8 only changes how its resolution is named.
- **Results and status.** The single status model covered every case examined. U9 settles one edge.
- **Choosing the codec once.** No path was found by which a file is decoded partly by one codec and partly by another.
- **Containment.** No path was found by which BIL reaches the network or writes a file unasked. R38 concerns what it reads, not what it writes.
