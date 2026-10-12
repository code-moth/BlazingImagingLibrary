Experiment 1b replaces experiment 1's whole planes with the rolling buffer of specification section 12: "every bilinear resolution level [is] produced in the same pass, on a rolling buffer of a few rows that stays in the processor cache." It was run in this workspace on 12 October 2026, against specification version 65.

## Result

**Passed on correctness. On speed, passed for `pixel.raw`, and for the other two functions only against a different yardstick, which needs the author's ruling.**

- **The same bits.** Experiment 1's bitwise tests, unchanged, pass through the rolling buffer: about 580 million values over 2,400 random Windows, at Levels 0 to 4, on scalar, AVX2 and AVX-512 code. No value differs from the Image read whole, from scalar code, or from experiment 1's whole planes.
- **The edge cases too.** Every Window of 25 small Images, from 1 × 1 to 9 × 9 pixels, with up to 6 Levels asked for, gives the same bits as experiment 1: 308,700 reads.
- **Into the caller's buffers.** A read writes every value of Slice buffers the caller allocated once, and depends on nothing left in them. Its own working rows, a few rows a Level, are still allocated on each call.
- **About 3 to 4 times faster than experiment 1:** 2.8 to 4.2 times, depending on the function and the Levels.
- **Against the stated criterion,** a small factor of a copy into warm memory: `pixel.raw` takes 1.9 times a copy at Level 0 and 3.6 times for Levels 0 to 4. `complex.polar` takes 6 to 11 times and `color.hsv` 10 to 20 times, because their arithmetic alone costs several copies.
- **Against their own arithmetic,** the Level 0 kernel alone on data already in memory, those two take 1.7 to 1.8 times at Level 0 and 2.3 to 2.4 times for Levels 0 to 4, on the fastest instruction set. This is the proposed yardstick for functions whose arithmetic exceeds a copy, and it is not yet agreed.

## How it works

`read_levels_into` in `src/rolling.rs`, about 440 lines, with `plan_outputs` to allocate the Slice buffers once.

1. **One row at a time.** Each row of Level 0 is converted from the native samples and run through the Level 0 kernel.
2. **A ring of three rows for each Level above Level 0.** The row is halved along the row into the next Level's ring. A row of Level k + 1 needs three rows of Level k, and is made as soon as the last of them arrives. Its oldest source is then at most two rows back, so three are enough. Each read of the ring checks that the row it finds is the one it wants.
3. **Written out at once.** Each row of each Level is written into its Slices as soon as it is made, then passed on. No full-size intermediate plane exists.
4. **The same arithmetic.** The kernels are experiment 1's, called in the same order on the same values, which is why the bits agree. Splitting a kernel's work into rows changes nothing, because the last values of every call go through the same vector code on a padded copy.
5. **Mirrored only at the ends.** Halving along a row reads the interior's sources in order, as even and odd columns, and gathers by index only at the two ends of the row, where the Image edge mirrors them. Before this step, Levels 0 to 4 took 0.188 s for `pixel.raw`, 0.155 s for `complex.polar` and 0.235 s for `color.hsv` on AVX2. After it, they took 0.102 s, 0.083 s and 0.129 s: about 1.8 times faster.

## Results

### Bitwise agreement

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

### Small Images

Images of 1, 2, 3, 5 and 9 rows by 1, 2, 3, 5 and 9 columns, every possible Window, Levels 0 to `top` for every `top` from 0 to 6, all three functions on all three instruction sets: 308,700 reads, and no value differing from experiment 1 in a single bit. These cover single rows and columns, Levels of a single point, and more Levels asked for than the Image has.

### Reused buffers

The buffers of a first read were filled with NaN and read into again. Every value was rewritten, with the same bits as the first read, on every instruction set.

### Speed of the whole pass

One thread, best of five runs, the whole Image as the Window, into Slice buffers allocated once. "Copy" is a copy of one float32 plane of the Image's size into memory already in use. "Arithmetic" is the Level 0 kernel alone over the whole Image, on buffers already in memory; for `pixel.raw`, which has no kernel, it is the copy. Experiment 1's whole planes were timed again in the same run, on the same instruction set. The fastest instruction set is shown for each row, and `cargo run --release --bin exp1b` prints them all.

| Function | Image | Levels | Slices | Rolling buffer | Times a copy | Times the arithmetic | Experiment 1 | Times a copy |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `pixel.raw`, U16 | 8,192 × 8,192 | 0 | 1 | 0.054 s, AVX2 | 1.9 | 1.9 | 0.150 s | 5.2 |
| `pixel.raw`, U16 | 8,192 × 8,192 | 0 to 4 | 1 | 0.102 s, AVX2 | 3.6 | 3.6 | 0.415 s | 14.5 |
| `complex.polar`, ComplexI16 | 4,096 × 4,096 | 0 | 2 | 0.048 s, AVX-512 | 6.3 | 1.7 | 0.160 s | 21.2 |
| `complex.polar`, ComplexI16 | 4,096 × 4,096 | 0 to 4 | 2 | 0.083 s, AVX2 | 10.9 | 2.3 | 0.345 s | 45.6 |
| `color.hsv`, U8 color | 4,096 × 4,096 | 0 | 3 | 0.067 s, AVX-512 | 10.3 | 1.8 | 0.221 s | 34.0 |
| `color.hsv`, U8 color | 4,096 × 4,096 | 0 to 4 | 3 | 0.129 s, AVX2 | 19.8 | 2.4 | 0.510 s | 78.6 |

The copy took 0.029 s for 8,192 × 8,192, and 0.006 to 0.008 s for 4,096 × 4,096 in the two cases. Timings on this shared two-core machine vary by about 10 percent between runs, and experiment 1's times here differ by as much from those in `RESULTS.md`.

**Reading the table.** `complex.polar` and `color.hsv` are limited by their arithmetic: an arctangent, a square root and divisions for every pixel. Levels 1 to 4 then add about as much again as Level 0, though they hold only a third as many values. That is the cost of the two scalar copies below.

## What it leaves

- **Two scalar copies.** Conversion from native samples, and splitting a row into even and odd columns, are plain loops. Both can use vector shuffles without changing the arithmetic, and they are most of what Levels 1 to 4 add.
- **Working rows allocated per call.** A reusable pass, planned once, would make a repeated read allocate nothing, as section 12 asks of `read_slice_into`.
- **Missing data and NaN skipping,** with their own bitwise tests. This is the next step.
- **Threads.** Each thread would take a band of rows with its own rings, and the rows a band shares with its neighbor would be made twice.
- **Not covered,** as in experiment 1: stored levels, other kinds of Slice, local functions such as demosaicing, and `binomial5`.

## Running it

```sh
cargo run --release --bin exp1b             # the full report, including speed
cargo run --release --bin exp1b -- --check  # correctness only, as CI runs it
```
