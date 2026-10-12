This document shows BIL in use, through calls a caller would actually write. It covers every part of specification version 67: opening, Windows, Levels, the slice functions for each kind of sensor, Fragments, frames, metadata, markings, geocoding, writing, performance, safety and extension.

**How to read it**

- **Python first.** Most examples are in Python, because it is the shortest. Section 17 shows the same calls in Rust and C.
- **Only calls the specification defines.** Every call, option and name comes from version 67. Version 1 of this document found 17 gaps in the calling interface. Version 58 of the specification closes all of them, and section 18 lists where.
- **Two assumptions.** `bil.Window(row_start, col_start, row_count, col_count, step=1, frame=0)` builds a Window, as the Rust struct in section 4 does. Options named in the specification are passed as keyword arguments.
- **Illustrative.** The specification marks its own code as "not final", and so are these examples.

## 1. Opening and looking

### 1.1 Open a file and read its default Slice

The simplest use: open a file, take its first Image, and read the Slice BIL chooses.

```python
import bil

source = bil.open_source("scene.tif")
image  = source.image(0)

name   = image.default_slice()                 # e.g. "pixel.raw.intensity"
window = bil.Window(0, 0, 1024, 1024)
tile   = image.read_slice(name, window)        # a 2D float32 Slice

array  = tile.data                             # a NumPy array over the same memory
print(tile.name, tile.level_name, tile.status) # "intensity", "level0", Status.Complete
```

### 1.2 See what an Image offers

```python
image.list_slice_functions()          # ["pixel.raw", "pixel.scaled", "color.rgb", "color.hsv", ...]
image.list_slice_names("color.hsv")   # ["hue", "saturation", "value"]
image.level_count()                   # how many resolution levels this Image offers
image.list_level_methods()            # ["bilinear", "binomial5", "decimate"]
image.list_stored_levels()            # stored1 at 1/2, stored2 at 1/4, ...
image.list_coordinates()              # e.g. wavelength on component, time on frame
```

### 1.3 A file with several Images

A NITF file with several image segments, or a satellite product with a panchromatic band beside lower-resolution multispectral bands, holds several Images.

```python
source = bil.open_source("product.ntf")
for index, info in enumerate(source.list_images()):
    print(index, info.representation, info.row_count, info.col_count, info.relations)
    print(source.image(index).list_slice_functions())

source.image_count()   # how many Images
```

### 1.4 Choose the driver yourself

```python
source = bil.open_source_with("gdal", "scene.tif")
source.describe_decoding()   # driver and codec chosen, with their versions
source.describe_held()       # every kind of held data, with counts and bytes
```

### 1.5 Open with options

```python
options = bil.OpenOptions(
    mapping="Private",                  # copy-on-write mapping (section 4)
    max_image_pixels=50_000_000_000,    # resource limits (section 10.3)
    max_decoded_bytes=2_000_000_000,
    max_held_positions=500,             # metadata held per index (section 14.3)
)
source = bil.open_source("scene.ntf", options)
```

## 2. Windows, tiles and large images

### 2.1 Plan before reading

`plan_slice` reports the size, the access class, any stored level used, and the blocks or frames to be decoded, before any pixel is read.

```python
window = bil.Window(40_000, 52_000, 4096, 4096)
plan   = image.plan_slice("pixel.raw.intensity", window)
print(plan)   # size 4096 × 4096, access class SinglePass, 16 blocks to decode
```

### 2.2 Work through an image too large for memory

Window independence (section 5) means processing tile by tile gives the same result as processing the whole image at once.

```python
info       = image.info()
rows, cols = info.row_count, info.col_count
tile_size  = 4096

for row in range(0, rows, tile_size):
    for col in range(0, cols, tile_size):
        window = bil.Window(row, col, min(tile_size, rows - row), min(tile_size, cols - col))
        tile   = image.read_slice("pixel.raw.intensity", window)
        process(tile.data)
```

### 2.3 Reuse one buffer for every tile

```python
import numpy as np

buffer = np.empty((4096, 4096), dtype=np.float32)
for window in tiles:
    image.read_slice_into("pixel.raw.intensity", window, buffer)   # allocates nothing
    process(buffer)
```

### 2.4 Stream row blocks through the processor cache

```python
total = 0.0
def add_block(block):          # called with float32 row blocks, in order
    global total
    total += float(block.data.sum())

image.stream_slice("pixel.raw.intensity", window, add_block)
```

### 2.5 Every second pixel, with no filtering

A `step` reads every nth pixel. It cannot be combined with a Level above 0.

```python
sparse = image.read_slice("pixel.raw.intensity", bil.Window(0, 0, 8192, 8192, step=2))
```

## 3. Resolution levels

### 3.1 A filtered overview

```python
overview = image.read_slice("pixel.raw.intensity", window, level=3)   # 1/8 in each direction
overview.origin, overview.scale    # where it sits on the Image: (0.0, 0.0), (8.0, 8.0)
```

### 3.2 A whole pyramid in one pass

```python
levels = image.read_slice_levels("pixel.raw.intensity", window, 4)
for slice in levels:
    print(slice.level_name, slice.data.shape)   # level0 ... level4
```

