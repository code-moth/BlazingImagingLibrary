This manual explains how to use the Blazing Imaging Library (BIL): what to call, what comes back, what it costs and what can go wrong. It is written beside the code, against specification version 69, and read as it grows to find where the design is unclear, inconsistent or incomplete. Chapter 24 lists what that reading has found so far.

## 1. About this manual

**Who it is for.** Anyone who will call BIL from Python, Rust, C or C++ to read imaging sensor data: algorithm developers, people building processing systems, and people writing drivers for new formats.

**How it relates to the other documents**

| Document | Says |
| --- | --- |
| The specification | What BIL must do. It is the source of truth, and where this manual and the specification differ, the specification is right and this manual has a fault. |
| BIL by Example | Short calls for every part of the specification, with little explanation |
| This manual | How to use BIL: the concepts in order, the reasons behind them, the costs, and advice |
| The implementability assessment | Whether BIL can be built, and the risks |

**What runs today.** Very little. BIL is designed, and the first experiments on its core have passed, but there is no library to install yet. Each chapter opens with its status, so a reader always knows which parts are design and which are code.

| Status | Means |
| --- | --- |
| **Specified** | The specification defines it. No code exists, and the calls shown are the specification's, not yet working. |
| **Prototyped** | Experiment code does it, with tests, but not through the interface shown. Chapter 23 shows the experiment's own interface. |
| **Built** | It is in the `bil` crate and can be used as described. |

**Conventions**

- **Python first.** Most examples are in Python, because it is the shortest. Chapter 22 shows the same calls in Rust and C.
- **Two assumptions,** shared with BIL by Example: `bil.Window(row_start, col_start, row_count, col_count, step=1, frame=0)` builds a Window, and options are passed to calls as keyword arguments.
- **Section numbers** such as "(section 5)" refer to the specification.
- **Names.** Types and enum values are UpperCamelCase, and every other name is snake\_case, including the names passed as text, such as `color.hsv.hue` and `level3`.

## 2. What BIL is

> **Status:** Specified.

BIL is one interface to imaging sensor data in any format. It reads the part of the data an algorithm needs, as the native pixels or as 2D float32 arrays, leaves the rest unread, and locates every pixel in the world.

**Why it exists.** Sensor data grows faster than memory: one band of 100,000 × 100,000 pixels is 40 GB as float32, and one 1.8-gigapixel frame of a wide area sensor is 7.2 GB. No one processes such data whole. Each format solves the problem of reading a part differently, through tiles, strips, chunks, key frames or pyramids, and without a common layer every algorithm has to learn each one. BIL is that layer.

**What it does**

- **Reads a part of the data.** A Window takes a rectangle, a Level a coarser resolution, a Slice one 2D view of the components, and a frame one moment in time.
- **Delivers native pixels or float32.** A Fragment is the stored samples, unaltered. A Slice is a named float32 array, such as `complex.polar.magnitude` or `color.hsv.hue`.
- **Locates pixels.** Every format's geocoding is a chain of the same steps, run in both directions, with a full record of its accuracy.
- **Reads metadata on demand.** Fields under their native names, decoded when first read, and a dictionary of essential items with the same names in every format.
- **Writes a few simple formats:** TIFF and GeoTIFF, ENVI and NumPy `.npy`.

**What it does not do.** Registration, orthorectification and analysis are layers above BIL. BIL prepares the data they start from. BIL reads medical data for research and processing; it is not for diagnostic use.

**Seven rules.** Every part of BIL follows them, and they explain most of its behavior.

1. **Proportional:** the cost of a read follows the data it covers, not the size of the file.
2. **Lazy:** nothing is decoded until a region is asked for.
3. **Faithful:** a Fragment delivers the stored samples unaltered.
4. **Composable:** choosing an Image, a frame, a Window, a Level and a slice function are separate steps that stack.
5. **Transparent:** the original metadata is always reachable, under its native names.
6. **Contained:** BIL reaches the network only for a location the caller gave, and writes only where the caller named a place.
7. **Uniform:** every format and sensor passes through the same calls. A difference between sources is a property the caller can read, never a different call.

## 3. Installing

> **Status:** Specified. The package names are held by early releases with no reading in them yet.

| Language | Package | Install |
| --- | --- | --- |
| Python | `bil` on PyPI | `pip install bil` |
| Rust | `bil` on crates.io | `cargo add bil` |
| C and C++ | The `bil` shared library, with its header | From the same build as the Rust crate, or a package manager |

- **One shared library.** The C interface covers the whole Rust library and is delivered with it as one shared library. C++ uses the C interface.
- **Libraries from the package manager.** BIL takes each library it needs from an existing crate or from the installing package manager, and carries a private copy only as a last resort (section 10.5). FFmpeg, for video, always comes from conda or the system's package manager, at version 6 or later.
- **Python only for scripted drivers.** Rust, C and C++ callers need no Python, unless they use drivers written in Python. Those run in the Python installed on the machine, in a helper process (chapter 20).
- **Optional plugins are separate.** Plugins under a license stricter than BIL's, such as the Grok JPEG 2000 codec, and the `bil-extras` analysis functions, are separate packages. Installing one records its location in BIL's configuration.
- **Not every format at first.** BIL is built in tiers. Tier 1 reads GeoTIFF and NITF. Tier 2 adds ENVI, video with KLV metadata, and SICD and SIDD. Tier 3 adds the rest, case by case.

**License.** The GNU Lesser General Public License, version 2.1 or later, with a static-linking exception: a program may link BIL statically, as Rust programs normally do, without the duty to let its users relink it. BIL's own packages contain only builds compatible with LGPL 2.1, with Apache-2.0 components accepted. Libraries under LGPL 3 are used only as test references or as optional plugins packaged separately.

**What can be run today.** The experiments on BIL's core, from a clone of the repository (chapter 23).

```sh
cd experiments/exp1-fused-loop
cargo run --release --bin exp1b -- --check   # correctness, as CI runs it
cargo run --release --bin exp1b              # with the speed tables
```

## 4. The concepts in ten minutes

> **Status:** Specified.

**From file to array in three steps**

| Step | You get | It is |
| --- | --- | --- |
| Open | A `Source` | One file, a set of files or a stream, holding one or more Images |
| Read | A `Fragment` | The native pixels of a Window, unaltered, borrowed without a copy where the storage allows |
| Process | A `Slice` | A named 2D float32 array made from a Fragment, such as `color.hsv.hue` |

**The vocabulary**

| Concept | What it is |
| --- | --- |
| Source | What `open_source` opens: one file, a set of files or a stream |
| Image | One 2D pixel grid in a Source, with one pixel type and named components. It may repeat along frames, as video does. |
| Window | A rectangle of an Image, in full-resolution rows and columns, with a frame and an optional `step` |
| Fragment | The stored pixels of a Window |
| Slice | One named 2D float32 array for a Window |
| Slice function | What makes Slices from a Fragment: `color.hsv` makes `hue`, `saturation` and `value` |
| Level | A coarser resolution. Level n holds the pixels whose row and column are multiples of 2ⁿ. |
| Frame | The nth 2D array of an Image, counted from 0 |
| Index and coordinate | An index says where data is packed, such as a frame number. A coordinate says what it physically is, such as a time. Reads use indexes. |
| Field | One named item of metadata |
| Status | How a result turned out: `Complete`, `Partial`, `Missing`, `NotConverged` or `Ambiguous` |

**Why a Source has several Images.** All the components of a pixel share one grid and one sample type. Data on another grid, or of another type, is another Image: a panchromatic band beside coarser multispectral bands, the segments of a NITF file, or the luma and half-size chroma of video. A driver says how Images relate, so a slice function can use them together.

**The five cuts.** Each concept cuts the data along one dimension, and they stack.

| Too much | Cut with | How |
| --- | --- | --- |
| Spatial extent | Window | Take a rectangle |
| Resolution | Level | Take a coarser version, filtered or decimated |
| Bands or components | Slice | Take one 2D view |
| Time | Frame | Take one frame |
| Sample precision | float32 | One type for every sensor |

**Two promises to rely on**

- **A Window gives the same answer as the whole.** A Slice of any Window equals the same region cut from the Slice of the whole Image, to within one unit in the last place, or the stated bound of a transcendental function. An algorithm can therefore work through an image of any size in pieces.
- **Every Slice at one Level shares one grid.** Whatever slice function made it, a Slice at Level n has the same size and position as every other Slice of the Image at Level n.

**Everything beyond 2D is selected, never ranged.** Rows and columns are read as a range, through a Window. A frame and a component are chosen one at a time. A read across frames is therefore outside BIL: a caller reads frame by frame.
## 5. Opening a Source

> **Status:** Specified.

```python
import bil

source = bil.open_source("scene.ntf")
image  = source.image(0)
```

**What opening does.** It finds a driver, reads what describes the Images (size, pixel type, component names and frame count) and nothing else. No pixel is read, no metadata group is extracted beyond those descriptions, and a video's frames are not counted.

**How the driver is chosen.** In two stages, because many formats share a container: DNG, GeoTIFF and most whole-slide formats are TIFF underneath.

1. **The container,** from the file name and its first bytes.
2. **The content.** The drivers for that container look inside and each returns a confidence. The highest wins, and ties follow the installation's configured order, never the order plugins were loaded.

To choose the driver yourself, call `open_source_with`. `describe_decoding()` on a Source reports the driver and codec chosen, with their versions. They are kept for the life of the Source, so an Image is never decoded partly by one codec and partly by another.

**Options.** `open_source` takes `OpenOptions`. Every option has a default, and most callers set none.

| Option | Default | Use it to |
| --- | --- | --- |
| `mapping` | `Shared` | Choose how files are mapped into memory: `Shared`, `Private` (copy-on-write, so a borrowed array can be changed) or `Off` (for network file systems where mapping is unreliable). Chapter 10. |
| `access_pattern` | `Random` | Say how you will move through the Source: `Random` or `Sequential`. Chapter 12. |
| `read_ahead_bytes` | 64 MB | Bound what `Sequential` reads ahead |
| `untrusted` | False | Open a file from outside with default limits, and isolate it where the installation says so. Chapter 19. |
| `max_image_pixels`, `max_decoded_bytes`, `max_field_group_bytes` | None | Refuse an Image, a compressed block or a metadata group larger than this |
| `corrupt_as_missing` | False | Deliver a corrupt block as missing pixels, with status `Partial`, instead of an error |
| `companion` | The file's name with `.metadata` added | Name a companion file of metadata (chapter 16) |
| `trust_companion` | True, or False when `untrusted` is set | Use the companion file's values |
| `frame_index` | None | Use a saved frame index instead of scanning a video (chapter 11) |
| `max_held_positions` | 1,000 | Bound the frames for which metadata is held |
| `max_held_group_bytes` | None | Bound the bytes of extracted metadata held |
| `remote_timeout` | 30 seconds | Bound each request to a remote Source |
| `remote_retries` | 3 | Retry a failed remote request |
| `remote_merge_gap` | 1 MB | Read byte ranges closer than this in one request |

