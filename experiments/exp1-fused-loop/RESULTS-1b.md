Experiment 1b replaces experiment 1's whole planes with the rolling buffer of specification section 12: "every bilinear resolution level [is] produced in the same pass, on a rolling buffer of a few rows that stays in the processor cache." It then adds missing data, the other kinds of Slice, `decimate` and `binomial5`, for section 5. It was run in this workspace on 12 and 13 October 2026, against specification versions 67 to 69.

## Result

**Passed.** The author ruled that the speed criterion is there to shape the design: as fast as possible, allowing for the arithmetic each slice function needs. Section 12 now says so.

- **The same bits.** Experiment 1's bitwise tests, unchanged, pass through the rolling buffer: about 580 million values over 2,400 random Windows, at Levels 0 to 4, on scalar, AVX2 and AVX-512 code. No value differs from the Image read whole, from scalar code, or from experiment 1's whole planes.
- **The edge cases too.** Every Window of 25 small Images, from 1 × 1 to 9 × 9 pixels, with up to 6 Levels asked for, gives the same bits as experiment 1: 308,700 reads.
- **Missing data, for section 5.** A missing pixel is NaN in every Slice. Each halving leaves NaN out and rescales the weights, and a value is NaN only when nothing that would contribute is present. `missing_value` is applied last. `missing_weights` chooses how a Level weights the values below it: `Equal`, the default, or `PixelCount`, by the share of present full-resolution pixels behind each, as the author ruled in version 67. Both agree bitwise with an independent reference, between Windows and the whole Image, and between instruction sets. The rule itself agrees with a float64 computation to 5.6 × 10⁻⁸, and `PixelCount` equals the pyramid of the masked data over the pyramid of the mask to 2.7 × 10⁻⁷. Three details the specification leaves open are proposals, set out below.
- **The other kinds of Slice, and `decimate`.** Labels keep every second value, range takes the least present value of each 3 × 3 block, and `pixel.mask` gives `valid` and `valid_fraction`. `decimate` keeps the value at each grid point for every kind. Each agrees with its definition, between Windows and the whole Image, and across instruction sets, with and without missing data.
- **`binomial5`, the second level method.** Weights 1 4 6 4 1 at each halving, with a radius of 2, so each Level keeps a ring of five rows and the margins are wider. It agrees bitwise with a whole-plane reference, with and without missing data and under both weightings, between Windows and the whole Image, across instruction sets, and on every Window of 36 small Images. Its arithmetic agrees with float64 to 1.9 × 10⁻⁷, and a wave at the highest frequency the halved grid holds comes through at a quarter of its amplitude, against half for `bilinear`, as section 5 states. It costs about what `bilinear` costs: 0.9 to 1.3 times over two runs, probably because the halvings are limited by memory rather than arithmetic.
- **No cost where nothing is missing.** An Image without a mask makes no checks. A mask with nothing missing gives exactly the values of no mask, including values near the top of the float32 range.
- **Nothing allocated per read.** A `Pass` is planned once, and running it again writes into the same Slice buffers and working rows.
- **3 to 5 times faster than experiment 1,** on the fastest instruction set. `pixel.raw` takes 1.7 times a copy into warm memory at Level 0, and 2.7 times for Levels 0 to 4. `complex.polar` and `color.hsv` take 1.4 to 1.7 times their own arithmetic at Level 0, and 2.2 to 2.8 times for Levels 0 to 4.

## How it works

`src/rolling.rs`, about 740 lines: `Pass::new` plans a read, `Pass::outputs` allocates its Slices, and `Pass::run` fills them. `plan_outputs` and `read_levels_into` are short forms for a single read.

1. **One row at a time.** Each row of Level 0 is converted from the native samples and run through the Level 0 kernel. For `pixel.raw`, it is converted straight into the Level 0 row.
2. **A ring of three rows for each Level above Level 0.** The row is halved along the row into the next Level's ring. A row of Level k + 1 needs three rows of Level k, and is made as soon as the last of them arrives. Its oldest source is then at most two rows back, so three are enough. Each read of the ring checks that the row it finds is the one it wants.
3. **Written out at once.** Each row of each Level is written into its Slices as soon as it is made, then passed on. No full-size intermediate plane exists.
4. **The same arithmetic.** Without missing data, the kernels are experiment 1's, called in the same order on the same values, which is why the bits agree. Splitting a kernel's work into rows changes nothing, because the last values of every call go through the same vector code on a padded copy.
5. **Halving read from the row itself.** A new kernel, `halve_row`, separates even and odd columns with vector shuffles, which only move values, and halves them in the same arithmetic as `halve`. Only the two ends of a row, where the Image edge mirrors the sources, are gathered by index.

