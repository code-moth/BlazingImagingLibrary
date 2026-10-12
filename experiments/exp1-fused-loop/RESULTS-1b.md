Experiment 1b replaces experiment 1's whole planes with the rolling buffer of specification section 12: "every bilinear resolution level [is] produced in the same pass, on a rolling buffer of a few rows that stays in the processor cache." It then adds missing data, for section 5. It was run in this workspace on 12 October 2026, against specification version 66.

## Result

**Passed.** The author ruled that the speed criterion is there to shape the design: as fast as possible, allowing for the arithmetic each slice function needs. Section 12 now says so.

- **The same bits.** Experiment 1's bitwise tests, unchanged, pass through the rolling buffer: about 580 million values over 2,400 random Windows, at Levels 0 to 4, on scalar, AVX2 and AVX-512 code. No value differs from the Image read whole, from scalar code, or from experiment 1's whole planes.
- **The edge cases too.** Every Window of 25 small Images, from 1 × 1 to 9 × 9 pixels, with up to 6 Levels asked for, gives the same bits as experiment 1: 308,700 reads.
- **Missing data, for section 5.** A missing pixel is NaN in every Slice. Each halving leaves NaN out and rescales the weights, and a value is NaN only when nothing that would contribute is present. `missing_value` is applied last. All of it agrees bitwise with an independent reference, between Windows and the whole Image, and between instruction sets. The rule itself agrees with a float64 computation to 5.6 × 10⁻⁸. Three details the specification leaves open are proposals, set out below.
- **No cost where nothing is missing.** An Image without a mask makes no checks. A mask with nothing missing gives exactly the values of no mask, including values near the top of the float32 range.
- **Nothing allocated per read.** A `Pass` is planned once, and running it again writes into the same Slice buffers and working rows.
- **3 to 5 times faster than experiment 1,** on the fastest instruction set. `pixel.raw` takes 1.6 times a copy into warm memory at Level 0, and 2.8 times for Levels 0 to 4. `complex.polar` and `color.hsv` take 1.3 to 1.6 times their own arithmetic at Level 0, and 2.3 to 2.9 times for Levels 0 to 4.

## How it works

`src/rolling.rs`, about 550 lines: `Pass::new` plans a read, `Pass::outputs` allocates its Slices, and `Pass::run` fills them. `plan_outputs` and `read_levels_into` are short forms for a single read.

1. **One row at a time.** Each row of Level 0 is converted from the native samples and run through the Level 0 kernel. For `pixel.raw`, it is converted straight into the Level 0 row.
2. **A ring of three rows for each Level above Level 0.** The row is halved along the row into the next Level's ring. A row of Level k + 1 needs three rows of Level k, and is made as soon as the last of them arrives. Its oldest source is then at most two rows back, so three are enough. Each read of the ring checks that the row it finds is the one it wants.
3. **Written out at once.** Each row of each Level is written into its Slices as soon as it is made, then passed on. No full-size intermediate plane exists.
4. **The same arithmetic.** Without missing data, the kernels are experiment 1's, called in the same order on the same values, which is why the bits agree. Splitting a kernel's work into rows changes nothing, because the last values of every call go through the same vector code on a padded copy.
5. **Halving read from the row itself.** A new kernel, `halve_row`, separates even and odd columns with vector shuffles, which only move values, and halves them in the same arithmetic as `halve`. Only the two ends of a row, where the Image edge mirrors the sources, are gathered by index.

### Missing data

An Image with a validity mask takes a second path, chosen once when the `Pass` is planned.

- **Level 0.** The mask row becomes 1.0 where present and NaN where missing. A select then makes every plane of a missing pixel NaN, whatever its kernel made of the stored samples.
- **Along the row.** `halve_sum` and `halve_row_sum` keep two numbers for each output: the weighted sum of the values present, and the sum of their weights, with weights 1 2 1 and scaled by a quarter as `halve` scales, but not divided.
- **Down the column.** `halve_skip` combines three rows of both with weights 1 2 1, scales them by a quarter, and divides once. The result is the two-dimensional average of the values present, with the weights rescaled, and NaN where the weights sum to zero.
- **Angles.** Phase and hue are made from the averaged unit vector, then set to NaN wherever the vector is missing.
- **Filled last.** `missing_value`, when given, replaces NaN in the finished Slice.
- **Exact when nothing is missing.** With all three sources present, the weight is exactly 1 at each step and the sum has the bits of `halve`, so the division changes nothing. Scaling at each step, rather than once at the end, also keeps the sums within range: an earlier version scaled only at the end, and the checking agent found that values above about a sixteenth of the float32 maximum then overflowed. The cost is at the other end of the range: a subnormal value with few neighbors present can lose up to about 3 bits, within the tolerance of section 5.

### Proposals awaiting the author

Section 5 says averaging "leave[s] NaN out and rescale[s] the remaining weights", and that a result is NaN only when every contributing value is missing. Three details are left open, and the code makes these choices.

| Detail | Choice made | The other reading |
| --- | --- | --- |
| Rescaling a 3 × 3 halving | Once, over the 3 × 3 weights of the values present: a true two-dimensional average | Rescaled after the row pass and again after the column pass, which weights rows with fewer present values more |
| Coarser Levels | Each Level averages the values of the Level before it with equal weight | Each value also carries how many full-resolution pixels lie behind it, and later Levels weight by that |
| NaN stored in float data | On the masked path, any NaN counts as missing, so it is skipped and filled like a masked pixel | Only the mask decides, and a stored NaN at a valid pixel spreads as it does without a mask |

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