```python
source = bil.open_source(
    "s3://bucket/scene.tif",
    access_pattern=bil.AccessPattern.Sequential,
    remote_timeout=60,
)
```

**Remote Sources** are read through the same calls. BIL runs its network code on a small runtime of its own, so a caller's own asynchronous runtime is never touched. Requests are counted and merged, because over a network the number of requests costs more than the bytes, and `plan` reports both.

**Closing.** In C, `bil_close_source` closes a Source. For Rust and Python the specification does not yet say; this manual assumes a Source closes when the last reference to it goes (chapter 24, M3). A borrowed Fragment or Slice holds a reference, so the Source stays open while any of its borrowed arrays lives. Closing frees the cache, the held metadata and anything read ahead.

## 6. Looking at an Image

> **Status:** Specified.

```python
source.image_count()            # how many Images
for info in source.list_images():
    print(info.row_count, info.col_count, info.pixel_type, info.component_names)

image = source.image(0)
info  = image.info()
```

**`ImageInfo`**

| Field | Says |
| --- | --- |
| `row_count`, `col_count` | The size, in pixels |
| `frame_count` | The number of frames, with whether it is `Known`, `AtLeast` or `Unknown` |
| `pixel_type` | The sample type, the component count, the significant bits and the bit shift (chapter 10) |
| `representation` | What the samples are, with its parameters: intensity, color, a mosaic and its pattern, complex, spectral with its band coordinates, and so on. It decides which slice functions apply. |
| `component_names` | The name of each component |
| `relations` | Each related Image, how it relates, and for a scaled grid the reduction and where its samples sit |
| `storage` | How the pixels lie in the file: block size, interleave, and whether the order of the blocks is known (chapter 12) |

**What else to ask an Image**

| Call | Answers |
| --- | --- |
| `list_slice_functions()` | Which slice functions this Image can use, given its representation and the metadata available |
| `list_slice_names(function_name)` | The Slices a function makes for this Image, in order |
| `default_slice()` | The Slice BIL reads when you do not choose |
| `list_stored_levels()` | The reduced-resolution data the file holds |
| `list_level_methods()` | The downsampling methods available |
| `list_coordinates()` | The coordinates, such as time or wavelength, and the index each attaches to |
| `list_geo_models()` | The geocoding models, most rigorous first |
| `list_field_groups()` | The groups of metadata present, extracting none |

**Relations.** A Source declares how its Images relate. "Same grid" Images line up pixel for pixel, such as float range beside 16-bit intensity. A "scaled grid" Image covers the same scene at a whole-number reduction, such as video chroma at half size. A slice function that reads several Images, such as `video.rgb`, is called on the first, and is offered only when the others are present.

## 7. Reading Slices

> **Status:** Specified. The arithmetic of `pixel.raw`, `complex.polar` and `color.hsv`, and the requests for several Slices and Levels, are prototyped (chapter 23).

**One Slice**

```python
window    = bil.Window(0, 0, 2048, 2048)
magnitude = image.read_slice("complex.polar.magnitude", window)

magnitude.data          # a NumPy float32 array, over BIL's memory where it can be
magnitude.status        # Complete, Partial or Missing
magnitude.origin, magnitude.scale   # where it sits in the Image, in pixels
```

**Names have three parts.** Family, function and Slice: `complex.polar.magnitude`, `color.hsv.hue`, `pixel.raw.intensity`. There are no short forms and no aliases, even where a function makes one Slice. A function alone is named with two parts, `color.hsv`.

**Parameters go in a selector,** never in the name.

```python
green = bil.SliceSelector("spectral.integrate.value", from_nm=500, to_nm=600)
band  = image.read_slice(green, window)

hv    = bil.SliceSelector("complex.polar.magnitude", component="hv")
```

A plain string is a selector with no parameters. A function that works on one component takes the component's name as `component`.

**Names from the file.** Some functions name their Slices from metadata, such as `spectral.bands`. A name the file gives is made snake\_case ("Near Infrared (B8)" becomes `near_infrared_b8`), repeats are numbered from 1 (`red_1`, `red_2`), and the name as the file wrote it is kept as the Slice's `label`. A Slice can also be chosen by its position in `list_slice_names`, counted from 0.

**Several Slices, one pass**

```python
hsv = image.read_slices("color.hsv", ["hue", "value"], window)
hue = hsv["hue"]
```

The Fragment is decoded once, every requested Slice is made in the same pass, and work they share, here the one color conversion, is done once. A Slice you do not ask for costs nothing.

**The general form: one request.** Every read call is a short form of `read(request)`.

```python
request = bil.SliceRequest(
    slices=["complex.polar.magnitude", "complex.polar.phase", "pixel.mask.valid"],
    window=window,
    levels=["level0", "level2", "level4"],
    options=bil.SliceOptions(missing_value=0.0),
    delivery=bil.Delivery.Returned,
)
plan   = image.plan(request)     # sizes, access class, blocks to decode, before any pixel is read
result = image.read(request)
result["complex.polar.phase", "level2"]   # indexing by Slice and Level is assumed (chapter 24, M5)
```

| Part | Choice |
| --- | --- |
| `slices` | One or more Slices, from one function or several |
| `window` | The Window, in full-resolution rows and columns |
| `levels` | One or more Levels by name: `level3`, `stored2`, `stored2.level1`. A number n means `leveln`. |
| `options` | The `SliceOptions` below |
| `delivery` | `Returned` as arrays, `IntoBuffers` you own, or `Streamed` as row blocks to a callback |

**`SliceOptions`**

| Option | Default | Meaning |
| --- | --- | --- |
| `level_method` | `bilinear` | How Levels are made: `bilinear`, `binomial5`, `decimate`, or one a plugin adds (chapter 8) |
| `level_source` | `Computed` | `Stored` lets BIL start from a stored level it chooses |
| `decimate_first` | 0 | Halvings taken by striding before the level method |
| `missing_value` | None | A number for missing positions instead of NaN (chapter 9) |
| `missing_weights` | `Equal` | How a Level weights the values below it where some are missing: `Equal` or `PixelCount` |
| `timeout` | None | The longest a read may wait, as for the next frame of a live stream |
| `cancel` | None | A handle another thread can trigger to stop the read |

**Delivery.** Choose by where the values are going.

| Delivery | Short form | Use it when |
| --- | --- | --- |
| `Returned` | `read_slice`, `read_slices`, `read_slice_levels` | The result fits in memory and you want arrays |
| `IntoBuffers` | `read_slice_into` | You read many Windows of one shape: allocate once, read into the same buffers every time |
| `Streamed` | `stream_slice` | You process rows as they come, so the float32 form never fills memory |

```python
import numpy as np

buffer = np.empty((2048, 2048), dtype=np.float32)
for window in windows:
    image.read_slice_into("pixel.raw.intensity", window, buffer)   # allocates nothing
    process(buffer)

def take(block):            # block.slice_name, block.level_name, block.rows, block.values
    accumulate(block)
image.stream_slice("pixel.raw.intensity", window, take)
```

Streamed blocks of one Slice and Level arrive in row order. Blocks of different Slices and Levels may interleave.

**Know the cost first.** `plan_slice(slice_name, window)` and `plan(request)` report, before any pixel is read, the output sizes, the access class (`Borrowed`, `Converted` or `Decoded`), any stored level used, the blocks or frames to be decoded, the blocks already read ahead, and, for a remote Source, the requests and bytes. A conformance test holds every driver to its plan.

**Where a Slice sits.** Every Slice carries its `origin`, the Image position of its first value, and its `scale`, the spacing of its values, both in pixels and 64-bit. Geocoding reads them, so a Slice from any Window, Level or stored level is located without adjustment (chapter 17).

## 8. Levels

> **Status:** Prototyped for `bilinear`, `binomial5` and `decimate`, every kind of Slice, Levels 0 to 4 in one pass, and coarse Levels alone, where `decimate` reads only the pixels kept, in an Image in memory. `decimate_first` and stored levels are specified.

**What a Level is.** Level 0 is full resolution. Level n holds the Image pixels whose row and column are multiples of 2ⁿ, each made from the Level below by the level method. The grid is anchored to the Image origin, not to the Window, which is why a Window gives the same values as the whole.

```python
overview = image.read_slice("pixel.raw.intensity", window, level=3)
pyramid  = image.read_slice_levels("pixel.raw.intensity", window, 4)   # level0 to level4, one pass
pyramid["level2"]
```

**Size.** A Slice at Level n holds the grid points that fall inside the Window. A Window of rows 100 to 199 at Level 2 holds the grid rows at 100, 104, … 196: 25 rows. `plan_slice` reports the size.

**How many Levels.** `level_count()` gives the number of resolution levels the Image offers (chapter 24, M6).

**All Levels in one pass.** Each halving makes the Level above it, so every Level from 0 to n costs about a third more than Level 0 alone. Ask for them together rather than one at a time.

**Methods**

| Method | Weights at each halving | Use it for |
| --- | --- | --- |
| `bilinear`, the default | 1 2 1, divided by 4, along rows then columns | Most work. Cheap, and every Level in one pass. It passes half the amplitude at the highest frequency the halved grid can hold, so fine detail aliases. |
| `binomial5` | 1 4 6 4 1, divided by 16 | Where aliasing matters. It passes a quarter at that frequency, for about twice the arithmetic, which cost 0.9 to 1.3 times `bilinear` in experiment 1b. |
| `decimate` | None: the pixel at each grid point | A quick look. It reads only the pixels kept, where the storage allows. |

**Kinds of Slice.** Averaging suits most Slices, but not all, so each Slice declares how it is reduced.

| Kind | Examples | Reduced by |
| --- | --- | --- |
| Linear | `intensity`, `power`, `red`, band values | The level method |
| Angle | `phase`, `hue` | The level method on unit vectors, so 359° and 1° average to 0°, not 180° |
| Label | `class.labels.value`, `palette.lookup.index` | Keeping every second value. A label is never averaged. |
| Mask | `pixel.mask.valid` | 1.0 where the reduced value exists, else 0.0 |
| Range | `depth.sensor.range` | The least present value of the 3 × 3 block centered on each grid point, which favors the nearest surface |