### Missing data

An Image with a validity mask takes a second path, chosen once when the `Pass` is planned.

- **One weight row for every plane.** Every plane of a pixel is present or missing together, so each Level keeps one row of weights beside its planes. At Level 0 a weight is 1 where the pixel is present and 0 where it is missing, and a select makes every plane of a missing pixel NaN, whatever its kernel made of the stored samples.
- **Along the row.** `halve` and `halve_row` halve the weights once. For each plane, `halve_weighted` and `halve_row_weighted` make the sum of the present values times their weights, with weights 1 2 1 and scaled by a quarter as `halve` scales, but not divided.
- **Down the column.** The weights are halved down the column, and `halve_over` combines three rows of each plane's sums the same way and divides once by them. The result is the two-dimensional average of the values present, with the weights rescaled, and NaN where the weights sum to zero.
- **The next Level's weights.** With `Equal`, each weight becomes 1 where it is above 0 and stays 0 elsewhere, so every present value below counts alike. With `PixelCount`, the halved weight is kept: the share of present full-resolution pixels under the whole filter. By induction, a `PixelCount` Level is then the unmasked pyramid of the data with missing pixels set to 0, divided by the unmasked pyramid of the mask.
- **Angles.** Phase and hue are made from the averaged unit vector, then set to NaN wherever the vector is missing.
- **Filled last.** `missing_value`, when given, replaces NaN in the finished Slice.
- **Exact when nothing is missing.** With all three sources present, the weight is exactly 1 at each step and the sum has the bits of `halve`, so the division changes nothing. Scaling at each step, rather than once at the end, also keeps the sums within range: an earlier version scaled only at the end, and the checking agent found that values above about a sixteenth of the float32 maximum then overflowed. The cost is at the other end of the range: a subnormal value with few neighbors present can lose up to about 3 bits, within the tolerance of section 5.

### The other kinds of Slice, and `decimate`

Section 5 gives each kind of Slice its own reduction. Each `Func` declares one, and the level method `decimate` replaces it with keeping for every kind.

- **Keep, for labels and `decimate`.** The next Level's row i is this Level's row 2i at the even columns, copied as it is, with no ring. Every plane is kept, the Level 0 extras included, so a kept phase or hue is Level 0's own value, not one rebuilt from a unit vector.
- **Minimum, for range.** `min3_present` and `min3_row` take the least present value of three, along the row into the ring and down the column, so each Level holds the least present value of a 3 × 3 block of the Level before. NaN only when all nine are missing.
- **The mask Slices.** `pixel.mask` averages the mask itself, 1.0 where present and 0.0 where missing, with no missing values of its own. `valid_fraction` is that average, and `valid` is 1.0 wherever it is above 0, which is wherever any contributing pixel was valid.

### `binomial5`

- **The same structure, with five taps.** A halving has 3 taps for `bilinear` and 5 for `binomial5`; the gathered columns, the edge buffers and the ring each have one entry for each tap, and a row of Level k + 1 is made once the last of its five source rows arrives. Kinds that are not averaged keep their own reductions, labels keeping and range the 3 × 3 minimum, so `binomial5` changes only the averaged Slices.
- **Planned for its method.** The radius sets the margins, so a `Pass` is planned for its level method with `Pass::for_method`, as `plan` takes the whole request. Running it with a method of another radius is refused.
- **The arithmetic.** `((a + e) / 16 + (b + d) / 4) + 3c / 8`, in that order. Each scale is a power of two or 3/8, so weights of exactly 1 give exactly 1 and the weighted path keeps the bits of the unweighted one, as for `bilinear`; a sum overflows only where `bilinear`'s would.
- **Kernels.** `blur5`, `blur5_row`, `blur5_weighted`, `blur5_row_weighted` and `blur5_over` in the `bil` crate, beside their `bilinear` counterparts. Reading five taps from the row itself takes three even-and-odd loads; `row_taps` serves three taps and five alike.

### NEON