### 3.3 Less aliasing

```python
smooth = image.read_slice("pixel.raw.intensity", window, level=2, level_method="binomial5")
```

### 3.4 A quick look at a huge image

```python
quick = image.read_slice("pixel.raw.intensity", whole_image, level=8, level_method="decimate")
```

### 3.5 Decimate first, then filter

The first five halvings stride, and the last three filter. That is about a thousandth of the work of filtering all eight.

```python
thumb = image.read_slice("pixel.raw.intensity", whole_image, level=8, decimate_first=5)
```

### 3.6 Use the file's own overviews

```python
image.list_stored_levels()   # stored1 at 1/2, stored2 at 1/4 ...

as_stored = image.read_slice("color.rgb.green", window, level="stored2")
refined   = image.read_slice("color.rgb.green", window, level="stored2.level1")
from_file = image.read_slice("color.rgb.green", window, level=4, level_source="Stored")
from_file.level_name          # e.g. "stored2.level2"
```

## 4. Slices for each kind of sensor

### 4.1 Panchromatic and monochrome

```python
raw    = image.read_slice("pixel.raw.intensity", window)     # stored values as float32
scaled = image.read_slice("pixel.scaled.intensity", window)  # with the metadata gain and offset
```

### 4.2 RGB stills

```python
rgb = image.read_slices("color.rgb", ["red", "green", "blue"], window)
hsv = image.read_slices("color.hsv", ["hue", "saturation", "value"], window)   # from encoded values
lab = image.read_slices("color.lab", ["lightness", "green_red", "blue_yellow"], window)
lum = image.read_slice("color.luminance.luminance", window)

image.describe_field("common.transfer_characteristics")   # from the file, or the stated default
```

### 4.3 Palette images

```python
index = image.read_slice("palette.lookup.index", window)
color = image.read_slices("palette.lookup", ["red", "green", "blue"], window)
```

### 4.4 Multispectral bands and reflectance

```python
image.list_slice_names("spectral.bands")   # ["coastal", "blue", "green", "red", "near_infrared", ...]
nir = image.read_slice("spectral.bands.near_infrared", window)

image.list_required_fields("spectral.toa_reflectance")
# [common.sun_elevation: available, Derived; common.earth_sun_distance: available, Derived]
red = image.read_slice("spectral.toa_reflectance.red", window)
```

### 4.5 When a calibration is missing, supply it in a companion file

```python
image.list_required_fields("spectral.toa_reflectance")
# [common.sun_elevation: not available]

source.write_companion({"common.sun_elevation": 41.7})   # writes scene.tif.metadata
source = bil.open_source("scene.tif")                     # read back with origin Companion
"spectral.toa_reflectance" in source.image(0).list_slice_functions()   # True
```

### 4.6 Surface reflectance with your own atmospheric terms

```python
terms = bil.SliceSelector("spectral.surface_reflectance.red", gain=1.07, offset=-0.012)
surface_red = image.read_slice(terms, window)
```

The parameters are listed in section 9.7 of the specification: `gain` and `offset`, one for each band or a mapping by band name.

### 4.7 Hyperspectral cubes

```python
band_550 = image.read_slice(bil.SliceSelector("pixel.nearest.value", coordinate_name="wavelength", value=550.0), window)
green    = image.read_slice(bil.SliceSelector("spectral.integrate.value", from_nm=500, to_nm=600), window)
peak     = image.read_slices(bil.SliceSelector("pixel.peak", coordinate_name="wavelength"), ["value", "coordinate"], window)

curves   = {"pan": pan_curve, "red": red_curve}   # arrays of wavelength and weight
synth    = image.read_slices(bil.SliceSelector("spectral.response", curves=curves), ["pan", "red"], window)

image.read_coordinate("wavelength")   # the band centers, as 64-bit values
```

`coordinate_name` defaults to the component index's only coordinate, here wavelength, so `pixel.peak` could also be asked for by its plain name. A two-part `SliceSelector` names a function with its parameters, which is how `read_slices` takes one.

### 4.8 Ratios and differences of two components

```python
pair = bil.SliceSelector("pixel.pair.normalized_difference", i="near_infrared", j="red")
ndvi_like = image.read_slice(pair, window)
```

### 4.9 Thermal infrared

```python
counts, temperature = image.read_slices("thermal.calibrated", ["counts", "temperature"], window)
bt = image.read_slice("spectral.brightness_temperature.tirs1", window)
```

### 4.10 Complex SAR, from a SICD

```python
sicd = bil.open_source("collect.nitf").image(0)

magnitude = sicd.read_slice("complex.polar.magnitude", window)
phase     = sicd.read_slice("complex.polar.phase", window)
power     = sicd.read_slice("complex.power.power", window)   # avoids the square root
db        = sicd.read_slice("complex.power.power_db", window)
real_imag = sicd.read_slices("complex.cartesian", ["real", "imag"], window)
```

### 4.11 Calibrated backscatter

```python
sigma0    = sicd.read_slice("sar.calibrated.sigma0", window)
sigma0_db = sicd.read_slice("sar.calibrated.sigma0_db", window)
```