Levels are made after the slice function, so reduced `power_db` is not the decibels of reduced `power`. Reduce `power`, which is the physical quantity, and convert afterwards if you need decibels.

**What a coarse view costs.** A value that summarizes an area must read that area. A computed Level reads every full-resolution pixel under its Window, so an averaged overview of a whole Image means processing the whole Image. Two routes cost less.

| Route | Reads | Gives up |
| --- | --- | --- |
| Computed, filtered (`bilinear`, `binomial5`) | Everything under the Window | Nothing. The same values for every file and machine. |
| Computed, `decimate` | Only the pixels kept, where the storage can be strided. Compressed blocks are skipped only once the spacing exceeds the block size. | Filtering, so detail aliases |
| Stored | Reduced data the file already holds | Control of the method: values depend on how the file was made |

```python
quick = image.read_slice("pixel.raw.intensity", window, level=8, level_method="decimate")

# Level 8: every 32nd pixel by striding, then three bilinear halvings: about a thousandth of the work
fast  = image.read_slice("pixel.raw.intensity", window, level=8, decimate_first=5)
```

A `decimate` Level n holds the same values as a Window `step` of 2ⁿ that starts on the grid. `step` remains for spacings that are not powers of two. A `step` above 1 and a Level above 0 cannot be combined; asking for both is an error.

**Stored levels.** Many files hold their own reduced data: TIFF overviews, NITF reduced-resolution sets, whole-slide pyramids, and the resolution levels of JPEG 2000. BIL names them `stored1`, `stored2` and so on, finest first, and keeps them apart from the Levels it computes.

```python
image.list_stored_levels()     # each with its name, size, scale, method and whether it is aligned
as_stored = image.read_slice("color.rgb.green", window, level="stored2")
one_more  = image.read_slice("color.rgb.green", window, level="stored2.level1")
```

- **Aligned or not.** A stored level is aligned when its samples sit exactly on the Level grid. Many do not: a typical TIFF overview averages pixels 2i and 2i + 1, so its samples sit half a pixel off.
- **Only an aligned one makes a Level.** `stored2.level1` is an error if `stored2` is not aligned. A misaligned level can still be read as it stands, as `stored2`, and its Slices carry their true `origin` and `scale`.
- **Never used unasked.** BIL substitutes stored data for computed data only when `level_source` is `Stored`, and then `plan` says which it used.

**Every Slice reports its Level name,** `level3`, `stored2` or `stored2.level1`, so a stored value is never mistaken for a computed one.

## 9. Missing data

> **Status:** Prototyped: NaN, both weightings, `missing_value`, stored NaN, and `pixel.mask`, checked against an independent reference. Fill values and corrupt blocks are specified.

**NaN marks a value with no data behind it.** That covers pixels never recorded, pixels never imaged, pixels a format marks with a fill value, pixels a bad-pixel map marks, and, with `corrupt_as_missing`, corrupt blocks. Every Slice is NaN there, whichever function made it, and the read's status is `Partial`, or `Missing` if every pixel is.

**In a Fragment** a missing pixel is marked in the Fragment's validity mask. A Fragment is still a full array, and the stored values at missing positions mean nothing.

**The mask as a Slice**

```python
read  = image.read_slices("pixel.mask", ["valid", "valid_fraction"], window, level=2)
read["valid"]            # 1.0 where a value exists, else 0.0
read["valid_fraction"]   # at coarser Levels, the share of contributing pixels that were valid
```

**Levels leave missing values out.** Each halving skips NaN and rescales the remaining weights, so a value is NaN only when every value that would contribute is missing. `missing_weights` chooses how the values below are weighted.

| `missing_weights` | Each present value below counts with | A Level is then |
| --- | --- | --- |
| `Equal`, the default | Its filter weight alone | The average of the present values of the Level below |
| `PixelCount` | Its filter weight times the share of present full-resolution pixels behind it | The average of the present full-resolution pixels under the whole filter, as if made in one step |

With every pixel present, both give exactly the values of an Image with no missing data. They differ next to gaps.

**A worked example,** along one row for clarity. Nine pixels, of which the first five are missing.

| Position | 0 to 4 | 5 | 6 | 7 | 8 |
| --- | --- | --- | --- | --- | --- |
| Level 0 | missing | 0 | 12 | 12 | 12 |

Level 1 at position 4 sees positions 3, 4 and 5, and only position 5 is present, so it is 0, with a quarter of its filter's pixels behind it. Level 1 at position 6 sees 5, 6 and 7, all present: (0 + 2 × 12 + 12) / 4 = 9. Level 2 at position 4 combines Level 1 at positions 2 (missing), 4 (weight 2) and 6 (weight 1).

- **`Equal`:** (2 × 0 + 1 × 9) / 3 = **3**. The value at position 4 counts double, though only one pixel lies behind it.
- **`PixelCount`:** (2 × ¼ × 0 + 1 × 1 × 9) / (2 × ¼ + 1) = **6**, which is exactly the average of the present pixels under the combined filter.

Use `PixelCount` where a Level must not be pulled toward the few pixels that happen to survive beside a gap, as in statistics over a coarse Level. `Equal` is the common case and the default.

**A number instead of NaN.** One NaN spreads through a Fourier transform or a convolution, so `missing_value` fills missing positions with a number.

```python
filled = image.read_slice("pixel.raw.intensity", window, missing_value=0.0)
filled.status      # still Partial
```

- **Filled last,** after the slice function and the Levels, so averaging still leaves missing values out.
- **Still reported.** The status stays `Partial`. Ask for `pixel.mask.valid` in the same request to know which positions were filled.
- **Not borrowed.** Filling writes new values.

**No cost where nothing is missing.** A driver says whether an Image can have missing pixels. Where it cannot, no checks are made at all, and the fast paths are unchanged. The experiments confirm it: an Image with no mask runs the same code as before missing data was added.

**Stored NaN.** In float data, a NaN stored at a pixel the mask calls valid is treated as missing, the same as a masked pixel. The experiments do this, and it is a proposal awaiting the author (chapter 24).
## 10. Fragments: the native pixels

> **Status:** Specified. The experiments hold whole Images in memory as interleaved native samples, as a Fragment would.

A Fragment is the stored pixels of a Window, unaltered: not converted, scaled or reordered. Read one when you need exact values, the native sample type, or data to pass to code that expects it.

```python
fragment = image.read_fragment(window)               # level0 by default
fragment.pixel_type     # sample type, component count, significant bits, bit shift
fragment.is_borrowed    # True if these are the stored bytes themselves
fragment.data           # a NumPy array of the native type, read-only when borrowed
coarse = image.read_fragment(window, "stored2")      # a stored level, as it stands
```

A computed Level is not a Fragment, because it holds new values, so `read_fragment(window, "level2")` is an error.

**Pixel types.** A pixel is `component_count` samples of one `SampleType`: `U8`, `I8`, `U16`, `I16`, `U32`, `I32`, `U64`, `I64`, `F16`, `F32`, `F64`, `ComplexI8`, `ComplexI16`, `ComplexF32` or `ComplexF64`.

- **Complex is a sample type, not two components,** so the component axis stays free for polarization channels: quad-pol SAR is 4 × `ComplexF32`.
- **Significant bits are recorded separately.** A 12-bit sensor is `U16` with `significant_bits` 12.
- **Samples stay where they are stored.** Where the value sits in the high bits of its word, as some hardware video decoders deliver it, `bit_shift` says how many low bits are unused. Every Slice works from the value: the stored word shifted right by `bit_shift`. Ten-bit video is 0 to 1023 in `pixel.raw`, whichever decoder made it.
- **One-bit data is unpacked** to `U8` with `significant_bits` 1, so it is never borrowed.

**Layout.** A Fragment is a strided view, with a row, a column and a component stride in bytes. Pixel-interleaved, line-interleaved and band-sequential storage are all described this way without moving any data.

**Borrowed, without a copy.** A Fragment borrows the stored bytes when all five hold:

1. The pixel data is uncompressed.
2. The byte order matches the machine.
3. Samples are byte-aligned, not bit-packed.
4. The storage is memory-mapped or in memory, and the Window's rows are evenly spaced in it.
5. Each sample starts at an address that is a multiple of its size.

Otherwise the Fragment owns a buffer decoded once. The type is the same either way.

- **Read-only under `Shared` mapping.** In Python a borrowed array is read-only. Open with `mapping=Private` to change it in your own memory, copy-on-write.
- **It keeps its Source open,** wherever it goes.
- **Do not change a file BIL has mapped.** Mapping a file another process changes or shortens is not supported, and on most systems can end the reading process. Use `mapping=Off` for files that may change and for unreliable network file systems.

**Deferred Fragments.** For video, reading a Fragment decodes nothing: it seeks and extracts the compressed packets from the preceding key frame to the frame asked for, and `is_deferred` is true. The frame is decoded when a Slice is made from it, or when you ask the Fragment for its pixels. A deferred Fragment is self-contained and decoded once, by its own Reader where that Reader is free, and otherwise by a decoder made for the purpose. A Window does not reduce video decoding, only the conversion to float32.

## 11. Frames, time and coordinates

> **Status:** Specified.

**A frame is an index,** the nth complete 2D array of an Image, counted from 0. Time is a coordinate attached to it. Reads always take indexes; to read by time, find the index first.

```python
from datetime import datetime, timezone

frame = luma.find_index("time", datetime(2026, 3, 1, 12, 0, 5, tzinfo=timezone.utc))
slice = luma.read_slice("video.planes.luma", bil.Window(0, 0, 1080, 1920, frame=frame))
```

**Why not compute the frame from the time?** Frames are dropped and some sensors vary their rate, so the time of a frame is looked up, never calculated.

**Coordinates**

| Index | Coordinates it can carry |
| --- | --- |
| Row and column | Ground position, through geocoding; time, for line-scan sensors and rolling shutters |
| Component | Wavelength for spectral bands, range for the bins of a waveform, polarization |
| Frame | Time; gate range for range-gated sensors |

`list_coordinates`, `read_coordinate` and `find_index` work on any of them. Coordinates are 64-bit, because 32 bits cannot hold an absolute time to useful precision. A coordinate is a metadata field attached to its index, and where a dictionary item means the same thing, it is that item: the wavelength coordinate is `common.band_center`.

**Frame count and the frame index.** Some streams carry no index, so the number of frames cannot be known without reading them all.

- **Opening never scans.** `count_frames()` may say `AtLeast` or `Unknown` at first.
- **The index is built on first need:** the first call that needs a `Known` count, a time coordinate, or a jump to a distant frame. Call `build_frame_index()` to choose the moment yourself.
- **Save it.** `write_frame_index(location)` saves the index, and `frame_index` in `OpenOptions` uses it next time. A saved index records the stream's size and a checksum of its start, and one that no longer matches is refused.

