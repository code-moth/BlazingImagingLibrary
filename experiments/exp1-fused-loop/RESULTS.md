Experiment 1 of the implementability report (section 8) tests the riskiest part of BIL's core: one fused pass that makes a slice function's Slices at every Level, with Window independence exact to the bit, on scalar, AVX2 and AVX-512 code. It was run in this workspace on 12 October 2026, against specification version 56.

## Result

**Passed on correctness, partly on speed.**

- **Bitwise, everywhere.** Over 2,400 random Windows, about 580 million values were compared. No value differed in a single bit between a Window and the whole Image, at any Level from 0 to 4. None differed between scalar, AVX2 and AVX-512 code either. That is stronger than R27 of the second review asked for, which allowed a tolerance between instruction sets.
- **Accurate.** BIL's own arctangent and sine, built from basic operations only, are within 2.6 × 10⁻⁷ radians of a float64 reference.
- **The arithmetic runs at memory speed.** On data already in memory, `color.hsv` runs 8.7 times faster with AVX-512 than scalar, and `complex.polar` 2.5 times, where it is limited by memory rather than by arithmetic.
- **The whole pass does not yet.** End to end, `pixel.raw` at Level 0 matches a copy into fresh memory, but making Levels 1 to 4 and the other functions costs 5 to 90 times a copy into warm memory. The cause is the prototype's structure, not the arithmetic, and the fix is the rolling buffer section 12 already specifies.

## What was built

A Rust crate of about 1,050 lines, with no dependencies. It builds in 13 seconds from clean and 5 seconds after a change, and the test program is 0.6 MB.

| Part | What it does |
| --- | --- |
| Vector layer | One interface with three back ends: plain Rust, AVX2 (8 lanes) and AVX-512 (16 lanes), chosen at run time |
| Kernels | `pixel.raw`, `complex.polar` and `color.hsv`, written once and compiled for each back end |
| Levels | Weights 1 2 1 at each halving, rows then columns; Level n on the pixels at multiples of 2ⁿ; mirroring about each Level's first and last grid point; angles reduced as unit vectors |
| Sample types | `U16` and `F32` for `pixel.raw`; `ComplexI16` and `ComplexF32` for `complex.polar`; `U8` and `U16` color for `color.hsv` |
| Test program | Random Windows against whole Images, every instruction set against scalar, accuracy against float64, and speed |

## How bitwise agreement was achieved

Four rules, all cheap to keep.

1. **Only exactly rounded operations.** Add, subtract, multiply, divide and square root are correctly rounded by IEEE 754 in every instruction set, so a lane gives the same bits whatever runs it. Minimum, maximum, comparison and selection are exact.
2. **No fused multiply-add.** It rounds once instead of twice, so it gives different bits. Rust never fuses operations on its own, so leaving it out is enough.
3. **BIL's own transcendental functions.** Arctangent and sine are polynomials built from the operations in rule 1, after the Cephes library, so they run identically everywhere. Library versions differ between scalar and vector code.
4. **Row ends through the same vector code.** The last few values of a row are copied into a padded vector and run through the same code as the rest, so a value's bits never depend on where it falls in a row.

**Window independence follows from the grid.** A value at Level k is computed from the same Level k − 1 values whatever Window asks for it, because the Level grid and its mirroring are fixed to the Image, not to the Window.

## Results

### Bitwise agreement

Image of 1,037 × 1,291 pixels, chosen so that neither side is a multiple of 16. 400 random Windows for each case, including Windows at every edge and Windows smaller than a Level's spacing.

| Function | Sample type | Values compared, for each instruction set | Differing bits, Window against whole | Differing bits, against scalar |
| --- | --- | --- | --- | --- |
| `pixel.raw` | U16 | 15,691,144 | 0 | 0 |
| `pixel.raw` | F32 | 15,455,875 | 0 | 0 |
| `complex.polar` | ComplexI16 | 31,081,260 | 0 | 0 |
| `complex.polar` | ComplexF32 | 30,685,234 | 0 | 0 |
| `color.hsv` | U8 color | 49,986,690 | 0 | 0 |
| `color.hsv` | U16 color | 51,056,940 | 0 | 0 |

Each row was run on scalar, AVX2 and AVX-512, with the same result.

### Accuracy at Level 0

| Slice | Largest error against float64 |
| --- | --- |
| `complex.polar.phase` | 2.6 × 10⁻⁷ radians |
| `complex.polar.magnitude` | 1.1 × 10⁻⁷, relative |
| `color.hsv.hue` | 6.4 × 10⁻⁸ turns |