### 4.12 Quad-pol SAR

```python
quad = bil.open_source("quadpol.h5").image(0)

hv     = quad.read_slice(bil.SliceSelector("complex.polar.magnitude", component="hv"), window)
pauli  = quad.read_slices("pol.pauli", ["single_bounce", "double_bounce", "volume"], window)
span   = quad.read_slice("pol.span.span", window)
hh_vv  = quad.read_slices(bil.SliceSelector("pol.pair", a="hh", b="vv"), ["ratio", "phase_difference"], window)
```

### 4.13 An interferogram with its coherence

```python
ifg = bil.open_source("interferogram.h5").image(0)
phase, coherence = ifg.read_slices("insar.interferogram", ["phase", "coherence"], window)
```

The coherence is read from a second Image on the same grid. BIL reads both and passes them to the function together.

### 4.14 Raw camera files

```python
raw = bil.open_source("IMG_0042.CR3").image(0)

color   = raw.read_slices("cfa.demosaic", ["red", "green", "blue"], window)        # bilinear by default
sharper = raw.read_slices(bil.SliceSelector("cfa.demosaic", method="malvar_he_cutler"),
                          ["red", "green", "blue"], window)
sites   = raw.read_slices("cfa.sites", ["red", "green1", "green2", "blue"], window)
linear  = raw.read_slice("cfa.mosaic.linear", window)                              # black and white levels applied
one_site = raw.read_slice("cfa.mosaic.stored", bil.Window(0, 0, 2048, 2048, step=2))  # one site, unaltered
```

### 4.15 Polarization cameras

```python
pol    = bil.open_source("polarization.tif").image(0)
stokes = pol.read_slices("polcam.stokes", ["s0", "s1", "s2"], window)
dolp   = pol.read_slices("polcam.linear", ["degree", "angle"], window)
angles = pol.read_slices("polcam.angles", ["angle0", "angle45", "angle90", "angle135"], window)
```

### 4.16 Video

Video is two related Images: luma at full size, and chroma at reduced size. A Window carries one frame.

```python
video = bil.open_source("flight.ts")
luma  = video.image(0)

frame_window = bil.Window(0, 0, 1080, 1920, frame=1200)
y     = luma.read_slice("video.planes.luma", frame_window)
rgb   = luma.read_slices("video.rgb", ["red", "green", "blue"], frame_window)   # chroma upsampled
top, bottom = luma.read_slices("video.fields", ["top", "bottom"], frame_window)  # interlaced
```

### 4.17 Wide-area motion imagery

```python
wami   = bil.open_source("argus_frame_set").image(0)
target = bil.Window(52_300, 14_800, 640, 480, frame=37)   # one VGA-sized window from 1.8 gigapixels
chip   = wami.read_slice("pixel.raw.intensity", target)
```

### 4.18 Elevation and classification

```python
dem       = bil.open_source("dem.tif").image(0)
landcover = bil.open_source("landcover.tif").image(0)

height = dem.read_slice("elev.height.height", window)
labels = landcover.read_slice("class.labels.value", window)
water  = landcover.read_slice("class.members.water", window)
coarse = landcover.read_slice("class.labels.value", window, level=2)   # labels are kept, never averaged
```

### 4.19 Depth cameras and LIDAR

```python
depth    = bil.open_source("depth.png").image(0)        # range and intensity, two Images on one grid
waveform = bil.open_source("waveform.h5").image(0)

rng, intensity = depth.read_slices("depth.sensor", ["range", "intensity"], window)
x, y, z        = depth.read_slices("depth.points", ["x", "y", "z"], window)     # an organized point cloud
corrected      = depth.read_slice("depth.corrected.intensity", window)
bins           = waveform.list_slice_names("depth.waveform")                   # one per range bin
coarse_range   = depth.read_slice("depth.sensor.range", window, level=2)       # 3×3 minimum, nearest surface
```

### 4.20 Medical volumes and nuclear medicine

A volume's slices are frames.

```python
ct    = bil.open_source("series/").image(0)   # a DICOM series
axial = ct.read_slice("pixel.raw.intensity", bil.Window(0, 0, 512, 512, frame=120))

pet      = bil.open_source("pet_series/").image(0)
activity = pet.read_slice("nuclear.activity.activity", bil.Window(0, 0, 256, 256, frame=40))   # Bq/mL

# With bil-extras installed. BIL is not for diagnostic use.
suv = pet.read_slice("nuclear.uptake.suv", bil.Window(0, 0, 256, 256, frame=40))
```

### 4.21 Whole-slide pathology

```python
slide = bil.open_source("biopsy.svs").image(0)
slide.list_stored_levels()                     # the scanner's pyramid
low   = slide.read_slice("color.rgb.red", slide_window, level="stored3")
field = slide.read_slices("color.rgb", ["red", "green", "blue"], bil.Window(81_920, 40_960, 2048, 2048))
```

### 4.22 Security screening and dual energy

```python
scan = bil.open_source("bag_0193.dcs").image(0)
high, low = scan.read_slices("pixel.raw", ["component1", "component2"], window)
ratio = scan.read_slice(bil.SliceSelector("pixel.pair.ratio", i="component1", j="component2"), window)
```