The `bil` crate has a NEON back end for 64-bit ARM since 13 October 2026: four lanes, with minimum and maximum made from a compare and a select so that NaN and signed zeros behave as in scalar code, and the deinterleaving load for even and odd values. This workspace has no ARM target, so CI is the test: on Linux and macOS on ARM, every check of this report and of experiment 1 passes with NEON, bitwise against scalar code on the same machine, and on Linux no vector instruction is left as a call. Its speed has not been measured, since CI machines are shared and section 12 runs benchmarks elsewhere.

### Proposals awaiting the author

Section 5 left four details open, and the code raised a fifth. The author settled one in version 67, and for the other four the code makes these choices.

| Detail | Choice made | The other reading |
| --- | --- | --- |
| Rescaling a 3 × 3 halving | Once, over the 3 × 3 weights of the values present: a true two-dimensional average | Rescaled after the row pass and again after the column pass, which weights rows with fewer present values more |
| NaN stored in float data | On the masked path, any NaN counts as missing, so it is skipped and filled like a masked pixel | Only the mask decides, and a stored NaN at a valid pixel spreads as it does without a mask |
| `pixel.mask.valid_fraction` | The share of contributing pixels that were valid, each counted with its filter weight: the `PixelCount` weight | Each contributing pixel counted once, whatever its weight |
| `pixel.mask.valid` beside a kind not averaged | Follows the averaged mask, whatever kind of Slice it is read beside, so beside labels at a coarse Level it can be 1.0 where the label is NaN | Follows the reduction of the Slice it goes with: the grid-point pixel for labels and `decimate`, any of nine for range |

The settled detail, how coarser Levels weight the values below, is now the `missing_weights` option.

## Results

### Bitwise agreement, no mask

The same Image of 1,037 × 1,291 pixels and the same 400 random Windows for each case as experiment 1.

| Function | Sample type | Values compared, for each instruction set | Window against whole | Against scalar | Against experiment 1 |
| --- | --- | --- | --- | --- | --- |
| `pixel.raw` | U16 | 15,691,144 | 0 | 0 | 0 |
| `pixel.raw` | F32 | 15,455,875 | 0 | 0 | 0 |
| `complex.polar` | ComplexI16 | 31,081,260 | 0 | 0 | 0 |
| `complex.polar` | ComplexF32 | 30,685,234 | 0 | 0 | 0 |
| `color.hsv` | U8 color | 49,986,690 | 0 | 0 | 0 |
| `color.hsv` | U16 color | 51,056,940 | 0 | 0 | 0 |

Each row was run on scalar, AVX2 and AVX-512, with the same result. The counts match experiment 1's exactly.

Every Window of the 25 small Images, Levels 0 to `top` for every `top` from 0 to 6, all three functions on all three instruction sets: 308,700 reads, with no value differing from experiment 1.

### Missing data

The same Images with a validity mask: about one pixel in ten missing at random, a missing block, and a missing first row and last column. The reference makes whole planes, indexed directly, one Level after another, in scalar code, and shares only the arithmetic. 200 random Windows for each case.

| Function | Sample type | Values compared, for each instruction set and weighting | Missing at Level 0 | Missing at Level 4 | Whole against reference | Windows against reference | All present against no mask |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `pixel.raw` | U16 | 8,962,474 | 160,375 | 66 | 0 | 0 | 0 |
| `pixel.raw` | F32 | 7,366,199 | 160,946 | 66 | 0 | 0 | 0 |
| `complex.polar` | ComplexI16 | 16,525,364 | 161,689 | 66 | 0 | 0 | 0 |
| `complex.polar` | ComplexF32 | 17,452,860 | 161,641 | 66 | 0 | 0 | 0 |
| `color.hsv` | U8 color | 23,882,715 | 161,434 | 66 | 0 | 0 | 0 |
| `color.hsv` | U16 color | 26,792,565 | 161,194 | 66 | 0 | 0 | 0 |

Each row was run with `Equal` and with `PixelCount`, on scalar, AVX2 and AVX-512, with the same result. At Level 4 only the inside of the missing block is still missing. With `missing_value` set, every NaN became the number given and no other value changed, on every instruction set.