**Live streams.** A live stream's count is `AtLeast`, and its frames are numbered from the first received. All Readers see the same frames, a frame older than those still held is `Missing`, and `latest_frame()` gives the newest. Give every read a `timeout`, and a `cancel` handle if another thread may need to stop it.

**Nested cycles.** A sensor that sweeps 50 range gates once a second can declare further indexes, such as `sweep` and `gate`. They resolve to a frame number through `resolve_frame`, and a Window still carries one `frame`.

## 12. Readers, the cache and access patterns

> **Status:** Specified. Access patterns and read-ahead are new in specification version 68, and their details were settled in version 69.

**Readers hold decoder state.** A thread that reads an Image gets its own Reader, made on first use. A Reader keeps its place in a stream, so reading the next frame decodes one frame, not the whole run from a key frame.

```python
second = image.open_reader()       # a further Reader, holding its own place
here   = image.read_slice("video.planes.luma", bil.Window(0, 0, 1080, 1920, frame=1200))
there  = second.read_slice("video.planes.luma", bil.Window(0, 0, 1080, 1920, frame=9000))
```

A Reader offers every read call its Image offers. A call on the Image uses the calling thread's Reader.

**Threads**

| Object | Shared between threads |
| --- | --- |
| Source and Image | Yes |
| Reader | No: one thread at a time, and it may be handed to another |
| Fragment and Slice | Yes, once delivered |

In Python the interpreter lock is released during every read, so a thread pool reads in parallel.

**How far decoding runs in parallel** depends on the library underneath, and each driver declares it.

| `concurrency` | Means |
| --- | --- |
| `Parallel` | Readers read at once, with no lock. BIL's own Rust drivers. |
| `OneHandlePerThread` | Each Reader opens its own library handle, then reads in parallel |
| `Serial` | One call at a time in the whole process, as the HDF5 library requires. Conversion and slice functions still run in parallel. |

`plan_slice` reports the declaration for a read.

**The Fragment cache.** Compressed data decodes in whole blocks: a tile, a strip, a chunk or a frame. BIL keeps recently decoded blocks so that a block several reads share is decoded once: adjacent Windows over one tile, the margins BIL reads around a Window, a frame read twice.

| Call | Does |
| --- | --- |
| `set_cache_size(fragment_count)` | Sets how many decoded Fragments the Source keeps; 16 by default, 0 to turn the cache off |
| `set_cache_bytes(byte_count)` | Caps the bytes held; a quarter of physical memory by default |
| `describe_cache()` | The Fragments and bytes held, the reads served, and those served from blocks read ahead |
| `clear_cache()` | Frees every cached Fragment |
| `describe_held()` | Every kind of data the Source holds, with counts and bytes |

- **One cache for each Source,** shared by its Readers, freed when the Source closes.
- **Whichever limit is reached first applies,** so a wide area frame of several gigabytes cannot exhaust memory however large the count.
- **Borrowed data is never cached:** there is nothing to decode.
- **Safe to hold.** A Fragment you hold stays valid after its block leaves the cache.

**Access patterns.** Callers move through very large Images in one of two ways, and BIL reads faster when it knows which.

| `AccessPattern` | You | BIL |
| --- | --- | --- |
| `Random`, the default | Jump from place to place, as a viewer or a target search does | Reads only the blocks each read needs, keeps the cache in order of last use, and tells the operating system the access is random, so it does not read ahead either |
| `Sequential` | Move through adjacent blocks, normally forward in the order they are stored, as processing a whole Image Window by Window does | Reads and decodes the next blocks ahead of you on a background thread, merges adjacent reads, drops blocks you have passed first, and tells the operating system the access is sequential |

```python
batch = bil.open_source("mosaic.tif",
                        access_pattern=bil.AccessPattern.Sequential,
                        read_ahead_bytes=256 * 2**20)
image = batch.image(0)
image.info().storage        # tiles of 512 × 512, stored row by row

sweep = image.open_reader(access_pattern=bil.AccessPattern.Sequential)  # one Reader only
```

**What it never changes.** Values, statuses, sizes and errors are the same under either pattern. Only time and the memory held between reads differ, so choosing wrongly is never a fault, only slower.

**How read-ahead works**

- **It follows the storage.** Each driver reports how its blocks lie in the file, as a `Storage`; `ImageInfo.storage` gives its block size, interleave and whether the order is known. After each read, BIL reads the blocks that follow the last one in that order, for the same Image, Level and components, and for video the frames that follow. In band-sequential storage it reads ahead along each component your last read used. It runs forward only. Where the order is `Unknown`, BIL reads nothing ahead and gives the operating system the hint alone.
- **It is bounded.** It stops at `read_ahead_bytes` and never takes the cache past either of its limits: blocks read ahead count against the cache's count as well as its bytes, so with the cache off nothing is read ahead. With many small tiles, raise `set_cache_size` along with `read_ahead_bytes`.
- **It decodes too.** Compressed blocks read ahead are decoded into the cache, so your next read finds them ready. Uncompressed blocks are only fetched, or for mapped files the operating system is asked to bring them in.
- **It respects the driver.** Under `Serial` the background thread takes the library's one lock like any call, so it reads between your calls, not beside them.
- **Errors come with the read that needs them.** A block that fails while read ahead is tried again when you ask for it, and any error is reported then.
- **Passed blocks go first.** When the cache is full, a block wholly before your last read goes before any other. Blocks your last read shares with the next, such as the block row under two adjacent Windows, are kept.
- **It stops** when the Source closes, the Reader it reads ahead for is dropped, or a read is canceled.
- **One place for each Reader.** Each Reader reads ahead from its own place. The cache is shared, so a block counts as passed only when every Reader reading ahead has passed it.

**Choosing**

| You are | Use |
| --- | --- |
| Processing a whole Image in Windows, top to bottom | `Sequential`, with Windows that follow the storage: across a row of tiles, then down |
| Playing or processing video forward | `Sequential` |
| A viewer, a search, or random sampling for training | `Random` |
| Both at once on one Source | A Source with `Random`, and a Reader with `Sequential` for the sweep |

**Work with the storage, not against it.** `ImageInfo.storage` says how the file is laid out. A file stored in strips reads fastest in Windows the full width of the Image. A tiled file reads fastest in Windows that are whole tiles, or whole rows of tiles. A band-sequential file stores each band apart, so a request for every band of a Window touches as many places in the file as there are bands.

## 13. Performance

> **Status:** Specified. The pass is prototyped and measured (chapter 23).

**What BIL promises**

- **Opening reads no pixels.**
- **Memory between reads** is the cache, what is read ahead, and held metadata, each within limits you set.
- **No call needs memory in proportion to the whole Image,** unless you ask for the whole Image. Where a format makes that impossible, as with one compressed block covering a whole image, `plan_slice` says so before the read.
- **A read costs one pass over its Window.** Byte swapping, unpacking, component selection, normalization, conversion to float32, every Slice and every Level asked for, all happen in one fused loop on a few rows that stay in the processor cache.

**How fast.** BIL aims for the least cost each function's arithmetic allows. A function that only converts should run near the speed of a memory copy, and one that needs an arctangent or a square root for every pixel near the speed of that arithmetic. The experiments measure both (chapter 23).

**Access classes,** reported by `plan_slice`.

| Class | Means | Copies BIL makes |
| --- | --- | --- |
| `Borrowed` | The Slice is the stored float32 bytes, or a strided view of them | None |
| `Converted` | Integer or swapped data, converted once | The output only, or none when streamed |
| `Decoded` | Compressed data, converted as it leaves the decoder | The output, beyond the decoder's own buffer |

A local function or a filtered Level always computes new values, so it is never `Borrowed`. A `decimate` Level of borrowable data computes nothing, so it stays `Borrowed`.

**Advice**

- **Ask for everything at once.** Several Slices and several Levels in one request cost one pass.
- **Reuse buffers.** `read_slice_into` allocates nothing per call.
- **Stream large outputs.** `stream_slice` hands you row blocks sized for the processor cache, so the float32 form never fills main memory.
- **Use cheap relatives.** `power` avoids the square root of `magnitude`, and both avoid the arctangent of `phase` and the logarithm of `power_db`.
- **Say how you move.** `Sequential` for sweeps, and Windows that follow the storage.
- **Look before you read.** `plan` gives the sizes, the blocks, the access class and the remote requests.
- **Threads help up to a point.** Large Windows are split across threads, and gains flatten after a few cores, when memory bandwidth becomes the limit.
## 14. The slice functions

> **Status:** Specified. `pixel.raw`, `complex.polar`, `color.hsv`, `pixel.mask`, `class.labels` and the range of `depth.sensor` are prototyped.

A slice function turns the stored form of a sensor's data into the form analysis starts from. Each is defined for one representation, so an Image offers only the functions that fit it: `list_slice_functions()` says which.

**What belongs.** A function is in BIL when it undoes the storage (demosaicing, chroma, palettes, unpacking), calibrates (radiance, reflectance, backscatter, temperature), re-expresses the measurement in a standard form (color spaces, complex parts, Stokes terms), or presents it for display with statistics you pass in. Analysis, such as a vegetation index, is in the separate `bil-extras` package, under the same names. Work that needs the whole scene, such as speckle filtering or atmospheric correction, is outside BIL.

**The families,** in brief. Section 9 of the specification lists every function, Slice and parameter.

| Family | For | Functions |
| --- | --- | --- |
| `pixel` | Any data | `raw`, `scaled`, `reduce`, `weighted`, `pair`, `mask`, `nearest`, `integrate`, `peak`, `stretch` |
| `complex`, `sar`, `pol`, `insar` | Complex and SAR data | `complex.cartesian`, `complex.polar`, `complex.power`, `sar.calibrated`, `pol.channels`, `pol.pauli`, `pol.span`, `pol.pair`, `insar.interferogram` |
| `color`, `palette`, `video` | Color stills and video | `color.rgb`, `color.hsv`, `color.ycbcr`, `color.lab`, `color.luminance`, `palette.lookup`, `video.planes`, `video.rgb`, `video.fields` |
| `cfa`, `polcam` | Mosaic and polarization cameras | `cfa.mosaic`, `cfa.demosaic`, `cfa.sites`, `cfa.binned`, `polcam.angles`, `polcam.stokes`, `polcam.linear` |
| `spectral`, `thermal` | Multispectral, hyperspectral, thermal | `spectral.bands`, `spectral.radiance`, `spectral.toa_reflectance`, `spectral.surface_reflectance`, `spectral.brightness_temperature`, `spectral.integrate`, `spectral.response`, `thermal.calibrated` |
| `elev`, `class`, `depth`, `nuclear` | Other rasters | `elev.height`, `class.labels`, `class.members`, `depth.sensor`, `depth.corrected`, `depth.points`, `depth.waveform`, `nuclear.activity` |