### 4.23 Ultrasound, radar scans and sonar on a polar grid

```python
sweep = bil.open_source("radar_scan.h5").image(0)
echo  = sweep.read_slice("pixel.raw.intensity", window)   # rows are range, columns are beam
model = sweep.list_geo_models()[0]                        # a polar step locates each sample
```

### 4.24 Analysis functions from the extras package

```python
"spectral.indices" in image.list_slice_functions()   # True only with bil-extras installed
ndvi  = image.read_slice("spectral.indices.ndvi", window)
angle = image.read_slice(bil.SliceSelector("spectral.angle.value", reference=library_spectrum), window)
```

## 5. One request for everything

### 5.1 Several Slices in one pass

The Fragment is decoded once, and work the Slices share is done once.

```python
slices = image.read_slices("color.hsv", ["hue", "value"], window)
hue, value = slices["hue"], slices["value"]
```

### 5.2 Several functions, several Levels, streamed

```python
request = bil.SliceRequest(
    slices=["color.rgb.red", "color.rgb.green", "color.luminance.luminance"],
    window=window,
    levels=[0, 2, 4],
    options=bil.SliceOptions(level_method="binomial5"),
    delivery=bil.Delivery.Streamed,
)
print(image.plan(request))   # what it will deliver, and what it will cost
def consume(block):
    # One call for each block of rows, tagged with its Slice and Level.
    store(block.slice_name, block.level_name, block.rows, block.data)

image.read(request, consume)
```

Blocks of one Slice and Level arrive in row order. Blocks of different Slices and Levels may interleave.

### 5.3 Into buffers the caller owns

```python
request = bil.SliceRequest(slices=["color.rgb.red", "color.rgb.green"], window=window, levels=["level0"],
                           options=bil.SliceOptions(), delivery=bil.Delivery.IntoBuffers)
sizes   = image.plan(request).sizes             # the size of each buffer to provide
buffers = {("color.rgb.red", "level0"): red_buffer, ("color.rgb.green", "level0"): green_buffer}
image.read(request, buffers)
```

Buffers are keyed by Slice name and level name, and each must have the size `plan` reports.

## 6. Missing data

### 6.1 NaN by default, with the mask beside it

```python
values = image.read_slice("pixel.raw.intensity", window)
valid  = image.read_slice("pixel.mask.valid", window)
values.status                   # Status.Partial where pixels are missing
np.isnan(values.data).sum()     # the missing pixels
```

### 6.2 A number instead of NaN, for an FFT

```python
filled   = image.read_slice("pixel.raw.intensity", window, missing_value=0.0)
spectrum = np.fft.fft2(filled.data)
mask     = image.read_slice("pixel.mask.valid", window)   # to know which values were filled
```

### 6.3 Saturated pixels

```python
saturated = image.read_slice("pixel.mask.saturated", window)
```

### 6.4 Coarse Levels that count the pixels behind them

```python
# Equal, the default: each Level averages the present values of the Level below.
overview = image.read_slice("pixel.raw.intensity", window, level=4)

# PixelCount: as if each Level were made from full resolution at once, so a value next to a large
# gap is not pulled toward the few pixels that happen to survive at the Level below.
faithful = image.read_slice("pixel.raw.intensity", window, level=4,
                            missing_weights=bil.MissingWeights.PixelCount)
share    = image.read_slice("pixel.mask.valid_fraction", window, level=4)   # how much was present
```

## 7. Fragments: the native data

### 7.1 Read native pixels

```python
fragment = image.read_fragment(window)
fragment.pixel_type      # PixelType(U16, component_count=1, significant_bits=12, bit_shift=0)
fragment.is_borrowed     # True when the pixels are the stored bytes themselves
fragment.strides         # row, column and component strides in bytes
pixels = fragment.data() # a NumPy array; read-only when borrowed under Shared mapping
```

### 7.2 Exact 64-bit values

Slices are float32. For the full precision of `F64` or `U64` data, read the Fragment.

```python
exact = image.read_fragment(window).data()   # dtype float64, as stored
```

### 7.3 A deferred video Fragment

```python
fragment = luma.read_fragment(bil.Window(0, 0, 1080, 1920, frame=1200))
fragment.is_deferred   # True: only the compressed packets were extracted
pixels = fragment.data()   # decodes now, from the preceding key frame
```

### 7.4 Missing pixels in a Fragment

```python
fragment.validity        # a mask, or None when nothing is missing
fragment.status
```

### 7.5 A stored level as native pixels

```python
native_overview = image.read_fragment(window, level="stored2")
```

`level` is `level0` by default, or a stored level's name. A computed Level holds new values, so it is never a Fragment, and naming one is an error.

## 8. Frames, time and coordinates

### 8.1 Find the frame nearest a time

```python
count, certainty = video.image(0).count_frames()   # e.g. (54_000, Known)
frame = luma.find_index("time", bil.DateTime("2026-09-14T10:22:31.500Z"))
shot  = luma.read_slice("video.rgb.red", bil.Window(0, 0, 1080, 1920, frame=frame))
```