- **Small Images.** The 25 small Images, with masks missing two pixels in five or every pixel, both weightings: 617,400 reads, no value differing from the reference.
- **NaN stored in the data.** NaN stored at about one valid pixel in fifty, in either component of F32 and ComplexF32 data, gives exactly the result of masking those pixels instead, for `pixel.raw`, `complex.polar` and `pixel.mask`, with both weightings, on every instruction set. The checking agent found that an earlier version averaged a stored NaN as 0 at coarser Levels.
- **`PixelCount` by its definition.** Levels 0 to 4 against the unmasked pyramid of the data with missing pixels set to 0, divided by the unmasked pyramid of the mask: the largest relative difference is 2.7 × 10⁻⁷, and NaN falls exactly where the mask's pyramid is 0. `PixelCount` differs from `Equal` at 107,701 values, all near missing pixels.
- **Near the float32 limit.** F32 values up to about 2.5 × 10³⁷, with a mask that has nothing missing, give exactly the values of no mask on every instruction set.
- **The rule itself.** Level 1 of `pixel.raw` was computed in float64 as the 1 2 1 by 1 2 1 average of the pixels present, with the weights rescaled. The largest relative difference is 5.6 × 10⁻⁸, and the result is NaN exactly where no pixel is present.

### The other kinds of Slice, and `decimate`

The same Image size, 200 random Windows for each case, on scalar, AVX2 and AVX-512. Every row had no value differing from its definition, from the whole Image, or from scalar code.

| Slice | Image | Level method | Values compared, for each instruction set | Checked against |
| --- | --- | --- | --- | --- |
| `class.labels.value` | U16, and U16 with holes | `bilinear` | 8,006,187 | Level 0's value at each grid point |
| `pixel.raw`, `complex.polar`, `color.hsv` | U16, ComplexI16, U8 color, and U16 with holes | `decimate` | 7,283,721 to 21,851,163 | Level 0's value at each grid point |
| `depth.sensor.range` | F32 with holes | `decimate` | 7,283,721 | Level 0's value at each grid point |
| `depth.sensor.range` | F32, and F32 with holes | `bilinear` | 8,091,856 | A whole-plane reference, at every Level |
| `pixel.mask.valid`, `valid_fraction` | U16 with holes | `bilinear` | 17,371,198 | The mask averaged as an Image with no mask |
| `pixel.mask.valid`, `valid_fraction` | U16, no mask | `bilinear` | 16,489,598 | 1.0 everywhere |

Every Window of the 25 small Images with masks, Levels 0 to 0 up to 0 to 6, for labels, range, the mask Slices, and `decimate` on `pixel.raw` and `complex.polar`: 514,500 reads, no value differing from the whole Image or from the definition.

### `binomial5`

The six cases of the bitwise table, each with and without the mask of the missing-data tests and under both weightings, 200 random Windows each, on scalar, AVX2 and AVX-512: 7,884,067 to 24,068,058 values compared for each row, and no value differing from the reference, from the whole Image or from scalar code. Every Window of 36 small Images, of 1, 2, 3, 4, 5 and 9 rows by the same columns, at Levels 0 to 0 up to 0 to 6, with and without masks: 537,600 reads, no value differing. Labels and range read with `binomial5` are identical to `bilinear`, and `pixel.mask` gives the mask averaged by `binomial5`.

| Check | Result |
| --- | --- |
| Levels 1 and 2 against the 1 4 6 4 1 filter in float64 | Largest relative difference 1.9 × 10⁻⁷ |
| A wave with a period of four pixels, through Level 1 | 0.5 of its amplitude through `bilinear`, 0.25 through `binomial5` |

At Levels 0 to 4, on the vector instruction sets, `binomial5` took 0.94 to 1.13 times as long as `bilinear` in one run and 0.90 to 1.26 in another: in the first, 1.04 for `pixel.raw` on 8,192 × 8,192, 0.98 to 1.13 for `complex.polar` and 0.94 to 0.95 for `color.hsv` on 4,096 × 4,096. The difference is mostly within the variation of this shared two-core machine. The extra arithmetic of each halving is probably hidden behind the memory it reads.

### Reused buffers

The buffers of a first read were filled with NaN and read into again. Every value was rewritten, with the same bits as the first read, on every instruction set.

### Speed of the whole pass

One thread, best of five runs, the whole Image as the Window, through one `Pass` run repeatedly. "Copy" is a copy of one float32 plane of the Image's size into memory already in use. "Arithmetic" is the Level 0 kernel alone over the whole Image, on buffers already in memory; for `pixel.raw`, which has no kernel, it is the copy. Experiment 1's whole planes were timed again in the same run, on the same instruction set. The fastest instruction set is shown for each row, and `cargo run --release --bin exp1b` prints them all.