**Pointwise and local.** Most functions use one pixel. A few, marked local, read a small fixed neighborhood: demosaicing, chroma upsampling, mosaic sites, polarizer angles and interlaced fields. BIL reads the margin around your Window itself and mirrors at the Image edge by whole periods of any pattern, so you never pad a Window and the result is still the same as from the whole Image.

**Decibels sit beside linear values,** as `power_db` beside `power`, in the same function.

**Units are stated, never named:** `temperature`, not `temperature_k`.

**Color.** BIL describes color by the four properties of ITU-T H.273: `color_primaries`, `transfer_characteristics`, `matrix_coefficients` and `video_full_range`. Each comes from the file where it says, and otherwise from a stated default: for stills, BT.709 primaries with a D65 white point, the sRGB curve, the BT.601 matrix and full range; for video, BT.709 throughout and the reduced video range. `describe_field` tells you which. Embedded color profiles are exposed as fields and never applied; color management is a layer above BIL.

**When a function is not offered.** A function is offered only when every input it needs is available. `list_required_fields(function_name)` tells you what is missing, and a companion file can supply it (chapter 16).

```python
image.list_required_fields("spectral.toa_reflectance")
# [common.sun_elevation: available, Derived; common.earth_sun_distance: available, Derived; ...]
```

**Precision.** Every Slice is float32. It holds every value of `U8`, `I8`, `U16`, `I16`, `F16`, `F32` and the complex types up to `ComplexF32` exactly, 32 and 64-bit integers exactly up to 16,777,216, and `F64` to about seven digits. Functions compute in float64 for the wide types and narrow once at the end. For exact wide values, read the Fragment.

## 15. Normalization and calibration

> **Status:** Specified.

Normalization turns stored values into calibrated ones, inside a slice function, by a known function whose parameters come from the file's metadata or from you. A Fragment is never normalized.

| Function | Form | For |
| --- | --- | --- |
| `gain_offset` | a × value + b, per component | Radiance, scale and offset, black level |
| `polynomial2d` | A polynomial in row and column | SAR scale factors that vary with range |
| `interpolated_grid` | A coarse grid of factors, interpolated | Calibration vectors, flat fields, vignetting |
| `lookup_table` | Value to value | Palettes, amplitude tables |
| `matrix` | A linear combination of components | Band synthesis. Color is converted only by the color functions. |
| `planck` | The inverse Planck law | Brightness temperature |
| `pixel_gain_offset` | A gain and offset for every pixel, from a calibration Image | Infrared non-uniformity, flat fields, dark frames |
| `log10` | The base-10 logarithm; zero or less becomes missing | Logarithmic data |
| `value_polynomial` | A polynomial in the stored value | Detector linearity |

- **Pointwise.** It uses the pixel's values, its position, its frame's coordinates and the same position in a calibration Image.
- **Parameters are fields,** filled like dictionary items: yours first, then the file's, then a derivation, then a companion file.
- **Bad pixels are missing data.** A bad-pixel map makes them NaN in every Slice.

## 16. Metadata, the dictionary and domain profiles

> **Status:** Specified. Domain profiles are new in specification version 68.

**Fields.** Metadata is named fields with values. A name is a namespace, then the native name, spelled as the file or its specification spells it: `nitf.file_header.FTITLE`, `dicom.PatientName`.

```python
image.list_field_groups()                          # ["nitf.image_subheader", "nitf.RPC00B", ...]
image.list_fields("nitf.image_subheader")
title = source.read_field("nitf.file_header.FTITLE")
title.value, title.unit, title.origin              # origin Specified
```