### 8.2 Build and keep a frame index

```python
luma.build_frame_index()                 # reads through the stream once
luma.write_frame_index("flight.ts.idx")  # a later open that names it does not scan again
video = bil.open_source("flight.ts", bil.OpenOptions(frame_index="flight.ts.idx"))
```


### 8.3 Nested collection cycles

```python
frame = lidar.resolve_frame(sweep=12, gate=31)
burst = lidar.read_slice("pixel.raw.intensity", bil.Window(0, 0, 256, 256, frame=frame))
gate_range = lidar.read_field_at("common.gate_range", frame)
```

### 8.4 A pushbroom line's time

```python
times = pushbroom.read_coordinate("time")   # one 64-bit time per row
```

### 8.5 A live stream

```python
live   = bil.open_source("udp://239.0.0.1:5000").image(0)
live.count_frames()                 # (frames received so far, AtLeast)
newest = live.latest_frame()

stop   = bil.Cancel()               # stop.trigger() from another thread ends the read
frame  = live.read_slice("video.planes.luma", bil.Window(0, 0, 720, 1280, frame=newest),
                         timeout=2.0, cancel=stop)
```

A read that waits longer than its `timeout`, or is cancelled, returns an error.

## 9. Metadata

### 9.1 Browse what a file holds

```python
image.list_field_groups()                  # ["nitf.image_subheader", "nitf.RPC00B", ...]; extracts nothing
image.list_fields("nitf.image_subheader")  # the field names of one group
title = source.read_field("nitf.file_header.FTITLE")
title.value, title.unit, title.origin      # origin Specified
```

### 9.2 The same item in every format

```python
source.read_field("common.collection_time")   # NITF IDATIM, KLV time, Exif, or frame 0 time
source.read_field("common.sensor_name")
image.read_field("common.ground_sample_distance")   # provided, or derived from the geo model
image.describe_field("common.sun_elevation")        # its origin, and the formula and inputs behind it
```

### 9.3 Names with dots and repeated names

```python
source.read_field("hdf5.calibration.band\\.1")   # a dot inside one part is escaped
source.read_field(["hdf5", "calibration", "band.1"])   # or pass the list of parts
source.read_field("nitf.RPC00B[2]")              # the second instance of a repeated extension
source.read_field("fits.HISTORY[3]")
```

### 9.4 Search without extracting everything

```python
source.find_fields("*sun*", extract=False)   # defined names and groups already extracted
source.find_fields("*sun*", extract=True)    # extracts every group first
```

### 9.5 Per-frame metadata from KLV

```python
platform_heading = luma.read_field_at("klv.0601.platform_heading", 1200)
platform_heading.value, platform_heading.time   # the latest packet at or before frame 1200, with its own time
```

### 9.6 Times with and without a zone

```python
t = source.read_field("common.collection_time")
t.value.zone_known            # False when an Exif time had no offset and no lookup was installed
source.describe_field("common.collection_time")   # which of the four ways gave the zone
```

### 9.7 Add your own fields and definitions

```python
source.add_field("caller.task_id", "T-0417")
source.add_field("common.sun_elevation", 42.1)   # overrides the file, with origin Caller

source.add_field_definitions(vendor_tre_definitions)   # names, types and units for a new NITF extension
source.read_field("nitf.VNDRTRE.GAIN")
```

### 9.8 Free held metadata

```python
source.release_metadata("nitf.RPC00B")   # extracted again if asked for
```

### 9.9 Additional records that are not fields

```python
source.list_auxiliary()                    # e.g. ["calibration_table", "line_times"]
table = source.read_auxiliary("calibration_table")
```

## 10. Markings and identity

### 10.1 See the markings

```python
source.list_markings()   # security classification and caveats, in native form
tile.list_markings()     # a Slice keeps a reference to its Source's markings
```

### 10.2 Writing keeps markings and empties identity

```python
out = bil.create_source("chip.tif", "GeoTIFF", info)   # markings written, identity empty
out.write_slice(window, tile)
out.close()
```

### 10.3 Keep identity, where policy allows

```python
out = bil.create_source("study.tif", "GeoTIFF", info, bil.WriteOptions(keep_identity=True))
```

## 11. Geocoding

### 11.1 List and choose a model

```python
image.list_geo_models()                     # most rigorous first: ["rational_polynomial", "projective"]
model = image.read_geo_model("rational_polynomial", 0)
model.label                                 # the format's own name for it, such as "RPC00B"
for step in model.list_steps():
    print(step)                             # each step with its spaces and parameters
```

A model is named for the step that does its main work. Two models that would share a name are numbered.

### 11.2 Where is this pixel?

```python
chain = model.for_window(window, 0)
world = chain.image_to_world([(512.0, 512.0)], bil.constant_terrain(0.0))
world[0].position, world[0].status   # latitude, longitude and height as 64-bit, Status.Complete
```

### 11.3 With terrain

```python
terrain = bil.open_terrain("dted/")            # a folder of tiles
terrain.coverage()
world = chain.image_to_world(pixel_positions, terrain)
```

### 11.4 Where does this place appear?