| Function | Image | Levels | Slices | Rolling buffer | Times a copy | Times the arithmetic | Experiment 1 | Times a copy |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `pixel.raw`, U16 | 8,192 × 8,192 | 0 | 1 | 0.046 s, AVX2 | 1.7 | 1.7 | 0.146 s | 5.2 |
| `pixel.raw`, U16 | 8,192 × 8,192 | 0 to 4 | 1 | 0.076 s, AVX2 | 2.7 | 2.7 | 0.393 s | 14.1 |
| `complex.polar`, ComplexI16 | 4,096 × 4,096 | 0 | 2 | 0.046 s, AVX-512 | 6.8 | 1.4 | 0.163 s | 24.2 |
| `complex.polar`, ComplexI16 | 4,096 × 4,096 | 0 to 4 | 2 | 0.078 s, AVX2 | 11.5 | 2.2 | 0.373 s | 55.3 |
| `color.hsv`, U8 color | 4,096 × 4,096 | 0 | 3 | 0.074 s, AVX-512 | 11.6 | 1.7 | 0.224 s | 35.0 |
| `color.hsv`, U8 color | 4,096 × 4,096 | 0 to 4 | 3 | 0.118 s, AVX-512 | 18.5 | 2.8 | 0.544 s | 85.1 |

The copy took 0.028 s for 8,192 × 8,192, and 0.006 to 0.007 s for 4,096 × 4,096. Timings on this shared two-core machine vary by about 10 percent between runs. `pixel.raw` is limited by memory, so scalar code is as fast as vector code. In the previous version of this file, before the halving was read from the row itself, Levels 0 to 4 of `pixel.raw` took 0.102 s on AVX2. Across runs since, they take 0.068 to 0.078 s.

**With missing data**, on vector code, a read with a mask takes 1.4 to 1.8 times the same read without one for `pixel.raw`, and 1.2 to 1.4 times for `complex.polar` and `color.hsv`. The extra cost is one pass at Level 0 that reads every plane, sets each pixel's weight and makes a missing pixel NaN, and halving a row of weights beside the planes; for `pixel.raw`, with one plane, that doubles the halving. Only Images that can have missing pixels pay it.

**The other kinds**, at 8,192 × 8,192 on vector code, against the same copy:

| Slice | Level method | Level 0 | Levels 0 to 4 |
| --- | --- | --- | --- |
| `class.labels.value` | `bilinear` | 1.8 times a copy | 3.2 |
| `depth.sensor.range` | `bilinear` | 2.4 | 3.8 |
| `pixel.mask`, two Slices, with holes | `bilinear` | 3.1 | 5.2 |
| `pixel.raw.intensity` | `decimate` | 1.9 | 3.2 |

The fastest instruction set is shown; the copy took 0.026 s in that run. `pixel.mask` converts the samples too when they are float, to find stored NaN; integer samples cannot hold one, so for them it reads only the mask.

**One more inlining trap.** At first, range on AVX-512 took 0.30 s for Levels 0 to 4, against 0.09 s on AVX2. The binary showed calls to `select` and to array `map`: the minimum kernel's closures over arrays had not been inlined into the AVX-512 code. Written out without them, it takes 0.089 s. The same was done for the weighted sums. This is rule 5 of the working notes again, and `objdump` found it.

**Reading the table.** `complex.polar` and `color.hsv` are limited by their arithmetic: an arctangent, a square root and divisions for every pixel. Levels 1 to 4 add about three quarters as much again as Level 0. For these functions that is mostly the arctangent of the reduced phase or hue at every Level, and the halving of three or four carried planes.

## What it leaves

- **Conversion from native samples** is a plain loop that the compiler vectorizes as it can. For interleaved complex and color samples, a vector shuffle could separate the components directly.
- **The mask at Level 0** is converted to weights in a plain loop, then applied by a select. For `pixel.raw` it could be applied during conversion instead.
- **`decimate` reads every pixel here.** A request in this experiment always includes Level 0, which needs every pixel, so `decimate` costs what `bilinear` does. Section 5 has `decimate` stride across the storage and touch only the pixels kept; that needs a request for coarser Levels alone.
- **Keeping copies by index.** The kept row is copied value by value; the even-column load of `halve_row` could do it in vector code.
- **Threads.** Each thread would take a band of rows with its own rings, and the rows a band shares with its neighbor would be made twice.
- **Not covered,** as in experiment 1: stored levels, local functions such as demosaicing, and `pixel.mask.saturated`.

## Running it

```sh
cargo run --release --bin exp1b             # the full report, including speed
cargo run --release --bin exp1b -- --check  # correctness only, as CI runs it
```