### The arithmetic alone

16 million values already in memory, one thread.

| Kernel | Scalar | AVX2 | AVX-512 | Best speed-up |
| --- | --- | --- | --- | --- |
| One 1 2 1 halving | 1.38 ns a value | 1.26 | 1.28 | 1.1, limited by memory |
| `complex.polar` at Level 0 | 4.93 | 2.22 | 2.01 | 2.5, limited by memory |
| `color.hsv` at Level 0 | 21.65 | 3.74 | 2.49 | 8.7 |

### The whole pass

One thread, best of three, against two baselines: a copy into memory already in use, and a copy into freshly allocated memory, which on this machine costs about 2 microseconds a page the first time it is touched.

| Function | Image | Levels | Best time | Copy into warm memory | Copy into fresh memory |
| --- | --- | --- | --- | --- | --- |
| `pixel.raw`, U16 | 8,192 × 8,192 | 0 | 0.14 s, scalar | 0.026 s | 0.20 s |
| `pixel.raw`, U16 | 8,192 × 8,192 | 0 to 4 | 0.39 s, scalar | 0.026 s | 0.20 s |
| `complex.polar`, ComplexI16 | 4,096 × 4,096 | 0 | 0.16 s, AVX-512 | 0.006 s | 0.05 s |
| `complex.polar`, ComplexI16 | 4,096 × 4,096 | 0 to 4 | 0.40 s, AVX-512 | 0.006 s | 0.05 s |
| `color.hsv`, U8 color | 4,096 × 4,096 | 0 | 0.24 s, AVX-512 | 0.006 s | 0.06 s |
| `color.hsv`, U8 color | 4,096 × 4,096 | 0 to 4 | 0.60 s, AVX-512 | 0.006 s | 0.06 s |

## Why the whole pass is slow, and the fix

**The prototype makes whole planes.** It makes every intermediate value for the whole Window at Level 0 as a full-size plane: four planes for `complex.polar`, five for `color.hsv`. Each is new memory, and new memory costs more than the arithmetic. Each Level is then made from the whole plane of the one before.

**Section 12 already says what to do instead.** "Demosaicing and every bilinear resolution level are produced in the same pass, on a rolling buffer of a few rows that stays in the processor cache." Two changes follow.

- **A rolling buffer.** Keep only the few rows each Level needs, and write each finished row of each Level straight to its output.
- **The caller's own buffers.** With `read_slice_into` and streaming, output goes to memory the caller already holds, so the cost of fresh memory disappears from repeated reads.

Neither change alters any arithmetic, so the bitwise tests carry over unchanged as the check that the faster version is still right.

**Two smaller costs.** The halving step gathers its inputs one value at a time, which limits it to scalar speed. A vector shuffle can replace the gather, again without changing the arithmetic. Conversion from stored samples to float32 is scalar too, and can be vectorized in the same way.

## What it means for the specification

- **R27 of the second review can be resolved more strongly than proposed.** Bitwise agreement across instruction sets costs little, so the specification can require it, not just a tolerance. The four rules above would go into section 12.
- **Rust's lack of stable portable vectors is not a blocker.** A small vector layer of about 150 lines, over the processor's own instructions, was enough. Generic code compiled for each back end gives one kernel per function, written once.
- **One trap was found and fixed.** At first, the vector paths were slower than scalar because the compiler called each vector instruction as a separate function. The vector methods must carry their instruction set, and the small functions inside each kernel must be inlined. This would have been easy to miss without measuring.

## What it did not cover

Missing data and NaN skipping, threads, stored levels, the other kinds of Slice (labels and range), local functions such as demosaicing, `binomial5`, and ARM. Later work covered several of these: missing data, the other kinds of Slice and `binomial5` in experiment 1b, and a NEON back end for ARM, checked in CI. Each fits the same structure. ARM's NEON needs CI on an ARM runner, since this workspace is x86 only.

## Next

1. **Experiment 1b: the rolling buffer.** The same arithmetic, rows streamed through every Level, with output into the caller's buffers. Passes when the bitwise tests still pass and the whole pass is within a small factor of a copy into warm memory.
2. **Add missing data**, with its own bitwise tests, since NaN skipping changes the weights.
3. **Move the code into the repository** once it exists, with CI on Linux, macOS, Windows and ARM.