```python
pixels = chain.world_to_image([(34.0522, -118.2437, 85.0)])
if pixels[0].status == bil.Status.Complete:
    row, col = pixels[0].position
```

### 11.5 A pixel of a reduced Slice

```python
overview = image.read_slice("pixel.raw.intensity", window, level=3)
chain    = model.for_slice(overview)          # built from the Slice's origin and scale
corner   = chain.image_to_world([(0.0, 0.0)], terrain)
```

### 11.6 Project the result

```python
utm   = chain.concatenate(bil.make_step("reference_change", target="EPSG:32611"))
east_north = utm.image_to_world(pixel_positions, terrain)
```

### 11.7 A model for each video frame

```python
for frame in range(0, frame_count, 30):
    chain = luma.read_geo_model(luma.list_geo_models()[0], frame).for_window(full_frame, 0)
    footprint = chain.image_to_world(corners, terrain)
```

### 11.8 SAR, with honest failure

```python
chain = sicd.read_geo_model(sicd.list_geo_models()[0], 0).for_window(window, 0)
for result in chain.image_to_world(points, terrain):
    if result.status in (bil.Status.NotConverged, bil.Status.Ambiguous):
        continue   # no position is returned
```

### 11.9 Your own terrain

```python
def lidar_surface(latitude, longitude):
    return my_dsm.height(latitude, longitude)

surface = bil.caller_terrain(lidar_surface)
heights = surface.height_at(positions)
hits    = surface.intersect(lines_of_sight)
```

### 11.10 A medical image in patient space

```python
chain = ct.read_geo_model(ct.list_geo_models()[0], 120).for_window(axial_window, 0)
patient_mm = chain.image_to_world([(256.0, 256.0)])   # no surface: the chain ends in patient space
```

### 11.11 How accurate is this position?

```python
results = chain.image_to_world(points, terrain, accuracy=True)
a = results[0].accuracy
a.status                                  # Status.Partial if a step stated no error
a.covariance                              # 3 x 3, east, north and up, in meters
for c in a.contributions:
    print(c.name, c.kind, c.provenance)   # rational_polynomial.error, Bias, Stated; surface.height, Random, Stated; ...
bil.circular_error(a, 0.9), bil.linear_error(a, 0.9)   # CE90 and LE90, computed exactly
bil.error_ellipse(a, 0.9)                 # semi-axes and orientation
```

A step with no stated error is named among the contributions as unknown, and the status is `Partial`. It is never taken as perfect.

### 11.12 How sure is the distance between two points?

```python
difference_covariance = chain.relative_accuracy(points_a, points_b)   # shared bias cancels
chain.list_error_sources()                            # every contribution the chain carries
```

### 11.13 Check the first-order result on steep geometry

```python
grazing = chain.image_to_world(edge_points, terrain, accuracy=True, accuracy_method="sampled")
```

### 11.14 Carry a registration correction's own error

```python
correction = bil.make_step("affine", **fitted_affine, covariance=fit_covariance)   # the affine's own parameters
registered = chain.concatenate(correction)   # its error is carried with the rest
```

The registration itself is a layer above BIL. BIL carries the correction and its error forward, and `write_geo_model` writes the error estimates with the model.

## 12. Writing

### 12.1 A float GeoTIFF from Slices

```python
info = bil.ImageInfo(4096, 4096, bil.PixelType(bil.SampleType.F32, 2, 32), ["sigma0", "coherence"])
out  = bil.create_source("product.tif", "GeoTIFF", info)
out.write_slice(window, sigma0, "sigma0")        # into the component named sigma0
out.write_slice(window, coherence, "coherence")
out.write_metadata([source.read_field("common.collection_time")])
out.write_geo_model(sicd.read_geo_model(sicd.list_geo_models()[0], 0))
out.close()
```

A Slice goes into the component of its own name unless another is named. A writer writes the Source's geo model without being asked, as far as the format holds it, with the rest in the companion file. `write_geo_model` writes a different one instead.

### 12.2 Native pixels to ENVI

```python
out = bil.create_source("cube.dat", "ENVI", cube.info)
for window in tiles:
    out.write_fragment(window, cube.read_fragment(window))
out.close()
```

### 12.3 A NumPy file

```python
out = bil.create_source("tile.npy", "NumPy", info)
out.write_slice(window, tile)
out.close()   # markings go to tile.npy.metadata
```

### 12.4 Check the round trip

```python
again = bil.open_source("product.tif").image(0).read_slice("pixel.raw.sigma0", window)
assert np.array_equal(again.data, sigma0.data)
```

### 12.5 What can be written

```python
bil.list_writers()   # ["ENVI", "GeoTIFF", "TIFF", "NumPy"]
```

## 13. Performance

### 13.1 Size the cache to the access pattern

```python
source.set_cache_size(64)    # keep 64 decoded Fragments, for overlapping Windows
source.describe_cache()      # Fragments held, bytes held, reads served
source.clear_cache()
source.set_cache_size(0)     # no cache, for a single pass over each block
```

### 13.2 Read from several threads

A Source and an Image may be shared between threads. Each thread gets its own Reader.