| Part of a field | Says |
| --- | --- |
| Value | Text, integer, float, boolean, date and time, bytes, a list, or a group of fields |
| Unit | Where the file or its specification states one |
| Origin | `SelfDescribed` (the file names it), `Specified` (the format's specification names it), `Derived`, `Companion` or `Caller` |
| Attachment | A Source, an Image, or one index, such as a frame |
| Handling | `Ordinary`, `Marking` or `Identity` (chapter 18) |

**Names in detail**

- **Repeats carry a number from 1:** `nitf.RPC00B[2]`, `dicom.ReferencedImageSequence[1].ReferencedSOPInstanceUID`.
- **Dots inside a part are escaped:** `hdf5.calibration.band\.1`. Every call also takes a name as a list of parts.
- **Text is Unicode,** and `read_field` still carries the stored bytes.
- **A date and time says whether its zone is known.** Without one it is local time, never silently taken as UTC.
- **Nothing is hidden.** A value with no definition is listed under its native code, as bytes.

**More ways to read**

| Call | Gives |
| --- | --- |
| `read_field_at(field_name, index)` | The value at one position of the index the field attaches to, such as one frame. For a field with times of its own, such as KLV, the latest value at or before that position, with its time. |
| `find_fields(pattern, extract)` | The names that match, among those defined and the groups already extracted; with `extract` true, every group is extracted first |
| `list_markings()` | The fields flagged as markings, on a Source, an Image, a Fragment or a Slice (chapter 18) |

**Read when asked, then held.** Opening extracts only what describes the Images. A group is extracted the first time you ask for any field in it, each value is decoded the first time it is read, and extracted groups stay with the Source until it closes or you call `release_metadata`. Values that vary by frame, such as KLV, are extracted for the frames you ask about, and held for at most `max_held_positions` frames.

**The dictionary.** BIL keeps a dictionary of essential items, in the `common` namespace, that every format offers under the same names as far as it can.

```python
source.read_field("common.collection_time")        # NITF IDATIM, the KLV time, Exif, or frame 0
image.read_field("common.ground_sample_distance")  # provided, or derived from the geo model
image.describe_field("common.sun_elevation")       # its origin, and the formula and inputs behind it
```

Each item gets its value from the first of these that works.

| Order | How | Origin | Example |
| --- | --- | --- | --- |
| 0 | You add it, with `add_field` | `Caller` | A value you know better than the file |
| 1 | **Provided:** a native field that is plainly the same thing, one to one, at most a change of unit | As the native field | NITF `IDATIM` is `common.collection_time` |
| 2 | **Derived** by a stated formula, the same for every format | `Derived` | `common.sun_elevation` from the time and the center position |
| 3 | **From a companion file** beside the Source | `Companion` | A value written for a format that cannot hold it |

An item with no value is delivered with status `Missing`. An unknown name is an error.

**Domain profiles.** Each kind of work needs a known set of metadata, whatever sensor or format the data came from. Calibrating SAR backscatter needs the radar wavelength, the polarization and the incidence angle; turning optical counts into reflectance needs the band centers, the calibration, the sun elevation and the Earth–Sun distance. A profile names that set as dictionary items, each, as a `ProfileNeed`, `Required` or `Recommended`.

```python
bil.list_profiles()           # provenance, geolocation, optical_radiometry, thermal, sar, ...
bil.describe_profile("sar")

report = image.check_profile("sar")
report.status                 # Complete, Partial or Missing, from the required items only
for item in report.items:
    print(item.name, item.need, item.status, item.origin)
```

| First profiles | For |
| --- | --- |
| `provenance` | What collected the data, when, and how it was processed |
| `geolocation` | Where the data is, in what reference system, and how accurately |
| `optical_radiometry` | Radiance and reflectance from optical bands |
| `thermal` | Radiance and brightness temperature |
| `sar` | Calibrated backscatter and SAR geometry |
| `motion_imagery` | Video from a moving platform: rate, sensor position and pointing |
| `photography` | Camera stills: exposure, aperture, focal length, color |
| `medical` | What the data is and where it sits in the patient, never whom it is of |
| `depth_sensing` | Range and depth cameras: scale, intrinsics, units |

- **Format-free.** A profile names dictionary items only, so one profile applies to every format, and each format fills the items as it can.
- **Built from each other.** A profile's required or recommended list can name another profile, whose items then count with that need: `sar` requires `provenance` and `geolocation` rather than repeating their items, and `thermal` recommends `geolocation`.
- **Checked, never enforced.** `check_profile` reports. It never refuses a read or holds a function back; what a function needs is its own declaration, which `list_required_fields` reports.
- **Gaps can be filled.** The report names each missing item and whether it could have been derived. Supply it with `add_field` or a companion file, and check again.
- **Yours too.** A layer above BIL can state its needs once as a profile, and check every Source before reading a pixel.

```python
bil.add_profile({
    "name": "change_detection",
    "description": "Two collections of one place, compared",
    "required": ["provenance", "geolocation", "common.ground_sample_distance", "common.view_elevation"],
    "recommended": ["common.sun_elevation"],
})
bil.load_profiles("profiles/our_team.json")      # the same form, from a file
luma.check_profile("motion_imagery", 1200)        # items that vary by frame, at one frame
```

**Companion files.** A companion file holds metadata a file does not, beside it, named for the file with `.metadata` added: one `name = value` line for each item, in the dictionary's units.

```text
common.sun_elevation = 41.7
common.earth_sun_distance = 0.9934
```

`write_companion(fields)` writes one without changing the Source. For a Source opened as untrusted, a companion's values are used only when `trust_companion` is set, since a file beside an outside file comes from outside too; markings in it are always reported.

**Adding definitions.** A coded field, such as a new NITF extension or a vendor's private DICOM tags, is named by a field definition. Add them without rebuilding BIL, with `add_field_definitions` or `load_field_definitions` (chapter 20).

**Local times.** Exif records times on the camera's clock, with no zone. BIL takes such a time as local at the place of collection, and finds the zone from a stated offset first; then, only where an optional time-zone lookup plugin is installed, from the sensor position or the footprint; and otherwise leaves it marked as having no known zone. `describe_field` says which.

## 17. Geocoding and accuracy

> **Status:** Specified.

**A geo model is a chain of steps** from Image pixels to a world space. Every format's model is built from the same steps, so the calls are the same for a map-projected GeoTIFF, an RPC-carrying NITF, a SAR image and a video frame.

```python
image.list_geo_models()                       # most rigorous first: ["rational_polynomial", "projective"]
model = image.read_geo_model("rational_polynomial", 0)
chain = model.for_slice(overview)             # adds BIL's image_window step for this Slice
where = chain.image_to_world([(0.0, 0.0), (511.0, 511.0)])
back  = chain.world_to_image([(where[0].lat, where[0].lon, where[0].height)])   # fields assumed (M5)
```

`for_window(window, level)` builds the same chain for a computed Level before any Slice is read, and `list_steps()` lists each step of a chain with its spaces and parameters.

**Steps.** `affine`, `projective`, `polynomial`, `rational_polynomial`, `coordinate_grid`, `tie_points`, `camera`, `pose`, `range_doppler`, `polar`, `surface`, `reference_change`, `sphere_rotation`, `external` and `image_window`. A step runs both ways, by iteration where it has no direct inverse.

**Rules worth knowing**

- **BIL adds its own first step.** `image_window`, from the Slice's `origin` and `scale`, covers the Window, the Level, any stored level and any `step`. Locate a pixel of any Slice without adjusting anything.
- **Whole-number positions are pixel centers.**
- **Image to world needs a surface.** A pixel gives a line of sight, not a point. With no surface given, BIL uses the model's reference height, or zero above the ellipsoid, and says which.
- **The first surface met wins,** the point the sensor saw.
- **A position is returned only when it is sound.** Each carries a status: `Complete`, `Missing` (no surface there), `NotConverged` or `Ambiguous`.
- **Positions are 64-bit,** never Slices.
- **Longitude wraps,** so chains work across the antimeridian and over the poles.
- **Several models.** An Image can carry several, listed most rigorous first and named for their main step, such as `rational_polynomial_1` and `rational_polynomial_2`. The format's own name is kept as each model's label.

**Terrain**

```python
dted  = bil.open_terrain("dted/")                       # a file, a folder of tiles, or a remote store
where = chain.image_to_world(points, dted)
flat  = bil.constant_terrain(250.0)                     # meters above the ellipsoid
```

A `Terrain` also answers directly: `height_at(positions)` gives heights above the ellipsoid, `intersect(lines_of_sight)` where each line first meets the surface, and `coverage()` the area and post spacing it covers. `caller_terrain(function)` makes a surface from your own function, for terrain BIL does not read.

Heights come out above the ellipsoid, converted from a geoid by PROJ using local grids. Voids fall back to a coarser terrain if given, then a constant height, and otherwise the position is `Missing`. Terrain tiles are read through BIL's own drivers and cached like any Fragment. No terrain is ever fetched unless you open it.

**Extending a chain.** `concatenate` adds a step or a chain to either end, such as a `reference_change` to a map projection made with `make_step("reference_change", target="EPSG:32611")`, or a correction from a registration layer above BIL.

**Accuracy.** Every position can carry a full record of how accurate it is, carried through every step of the chain and never reduced to one number until you ask.

```python
where = chain.image_to_world(points, dted, accuracy=True)
acc   = where[0].accuracy
acc.status                    # Complete, or Partial with the unknown steps named
acc.covariance                # 3 × 3, east, north and up, in meters
bil.circular_error(acc, 0.9)  # CE90, computed exactly from the covariance
bil.linear_error(acc, 0.9)    # LE90
bil.error_ellipse(acc, 0.9)

chain.relative_accuracy(points_a, points_b)   # shared bias cancels
chain.list_error_sources()                    # each contribution, with its kind and provenance
```

| Part of the record | Holds |
| --- | --- |
| `covariance` | The full covariance of the position |
| `contributions` | One `Contribution` for each source of error: its `name`, such as `rational_polynomial.error` or `surface.height`; its `kind`, `Bias`, `Random` or `Correlated`; its own `covariance`; and its `provenance`, `Stated`, `Derived`, `Given` or `Assumed` |
| `status` | `Complete` when every step has an estimate, `Partial` when some have none, `Missing` when none has |

- **Kinds are kept apart** because they behave differently: a bias cancels between two points of one Image, a random error does not, and a correlated error mostly cancels between nearby points.
- **Unknown is never zero.** A step with no estimate is named, and the record is `Partial`.
- **First order by default.** `accuracy_method="sampled"` checks the result by propagating points through the chain itself, for strongly curved geometry such as grazing lines of sight.
- **Your own error too.** `make_step` takes a `covariance`, so a correction you add carries its accuracy forward.
- **Written with the model.** Writers write the estimates where the format has a place, and otherwise to the companion file.

## 18. Writing, markings and identity

> **Status:** Specified.

BIL writes TIFF and GeoTIFF (uncompressed or Deflate, including BigTIFF), ENVI, and NumPy `.npy`, through writers of its own.

```python
out = bil.create_source("result.tif", "geotiff", info)    # info: an ImageInfo
for window in windows:
    out.write_slice(window, image.read_slice("pixel.raw.intensity", window), "intensity")
out.close()
```

| Call | Does |
| --- | --- |
| `list_writers()` | The formats BIL can write |
| `create_source(location, format_name, image_info)` | Starts a new file; takes `WriteOptions` |
| `write_fragment(window, fragment)` | Native pixels for a Window |
| `write_slice(window, slice, component)` | Float32 values for a Window, into one component |
| `write_geo_model(model)` | A geo model other than the one written by default |
| `write_metadata(fields)` | Fields the format can hold |
| `close()` | Completes the file |

- **Written by Window,** so a large image never has to be held whole.
- **The geo model is written without being asked,** including the `image_window` step of what was written. A Slice from a coarse Level is written on its own grid and located correctly.
- **Sample types are never converted silently.** Writing a type the format cannot hold is an error: ENVI holds no `I8`, `F16`, `ComplexI8` or `ComplexI16`, and `.npy` no complex integers.
- **New files only.** BIL never changes a Source it opened for reading.
- **The input is recorded:** the driver and codec that read it, as `bil.driver` and `bil.codec`.
- **A file BIL writes reads back** with identical pixel values.

**Markings and identity**

| Kind | Examples | When writing | You can |
| --- | --- | --- | --- |
| `Marking` | Security classification, handling caveats, releasability | Always written: inside the file where it has room, and to the companion file | Turn it off with `keep_markings=False` |
| `Identity` | A patient's name, identifier and date of birth | Left empty | Keep it with `keep_identity=True` |

- **Carried without being asked.** Every Fragment and Slice keeps a reference to its Source's markings, and the writers write them whether or not you call `write_metadata`.
- **A missing companion is noticed.** A file BIL writes records that a companion belongs to it, and reading it without one gives `Partial` and a warning.
- **Refuse rather than lose.** `require_internal_markings=True` makes writing a marked Source to a format with no place for markings an error.
- **Not interpreted.** BIL does not rank or combine markings; from several Sources, all are written.
- **Classification is never a dictionary item.** Schemes differ between formats and nations, and a wrong mapping does harm, so markings stay in their native form.
- **The limit.** BIL empties only fields its definitions flag as identity; identity can also sit in free text and in the pixels. Once an array leaves BIL as plain data, keeping its markings is your task.
## 19. Safety and containment

> **Status:** Specified.

BIL is used on closed networks and with data that must not leak, and imagery files often come from outside. Two rules and four measures follow.

**No network unless asked.** BIL connects only to read a remote Source, a stream or a remote terrain store you named. It fetches no default terrain, grids or updates, and reports nothing about its use. A reference inside a file is reported, and followed only if you allow. PROJ's grid downloading is off, and XML parsing fetches nothing.

**No copy unless asked.** BIL writes to storage only for an output file, a companion file, a saved frame index, or the mapped data of external decoding with its lock file and control channel. It creates no temporary files and no cache files: the cache, blocks read ahead and held metadata are in memory only.

**The limit.** A plugin or a scripted driver is code you chose to load, and BIL cannot hold it to these rules. `list_drivers` reports every plugin loaded, with its license.

**For files from outside**

```python
scene = bil.open_source(path, untrusted=True)             # default limits, and isolation if configured
scene = bil.open_source(path, untrusted=True, max_image_pixels=4 * 10**10)   # raise a limit
```

| Measure | What it does |
| --- | --- |
| Limits | `max_image_pixels`, `max_decoded_bytes` and `max_field_group_bytes` refuse impossible sizes before any memory is taken. None are set for a trusted Source, because very large images are BIL's purpose; an untrusted Source gets defaults scaled to the file. A header claiming more uncompressed pixels than the file can hold is always rejected. |
| Configured plugin locations | External, optional and scripted plugins load only from locations you or the installation's configuration name. BIL never searches the working directory, the library path or beside the data. |
| Isolation | C and C++ drivers and codecs can run in a separate decoder process: `Off` by default, `Untrusted` for Sources opened as untrusted, or `Always`. Set once in the installation's configuration. It needs a place for mapped data, and if none is configured the open fails rather than running the driver in your process. |
| Fuzz testing | BIL's own Rust readers, parsers, the mapped driver and the companion file reader are fuzz tested as a condition of release |

**What isolation costs.** An isolated read cannot borrow from the original file, passes each decoded block through shared memory, and pays a round trip between processes. It decodes only what each read needs. It protects your process; a decoder that has been taken over can still return wrong pixels.

**Audit.** `list_drivers()` gives every library and plugin with its name and version, where it came from (BIL's package, a crate, the package manager, or a plugin you installed), and its license as built, and lists a refused plugin as not loaded, with the reason. `describe_decoding()` gives the driver and codec a Source used, and every file BIL writes records them.

## 20. Extending BIL

> **Status:** Specified.

**Kinds of plugin.** All register through one call, `register_plugin`, with a description of the plugin and its kind. `bil_register_driver`, `register_slice_function`, `register_level_method` and the scripted `register_driver` are short forms of it.

| Kind | Adds |
| --- | --- |
| Container driver | A format: opens Sources, lists Images, produces Fragments |
| Codec | Decoding of compressed blocks, for every container that uses them |
| Stream source | Frames from video files, network streams and cameras |
| Metadata parser | Named fields from embedded metadata |
| Geocoding step | A step, such as a Community Sensor Model plugin as an `external` step |
| Storage backend | Bytes from files, memory or object stores |
| Time-zone lookup | Zones from positions, for local times |
| Level method | A named downsampling method |
| Scripted driver | A format's headers and extra data, in Python, with pixels left to the core |

**What a driver provides.** Recognize the format (`probe`), open it, describe its Images (`list_images`), describe how its blocks are stored (`describe_storage`), read Fragments (`read_fragment`), hold decoder state (`open_reader`), declare its `concurrency`, and expose metadata and geocoding. A driver never implements slice functions: the core makes every Slice from its Fragments, so a new driver gains every applicable function at once. Optionally it can describe its pixels for the core to read, add slice functions under its own family, add additional records, or emit float32 rows directly where that reproduces the core's result exactly.

**A driver in C, C++ or Rust,** built separately, is a shared library that calls `bil_register_driver` through the C interface, with the plugin interface version it was built against. A plugin of another major version is refused and listed as not loaded, with the reason.

**A scripted driver.** For an esoteric or evolving format, write the format-specific part in Python and leave the pixels to the compiled core.

```python
class VendorDriver(bil.ScriptedDriver):
    name = "vendor"

    def probe(self, location):
        return bil.Confidence.High if location.read_bytes(0, 4) == b"VNDR" else bil.Confidence.NoMatch

    def list_images(self, source):
        self.header = parse_header(source.read_bytes(0, 512))
        pixel_type = bil.PixelType(bil.SampleType.U16, 1, 12)
        return [bil.ImageInfo(self.header.row_count, self.header.col_count, pixel_type, ["intensity"])]

    def describe_pixels(self, source, image_index):
        return bil.PixelLayout(offset=512, byte_order="big", interleave=bil.Interleave.Pixel)

bil.register_driver(VendorDriver)
```

- **Pixels by description.** `describe_pixels` returns a `PixelLayout`: offset, byte order, interleave, row stride, blocks with their offsets, codec and validity. The core reads from it, with the zero-copy rule, Slices and Levels unchanged.
- **The slow path is reported.** A script that returns pixels as an array instead is interpreted and slower, and `plan_slice` says so.
- **CPython, from the machine.** Scripts run in the Python installed on the machine. Inside Python, BIL uses that interpreter. From Rust, C and C++, it runs in a BIL helper process started on first use, and each call into the script is a round trip between processes, made when a Source is opened and when additional data is read, never for pixels a layout describes.
- **Additional data.** A script can expose extra records by name, such as calibration tables or per-line records, and callers read them with `list_auxiliary()` and `read_auxiliary(name)`. A script can be attached to a compiled driver for data that driver does not read.
- **Code is trusted only by choice.** Scripts load only when you register them or name their folder.

**A slice function** gives its two-part name in a family it may use, the representations it accepts, its Slice names (listed, or a rule over the `ImageInfo`), the kind of each Slice (linear, angle, label or range), its radius, its parameters, the fields it needs, and its kernel. BIL calls the kernel for each block of rows with its inputs already in float32 and its margins read, and does everything else: reading, Levels, delivery and threads. A registered kernel runs as a call for each row block, slower than the core's compiled loop but with every other guarantee.

**A level method** gives its name, weights, radius and alignment. Its weights must be fixed and tied to the Image grid, so results stay independent of the Window.

**Field definitions** name coded fields without rebuilding BIL. Each record gives the native `name` as a list of parts, a `locator` in the format's own terms, a `value_type`, an `encoding`, a `unit`, a `handling`, and the dictionary item it `provides`, if it is plainly the same.

```json
{"name": ["nitf", "VNDRTRE", "GAIN"],
 "locator": {"extension": "VNDRTRE", "offset": 12, "length": 6},
 "value_type": "float", "encoding": "ascii_decimal", "unit": "dB",
 "handling": "Ordinary", "provides": null}
```

**Domain profiles** are added with `add_profile` or `load_profiles` (chapter 16).

**External decoding.** By configuration, any supported format can be decoded in a separate process into BIL's own memory-mapped format, and read from there with every Fragment borrowed. It serves fragile or slow decoders, decoders that cannot load into your process, such as one written in Java, one decoding shared by several processes, and isolation. Your code does not change. The configuration names a folder or shared memory, and there is no default, so nothing is decoded externally until one is chosen.

## 21. Errors and statuses

> **Status:** Specified.

**A call either delivers or fails.** A call that cannot deliver returns an error and no value: a file that cannot be opened, a bad argument, a failed read, a limit exceeded, or corrupt data. In Python an error is an exception, in Rust an `Err`, and in C a status code with the message from `bil_last_error`.

**What is delivered carries a status**

| Status | For pixels | For positions | For accuracy | For fields |
| --- | --- | --- | --- | --- |
| `Complete` | No pixel is missing | The position is sound | Every step has an estimate | The value was found |
| `Partial` | Some pixels are missing, and marked | Not used | Some steps have none, and are named | Some items of a group could not be read |
| `Missing` | Every pixel is missing | No surface there | No step has an estimate | No value at that index |
| `NotConverged` | Not used | The inverse did not converge | Not used | Not used |
| `Ambiguous` | Not used | More than one answer, as in the fold of a grid | Not used | Not used |

A profile report uses the same three statuses as accuracy, counted over its required items.

**Common cases**

| You see | Because | Do |
| --- | --- | --- |
| An error naming a field | The name is not defined for this Source | Check `find_fields` or `list_fields` |
| A field with status `Missing` | It is defined but has no value here | Supply it with `add_field` or a companion file, if you know it |
| A function not offered | An input it needs is missing | `list_required_fields` names it |
| An error on a corrupt block | Corrupt data is an error by default | Open with `corrupt_as_missing=True` to get `Partial` instead |
| An error on `stored2.level1` | `stored2` is not aligned with the Level grid | Read `stored2` as it stands, or a computed Level |
| An error on a Level with `step` above 1 | They cannot be combined | Use `decimate` for powers of two |
| A frame with status `Missing` from a live stream | It is older than the frames still held | Read sooner, or keep up with `latest_frame` |
| A read that fails after a time | `timeout`, or the remote timeout and retries, ran out | Raise them, or check the stream |

**A failure never becomes a crash.** Failures inside BIL are caught at the interface and returned as errors. A C callback must not let an exception or a `longjmp` cross into BIL.

## 22. Python, Rust and C

> **Status:** Specified.

The same calls exist in every language, with each language's own conventions.

**Python**

```python
import bil

source = bil.open_source("scene.ntf")
slice  = source.image(0).read_slice("complex.polar.magnitude", window)
slice.data                          # a NumPy array over the same memory, read-only when borrowed
```

- **No copy.** Slices and Fragments are NumPy arrays over BIL's memory, including strided and borrowed ones.
- **The interpreter lock is released** during every read.
- **One build for several Pythons:** the package uses Python's stable interface.

**Rust**

```rust
let source = bil::open_source("scene.ntf")?;
let image  = source.image(0)?;
let slice  = image.read_slice("complex.polar.magnitude", window)?;
let values: &[f32] = &slice.data;   // borrowed where the source is already float32
```

- **Threads are checked by the compiler.** Sources, Images, Fragments and Slices are `Send` and `Sync`; a Reader is `Send`.
- **Static linking is allowed** by the license's exception.

**C**

```c
BilSource *source;
BilImage  *image;
BilSlice  *slice;
BilStatus status = bil_open_source("scene.ntf", &source);
if (status == BIL_OK) status = bil_image(source, 0, &image);
if (status == BIL_OK) status = bil_read_slice(image, "complex.polar.magnitude", &window, &slice);
if (status == BIL_ERROR) {
    char message[512];
    bil_last_error(message, sizeof message);
}
/* BIL_PARTIAL: delivered, with missing pixels marked */
const float *values; uint32_t rows, cols; ptrdiff_t row_stride;
bil_slice_data(slice, &values, &rows, &cols, &row_stride);
bil_free_slice(slice);
bil_close_source(source);
```

| Convention | Rule |
| --- | --- |
| Handles | Every object is an opaque handle, read through accessors |
| Status | Every function returns a `BilStatus`, and results come back through the last arguments |
| Ownership | What a call creates, free with the matching `bil_free_` call. A handle from another is valid while its parent is. |
| Data | An accessor gives a pointer, the size and the strides, valid until the object is freed |
| Errors | `bil_last_error` gives the message for this thread's last failed call |

- **Names carry a prefix:** `bil_` on functions, `Bil` on types, `BIL_` on enum values in upper case.
- **Strings are UTF-8,** ending with a zero byte.
- **Options are structs whose first field is their size,** so new options never break old callers.

**Stability.** The crate and the shared library follow semantic versioning. The C interface changes incompatibly only at a major version, and the plugin interface carries its own version.
## 23. What runs today

> **Status:** Built (the `bil` crate's vector layer and kernels) and Prototyped (experiments 1 and 1b).

Two pieces of code exist. Neither reads a file: they work on Images already in memory, which is the part of BIL whose speed and correctness decide whether the rest of the design holds.

**The `bil` crate** (`crates/bil`, version 0.0.1) holds the vector layer and the kernels.

- **`simd`:** a `Vf` trait over vectors of float32, with `Scalar`, `Avx2`, `Avx512` and `Neon` back ends, and `Isa`, which finds the instruction sets the processor offers. NEON is tested only in CI, on Linux and macOS on ARM, where it agrees bitwise with scalar code; its speed has not yet been measured.
- **`kernels`:** the arithmetic of the pass. `halve` and `halve_row` make each `bilinear` halving, and `blur5` and `blur5_row` each `binomial5` halving; `halve_weighted`, `halve_row_weighted`, `halve_over` and their `blur5` counterparts the halvings with missing data; `min3_present` and `min3_row` the range reduction; `polar0`, `phase_from`, `hsv0` and `hue_from` the slice functions; and `atan2` and `cos_sin_turns` the transcendental functions, BIL's own, built only from correctly rounded operations.

The kernels are `unsafe` and work on raw rows. They are BIL's internals, not its interface, and callers will never use them directly.

**Experiment 1b** (`experiments/exp1-fused-loop`) is the rolling pass of specification section 12: every Slice and every Level of a request in one pass, on a ring of three rows for each Level, written straight into the caller's buffers.

```rust
use bil_exp1::{Func, Image, LevelMethod, MissingWeights, Pass, ReadOptions, Samples, Window};
use bil_exp1::simd::Isa;

let image = Image { rows: 4096, cols: 4096, comps: 1, samples: Samples::U16(stored), valid: None };
let window = Window { row_start: 0, col_start: 0, row_count: 2048, col_count: 2048 };

let mut pass = Pass::new(&image, Func::Raw, window, 4);   // plan Levels 0 to 4, once
// Pass::for_method(&image, Func::Raw, window, 4, LevelMethod::Binomial5) plans for binomial5
let mut out  = pass.outputs();                            // allocate the Slices, once
let options  = ReadOptions {
    level_method: LevelMethod::Bilinear,
    missing_value: None,
    missing_weights: MissingWeights::Equal,
};
pass.run(&image, Isa::best(), options, &mut out);          // fill them; allocates nothing
let level2 = &out[2][0];                                  // Level 2, the first Slice
level2.at(10, 20);                                        // the value at grid row 10, column 20
```

| Experiment type | Specification counterpart | Difference |
| --- | --- | --- |
| `Image { rows, cols, comps, samples, valid }` | An Image's Fragment | The whole Image in memory, interleaved. `valid` is the validity mask, one byte a pixel; `None` means nothing can be missing. |
| `Samples`: `U8`, `U16`, `F32`, `ComplexI16`, `ComplexF32` | `SampleType` | Five of the fifteen |
| `Func`: `Raw`, `Polar`, `Hsv`, `Mask`, `Labels`, `Range` | `pixel.raw`, `complex.polar`, `color.hsv`, `pixel.mask`, `class.labels`, `depth.sensor` (its range) | Six functions, by enum rather than by name |
| `Window { row_start, col_start, row_count, col_count }` | `Window` | No `step` and no `frame` |
| `Pass::new`, `Pass::outputs`, `Pass::run` | `plan`, `read` with `IntoBuffers` | The plan is an object the caller keeps |
| `BandedPass::new(..., threads)`, `outputs`, `run` | A read split across threads (section 12, rule 7) | Bands of rows, each its own `Pass` on its own thread; starting the threads allocates for each read |
| `ReadOptions { level_method, missing_value, missing_weights }` | `SliceOptions` | No `level_source` or `decimate_first`; `LevelMethod` is `Bilinear`, `Binomial5` or `Decimate`. A `Pass` is planned for its method, since `binomial5` reaches further. |
| `Slice { name, level, rows, cols, data }` | `Slice` | `rows` and `cols` are the Level's grid indices covered, not an `origin` and `scale` |
| `Vec<Vec<Slice>>`, by Level then Slice | `read_slice_levels` | Levels 0 to `top`, or `first` to `top` with `Pass::for_levels`, not chosen by name |
| `Pass::pixels_read()` | What `plan` reports of a read's cost | The pixels the last run converted |

**What it shows**

- **The same bits as the whole Image.** About 580 million values over 2,400 random Windows at Levels 0 to 4, on scalar, AVX2 and AVX-512 code here and NEON in CI, and every Window of 25 small Images from 1 × 1 to 9 × 9 pixels, agree to the bit with the Image read whole and across instruction sets. The specification asks only for agreement to a tolerance; the bits came at little cost.
- **Missing data as the specification states it,** with both weightings, checked bitwise against an independent reference and against a float64 computation of the rule.
- **Each kind of Slice** reduced by its own rule, and `decimate` for every kind.
- **Coarse Levels alone.** `decimate` at Level 4 alone reads 1 pixel in 256 and takes 0.017 times a copy; every value equals the same Level of a full read.
- **Threads by bands of rows,** with every value equal to one thread's, bit for bit; 1.65 to 1.84 times as fast on two threads, over two runs.
- **`binomial5`,** bitwise against a whole-plane reference with and without missing data, and at a quarter of the amplitude at the halved grid's highest frequency, against `bilinear`'s half, as the specification states. It cost 0.9 to 1.3 times `bilinear`.
- **Speed.** On the fastest instruction set, `pixel.raw` takes 1.7 times a copy into warm memory at Level 0, and `complex.polar` and `color.hsv` 1.4 to 1.7 times their own arithmetic alone. Missing data costs 1.2 to 1.8 times an unmasked read, and nothing at all where no mask exists.

**Running it**

```sh
cd experiments/exp1-fused-loop
cargo run --release --bin exp1b -- --check   # every correctness check, under a minute
cargo run --release --bin exp1b              # the checks and the speed tables
cargo run --release --bin exp1 -- --check    # experiment 1's own check
```

CI runs the checks on Linux on x86 and ARM, on Windows, and on macOS on ARM, and on Linux also checks that no vector instruction is left as a call. `RESULTS-1b.md` beside the code has the full results.

**What comes next** in code: a thread pool for the bands; then the Tier 1 readers, with the Fragment cache, Readers, read-ahead and the dictionary.

**What specification versions 68 and 69 change in this code.** Nothing. The pass already reads its rows top to bottom, which is the storage order of strips and of tiles stored row by row. Access patterns and domain profiles bear on the cache, the Readers, the input layer, the readers of GeoTIFF and NITF, and the dictionary, all still to be written. The implementability assessment, section 10, sets out where.

## 24. Gaps and inconsistencies found while writing

Writing this manual against specification versions 68 and 69 found the points below. Each is a proposal for the author, not a decision. Points fixed in the specification while writing are listed first, so the record is complete.

**Fixed while writing, or ruled on by the author**

| | Found | Done |
| --- | --- | --- |
| F1 | `set_cache_bytes` was named in section 4 and the held-data table, but missing from the cache's table of calls | Added to the table |
| F2 | Read-ahead was not said to work with external decoding, whose decoder owns the blocks | Read-ahead asks the decoder process for the next blocks, into slots no reader has marked |
| F3 | With the cache turned off, read-ahead would have had nowhere to go | Stated: with the cache off, nothing is read ahead |
| F4 | A profile report's items had no field names, and the profile calls belonged to no object | Items have `name`, `need`, `status`, `origin` and `basis`; the profile calls are BIL's own, beside `list_drivers`, and `check_profile` is on a Source or Image |
| F5 | The checking agent found that `ImageInfo.storage`, `describe_storage` and a scripted driver's `PixelLayout` described one thing three ways, and that a profile could not recommend another profile | One `Storage` record, which BIL derives from a `PixelLayout` for scripted drivers; profiles named in a profile's required or recommended list, with no separate `includes` |
| F6 | M1 of version 1: the values `RandomAccess` and `SequentialAccess` repeated their type's name | Renamed `Random` and `Sequential`, by the author's ruling (version 69) |
| F7 | M10 of version 1: whether blocks read ahead count against `set_cache_size`, and the other details of read-ahead | Accepted as proposed, by the author's ruling (version 69); with many small tiles, raise the count |

**Open, for the author**

| | Gap or inconsistency | Where | Proposal |
| --- | --- | --- | --- |
| M2 | **Whether an Image can have missing pixels** is something "a driver states" (section 5), but `ImageInfo` has no field for it, so a caller cannot see whether the fast path applies | 3, 5 | Add `may_have_missing` to `ImageInfo` |
| M3 | **How a Source is closed** is stated for C (`bil_close_source`) but not for Rust or Python. This manual says a Source closes when the last reference goes. | 2, 4 | State that, and add `close()` in Python for callers who want to free memory at a known point |
| M4 | **The configuration** names plugin locations, the decoder's place, `isolation` and the driver order, but its form and where it lives are not specified | 10.2, 10.3 | A file in a stated place, with an environment variable to point elsewhere and a call to read it, settled when Tier 2 starts |
| M5 | **Python forms not stated:** how a `SliceRequest` is built, how a returned set of Slices is indexed by Slice and Level, how a time is passed to `find_index`, and the fields of a world position | 3.1, 5, 13.4 | Constructors with keywords, indexing by `(slice_name, level_name)`, `datetime` for times, and positions with `lat`, `lon`, `height`, `status` and `accuracy`, as this manual assumes |
| M6 | **`level_count()`** is "how many resolution levels this Image offers", but computed Levels have no natural limit | 5 | The number of halvings until the Image is one pixel along its shorter side |
| M7 | **`common.units` and a Slice's unit.** A Slice's unit is part of its definition; `common.units` gives the unit of the calibrated value per component. They can disagree for a function that converts units. | 5, 14.2 | Define `common.units` as the unit of the calibration that `radiometric_gain` and `radiometric_offset` produce, and nothing else |
| M8 | **`common.sample_spacing` beside `common.ground_sample_distance`.** One is in the image plane, such as slant range, and the other on the ground, but the definitions do not yet say so | 14.2 | State it in both definitions |
| M9 | **Access patterns on a live stream.** A live stream has one position for every Reader, so neither pattern can change how it is read | 3.2, 4 | State that a live stream ignores the pattern, and `plan` reports it |
| M11 | **The four proposals of experiment 1b:** one 2D average rather than rescaling after each pass; stored NaN counted as missing; `valid_fraction` weighted by the filter; and `valid` following the averaged mask beside kinds that are not averaged | 5 | As the experiment does, set out in `RESULTS-1b.md` |
| M12 | **Labels at coarse Levels alone.** A label is kept, never averaged, so it needs only its grid point; but section 5's table of costs says a filtering method reads every pixel under the Window | 5 | Labels read only their grid points under any method, as the experiment does; state it in section 5 |

**How this list is kept.** Each new version of this manual rereads the chapters touched by the code or the specification since the last, adds what it finds here, and moves an item to the fixed table when the author rules on it.

## Appendix: where each call is described

| Call | Chapter |
| --- | --- |
| `open_source`, `open_source_with`, `describe_decoding` | 5 |
| `image_count`, `list_images`, `image`, `info` | 6 |
| `list_slice_functions`, `list_slice_names`, `default_slice` | 6, 7 |
| `read_slice`, `read_slices`, `read_slice_into`, `stream_slice`, `read`, `plan`, `plan_slice` | 7 |
| `list_level_methods`, `list_stored_levels` | 6, 8 |
| `read_slice_levels`, `level_count` | 8 |
| `read_fragment` | 10 |
| `count_frames`, `build_frame_index`, `write_frame_index`, `latest_frame`, `resolve_frame` | 11 |
| `list_coordinates`, `read_coordinate`, `find_index` | 11 |
| `open_reader`, `set_cache_size`, `set_cache_bytes`, `describe_cache`, `clear_cache`, `describe_held` | 12 |
| `list_required_fields` | 14, 16 |
| `list_field_groups`, `list_fields`, `read_field`, `read_field_at`, `describe_field`, `find_fields`, `add_field`, `release_metadata` | 16, 21 |
| `list_profiles`, `describe_profile`, `check_profile`, `add_profile`, `load_profiles` | 16 |
| `write_companion`, `add_field_definitions`, `load_field_definitions`, `list_markings` | 16, 20 |
| `list_geo_models`, `read_geo_model`, `for_slice`, `for_window`, `image_to_world`, `world_to_image`, `list_steps`, `concatenate`, `make_step` | 17 |
| `open_terrain`, `constant_terrain`, `caller_terrain`, `height_at`, `intersect`, `coverage` | 17 |
| `relative_accuracy`, `list_error_sources`, `circular_error`, `linear_error`, `error_ellipse` | 17 |
| `list_writers`, `create_source`, `write_fragment`, `write_slice`, `write_geo_model`, `write_metadata`, `close` | 18 |
| `list_drivers` | 19 |
| `register_plugin`, `register_driver`, `register_slice_function`, `register_level_method`, `bil_register_driver` | 20 |
| `list_auxiliary`, `read_auxiliary` | 20 |
| `bil_last_error` and the C conventions | 21, 22 |