| Function | Sample type | Values compared, for each instruction set | Missing at Level 0 | Missing at Level 4 | Whole against reference | Windows against reference | All present against no mask |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `pixel.raw` | U16 | 8,962,474 | 160,375 | 66 | 0 | 0 | 0 |
| `pixel.raw` | F32 | 7,366,199 | 160,946 | 66 | 0 | 0 | 0 |
| `complex.polar` | ComplexI16 | 16,525,364 | 161,689 | 66 | 0 | 0 | 0 |
| `complex.polar` | ComplexF32 | 17,452,860 | 161,641 | 66 | 0 | 0 | 0 |
| `color.hsv` | U8 color | 23,882,715 | 161,434 | 66 | 0 | 0 | 0 |
| `color.hsv` | U16 color | 26,792,565 | 161,194 | 66 | 0 | 0 | 0 |

Each row was run on scalar, AVX2 and AVX-512, with the same result. At Level 4 only the inside of the missing block is still missing. With `missing_value` set, every NaN became the number given and no other value changed, on every instruction set.

- **Small Images.** The 25 small Images, with masks missing two pixels in five or every pixel: 308,700 reads, no value differing from the reference.
- **Near the float32 limit.** F32 values up to about 2.5 × 10³⁷, with a mask that has nothing missing, give exactly the values of no mask on every instruction set.
- **The rule itself.** Level 1 of `pixel.raw` was computed in float64 as the 1 2 1 by 1 2 1 average of the pixels present, with the weights rescaled. The largest relative difference is 5.6 × 10⁻⁸, and the result is NaN exactly where no pixel is present.

### Reused buffers

The buffers of a first read were filled with NaN and read into again. Every value was rewritten, with the same bits as the first read, on every instruction set.

### Speed of the whole pass

One thread, best of five runs, the whole Image as the Window, through one `Pass` run repeatedly. "Copy" is a copy of one float32 plane of the Image's size into memory already in use. "Arithmetic" is the Level 0 kernel alone over the whole Image, on buffers already in memory; for `pixel.raw`, which has no kernel, it is the copy. Experiment 1's whole planes were timed again in the same run, on the same instruction set. The fastest instruction set is shown for each row, and `cargo run --release --bin exp1b` prints them all.

| Function | Image | Levels | Slices | Rolling buffer | Times a copy | Times the arithmetic | Experiment 1 | Times a copy |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `pixel.raw`, U16 | 8,192 × 8,192 | 0 | 1 | 0.043 s, scalar | 1.6 | 1.6 | 0.149 s | 5.5 |
| `pixel.raw`, U16 | 8,192 × 8,192 | 0 to 4 | 1 | 0.076 s, scalar | 2.8 | 2.8 | 0.398 s | 14.6 |
| `complex.polar`, ComplexI16 | 4,096 × 4,096 | 0 | 2 | 0.043 s, AVX-512 | 7.4 | 1.3 | 0.148 s | 25.6 |
| `complex.polar`, ComplexI16 | 4,096 × 4,096 | 0 to 4 | 2 | 0.076 s, AVX-512 | 13.2 | 2.3 | 0.400 s | 69.4 |
| `color.hsv`, U8 color | 4,096 × 4,096 | 0 | 3 | 0.066 s, AVX-512 | 11.0 | 1.6 | 0.218 s | 36.6 |
| `color.hsv`, U8 color | 4,096 × 4,096 | 0 to 4 | 3 | 0.116 s, AVX-512 | 19.6 | 2.9 | 0.514 s | 86.4 |

The copy took 0.027 s for 8,192 × 8,192, and 0.006 s for 4,096 × 4,096. Timings on this shared two-core machine vary by about 10 percent between runs. `pixel.raw` is limited by memory, so scalar code is as fast as vector code. In the previous version of this file, before the halving was read from the row itself, Levels 0 to 4 of `pixel.raw` took 0.102 s on AVX2. Across runs since, they take 0.068 to 0.078 s.

**With missing data**, on vector code, a read with a mask takes 1.4 to 1.5 times the same read without one for `pixel.raw`, and 1.1 to 1.3 times for `complex.polar` and `color.hsv`. The extra cost is checking every value and keeping weights beside the sums. Only Images that can have missing pixels pay it.

**Reading the table.** `complex.polar` and `color.hsv` are limited by their arithmetic: an arctangent, a square root and divisions for every pixel. Levels 1 to 4 add about three quarters as much again as Level 0. For these functions that is mostly the arctangent of the reduced phase or hue at every Level, and the halving of three or four carried planes.

## What it leaves

- **Conversion from native samples** is a plain loop that the compiler vectorizes as it can. For interleaved complex and color samples, a vector shuffle could separate the components directly.
- **The mask at Level 0** is converted to 1.0 and NaN in a plain loop, then applied by a select. For `pixel.raw` it could be applied during conversion instead.
- **Threads.** Each thread would take a band of rows with its own rings, and the rows a band shares with its neighbor would be made twice.
- **The mask Slice** `pixel.mask.valid`, and the other kinds of Slice: labels, which keep every second value, and range, which takes a minimum.
- **Not covered,** as in experiment 1: stored levels, local functions such as demosaicing, and `binomial5`.

## Running it

```sh
cargo run --release --bin exp1b             # the full report, including speed
cargo run --release --bin exp1b -- --check  # correctness only, as CI runs it
```