```python
from concurrent.futures import ThreadPoolExecutor

def work(window):
    return image.read_slice("pixel.raw.intensity", window).data.mean()

with ThreadPoolExecutor(8) as pool:
    means = list(pool.map(work, tiles))   # the interpreter lock is released during each read
```

### 13.3 Two places in one video

```python
ahead = luma.open_reader()   # a second Reader, holding its own place in the stream
here  = luma.read_slice("video.planes.luma", bil.Window(0, 0, 1080, 1920, frame=1200))
later = ahead.read_slice("video.planes.luma", bil.Window(0, 0, 1080, 1920, frame=9000))
```

A Reader offers every read call its Image offers.

### 13.4 Know which reads are borrowed

```python
plan = image.plan_slice("pixel.raw.intensity", window)
plan.access_class   # Borrowed, Converted or Decoded; delivery is reported separately
```

## 14. Safety and audit

### 14.1 A file from outside

```python
options = bil.OpenOptions(untrusted=True, max_image_pixels=2_000_000_000,
                          max_decoded_bytes=500_000_000, max_field_group_bytes=10_000_000)
source = bil.open_source("inbox/received.ntf", options)
```

With `isolation` set to `Untrusted` in the installation's configuration, its C and C++ drivers run in a separate process.


### 14.2 Audit what is installed

```python
for entry in bil.list_drivers():
    print(entry.name, entry.version, entry.source, entry.license)   # license as built
```

### 14.3 Record how a file was read

```python
source.describe_decoding()   # driver and codec, with versions; also written as bil.driver and bil.codec
```

## 15. Extending BIL

### 15.1 A scripted driver for a vendor format

```python
class VendorDriver(bil.ScriptedDriver):
    name = "vendor"

    def probe(self, location):
        return bil.Confidence.High if location.read_bytes(0, 4) == b"VNDR" else bil.Confidence.NoMatch

    def list_images(self, source):
        self.header = parse_header(source.read_bytes(0, 512))
        return [bil.ImageInfo(self.header.row_count, self.header.col_count,
                              bil.PixelType(bil.SampleType.U16, 1, 12), ["intensity"])]

    def describe_pixels(self, source, image_index):
        return bil.PixelLayout(offset=512, byte_order="big", interleave="pixel")

bil.register_driver(VendorDriver)
```

The same driver serves Rust, C and C++ callers. For them, BIL runs it in a helper process with the Python installed on the machine, so a failing script cannot take the caller down. Because `describe_pixels` tells the compiled core where the pixels are, they never pass through Python. A script that returns pixels as an array instead would pay a round trip for every read. Without an installed Python, only scripted drivers are unavailable.

### 15.2 A tiled, compressed vendor format

```python
def describe_pixels(self, source, image_index):
    return bil.PixelLayout(offset=self.header.data_offset, byte_order="little", interleave="band",
                           blocks=self.header.tile_table, codec="deflate",
                           validity=self.header.missing_regions)
```

### 15.3 A slice function under a driver's own family

```python
def vendor_gain_kernel(inputs, parameters, outputs):
    # inputs:  one float32 row block for each component, with any margin the radius needs
    # outputs: one float32 row block for each Slice, to be filled
    outputs["intensity"][:] = inputs[0] * parameters["gain"]

bil.register_slice_function(
    name="vendor.gain_corrected",
    representations=["intensity"],
    slice_names=["intensity"],
    kinds={"intensity": "linear"},
    radius=0,
    parameters=[("gain", float, 1.0)],
    inputs=[],
    kernel=vendor_gain_kernel,
)

bil.register_level_method(name="box2", weights=[1, 1], radius=1, alignment="between")
```

BIL calls a registered kernel once for each block of rows, and does the reading, conversion, Levels, delivery and threads itself. The core's own functions run in the fused loop; a registered one runs as a call for each block, which is slower but keeps every other guarantee.

### 15.4 A new NITF extension, without rebuilding

```python
source.load_field_definitions("new_tre.json")
```

```json
[
  {
    "name": ["nitf", "VNDRTRE", "GAIN"],
    "locator": {"extension": "VNDRTRE", "offset": 12, "length": 6},
    "value_type": "float",
    "encoding": "ascii_decimal",
    "unit": "dB",
    "handling": "Ordinary"
  }
]
```

### 15.5 A sensor model from a plugin

```python
csm_chain = image.read_geo_model("csm", 0)   # an external step handed to a Community Sensor Model plugin
```

## 16. Errors and status

### 16.1 A call either delivers or fails

```python
try:
    tile = image.read_slice("pixel.raw.intensity", window)
except bil.Error as error:
    log(error)   # a file that cannot be opened, a bad argument, a failed read, a limit exceeded, corrupt data
```

### 16.2 Corrupt blocks as missing data

```python
source = bil.open_source("damaged.ntf", bil.OpenOptions(corrupt_as_missing=True))
tile   = source.image(0).read_slice("pixel.raw.intensity", window)
tile.status   # Partial
```


### 16.3 A step that cannot answer

```python
for result in chain.world_to_image(points):
    if result.status != bil.Status.Complete:
        report(result.status)   # Missing, NotConverged or Ambiguous
```

## 17. The same calls in Rust and C

### 17.1 Rust: tiles, a pyramid and geocoding

```rust
use bil::{open_source, Window, SliceRequest, SliceOptions, Delivery};

let source = open_source("scene.ntf")?;
let image  = source.image(0)?;

let window = Window { row_start: 0, col_start: 0, row_count: 4096, col_count: 4096, step: 1, frame: 0 };
let tile   = image.read_slice("pixel.raw.intensity", window)?;
println!("{} {:?}", tile.level_name, tile.status);

let request = SliceRequest {
    slices:   vec!["color.rgb.red".into(), "color.luminance.luminance".into()],
    window,
    levels:   vec![0, 2],
    options:  SliceOptions::default(),
    delivery: Delivery::Returned,
};
let slices = image.read(&request)?;

let model  = image.read_geo_model(&image.list_geo_models()[0], 0)?;
let chain  = model.for_window(window, 0);
let world  = chain.image_to_world(&[(512.0, 512.0)], &bil::constant_terrain(0.0))?;
```

### 17.2 Rust: borrowed data with no copy

```rust
let fragment = image.read_fragment(window)?;
if fragment.is_borrowed() {
    let bytes: &[u8] = fragment.data()?;   // the stored bytes themselves
}
```

### 17.3 C: open, read and check every status

```c
BilSource *source;
BilImage  *image;
BilSlice  *slice;
BilWindow  window = { 0, 0, 1024, 1024, 1, 0 };

BilStatus status = bil_open_source("scene.ntf", &source);
if (status == BIL_OK) status = bil_image(source, 0, &image);
if (status == BIL_OK) status = bil_read_slice(image, "complex.polar.magnitude", &window, &slice);
if (status == BIL_OK) {
    const float *values;
    size_t rows, cols, row_stride;
    status = bil_slice_data(slice, &values, &rows, &cols, &row_stride);
    /* ... values are valid until the Slice is freed ... */
    bil_free_slice(slice);
}
if (status != BIL_OK) {
    char message[512];
    bil_last_error(message, sizeof message);
}
bil_close_source(source);
```

Every C function returns a status, every object is an opaque handle freed by its matching call, and the message for the last failed call on a thread is read with `bil_last_error`.

### 17.4 C: into a buffer the caller owns

```c
float *buffer = malloc(4096 * 4096 * sizeof(float));
status = bil_read_slice_into(image, "pixel.raw.intensity", &window, buffer);
```

## 18. Gaps closed in version 58

Version 1 of this document found these gaps in the calling interface. Version 58 of the specification closes each one.

| Gap | Example | Closed by | Section of the specification |
| --- | --- | --- | --- |
| Listing Images | 1.3 | `image_count`, `list_images`, `image` and `ImageInfo` | 3 |
| Image size | 2.2 | `info()` on an Image | 3 |
| The caller's `read_fragment` | 7.5 | `read_fragment(window, level)`, by Level name | 4 |
| Streamed and buffered requests | 5.2, 5.3 | `read(request, target)`: a callback of tagged blocks, or buffers keyed by Slice and Level | 5 |
| Function parameters | 4.6, 4.7 | A table of every parameter, and two-part selectors for `read_slices` | 5, 9.7 |
| `OpenOptions` | 1.5, 8.2, 14.1 | Every option listed with its default | 3.4 (version 57) |
| Corrupt data as missing | 16.2 | `corrupt_as_missing` in `OpenOptions` | 3.4 (version 57) |
| Geo model names | 11.1 | Named for the step that does the main work, with the format's name as label | 13.3 |
| Making steps | 11.6 | `make_step(step_name, parameters)` | 13.4 |
| Surfaces that are not needed | 11.10 | `surface` may be left out | 13.4 |
| Readers | 13.3 | A Reader offers every read call of its Image | 4 |
| Live streams | 8.5 | `latest_frame`, and `timeout` and `cancel` in `SliceOptions` | 3.2, 3.4 |
| Registering functions and methods | 15.3 | The descriptions a slice function and a level method give, and how a registered kernel runs | 10 |
| Field definitions | 15.4 | The parts of a definition, kept as JSON, and `load_field_definitions` | 14.1, 14.4 |
| Dictionary items | 8.3 | `common.gate_range` and `common.projection_angle` | 14.2 |
| The C interface | 17.3 | Five conventions: handles, status, ownership, data and errors | 2 |
| Writing several components | 12.1 | `write_slice(window, slice, component)` and `write_geo_model` | 11 |

**Settled since, in version 59.** A writer writes the geo model without being asked (R30). A live stream has one position for all Readers, and a frame no longer held is `Missing` (R34).

**Added since, in versions 62 to 67.** Every position can carry its accuracy, with `relative_accuracy`, `circular_error`, `linear_error` and `error_ellipse` (examples 11.11 to 11.14). Scripted drivers run in a helper process for Rust, C and C++ callers (example 15.1). `missing_weights` chooses how coarse Levels weight missing data (example 6.4). External decoding through the mapped format needs no change to the caller's code, so it has no example of its own.
