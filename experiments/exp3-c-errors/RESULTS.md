Experiment 3 of the implementability assessment, section 8: containing C libraries' errors (H2). It asks whether corrupt files given to C libraries behind BIL's C layer can crash the process, end it, print, hang it, leak memory, or change state the whole process shares. It was run on 13 October 2026, against specification version 69, with libjpeg-turbo 2.1.5 and OpenJPEG 2.5.0, the versions Ubuntu 24.04 provides.

## Result

**Passed** for libjpeg-turbo and OpenJPEG, here and in CI on Linux on x86 and ARM. GDAL, the assessment's second library, is not installed in this workspace and is left for CI (below).

| Corrupt files | Decoded | Decoded, with corrupt data recovered from | Errors | Refused by the limits |
| --- | --- | --- | --- | --- |
| 4,000,000 JPEG | 135,289 | 475,130 | 3,389,342 | 239 |
| 800,000 JPEG 2000 codestreams | 97,368 | 372 | 702,097 | 163 |
| 400,000 JP2 files | 47,055 | 289 | 352,656 | 0 |

The full run took five minutes on this machine's two cores; the check CI runs uses a tenth as many files and takes about 30 seconds. Over all 5.2 million files:

- **No crash, no exit, no hang.** Every worker finished, with status 0, within its time.
- **Nothing printed.** Every worker's standard error stayed empty.
- **No leak.** Heap in use after each file, once its samples were freed, was exactly what it was before the file, after each worker's first 50 files: 0 bytes left over all of them.
- **No process state changed.** The locale, every signal's handler and flags, the floating-point modes, the file mode mask, the working directory, the environment and the open file descriptors were the same after each worker's files as before. A check of the check makes three such changes (flush-to-zero, a signal handler, the file mode mask) and finds each one, and finds each undone.
- **The same answer twice.** Every tenth file was decoded twice, with the same outcome and the same samples each time, so no decode read memory it had not written.
- **Limits before memory.** Headers that claim 65,000 × 65,000 and 65,536 × 65,536 pixels are refused before decoding, with no memory taken for them.
- **Valid files still decode.** The twelve seed files decode: JPEG within 37 dB of the original, JPEG 2000 sample for sample.

**Without the layer,** libjpeg's own error handling, given a file that is not a JPEG, printed "Not a JPEG file: starts with 0x00 0x00" to standard error and ended the process with status 1. That is what the layer prevents. OpenJPEG with no handlers set printed nothing and returned: it reports only through the callbacks a caller sets.

## How it works

| Part | File | What it does |
| --- | --- | --- |
| The C layer | `c/shim.c` | One function for each operation, returning a status whatever the library does. libjpeg's `error_exit` formats its message into the caller's buffer and `longjmp`s back to the function that called the library, which frees everything and returns; the jump crosses only C frames. Warnings are counted, never printed. A progress monitor stops a decode after 500 progressive scans. OpenJPEG's callbacks copy the message. Each header's claimed size is checked against the caller's limits before any pixel memory is taken. |
| Process state | `c/state.c` | A description of the state a library could change, compared before and after; heap in use from glibc's `mallinfo2` |
| The harness | `src/main.rs` | Encodes the seed files with the libraries themselves, makes each corrupt file from its index by one to eight seeded mutations (bit flips, bytes after markers, cuts, repeats, insertions, splices between seeds), and runs workers, copies of itself, over ranges of indexes |
| Headers | `vendor/` | The libraries' headers, which this workspace's runtimes came without (`vendor/README.md`) |

- **A supervisor and its workers.** A worker prints each file's index before decoding it, so a crash or a hang names the file, which is made again from its index alone. The supervisor watches each worker's status, signal, standard error and time.
- **Heap counted exactly.** glibc's count of heap in use includes blocks held in its per-thread caches and fast bins, so workers run with both turned off (`GLIBC_TUNABLES`), and the count then falls back exactly when memory is freed. With them on, the count wandered by tens of kilobytes either way, though nothing leaked.

## What it shows for the design

- **The C layer works as section 2 states.** A callback, or a library's error path, never lets a `longjmp` cross into BIL's Rust: the `setjmp` is in the C function that calls the library.
- **Warnings are corrupt data.** About one corrupt JPEG file in eight decodes "successfully" with libjpeg warnings, its recovery from corrupt data, such as a file cut short and filled with gray. Section 3.3 makes corrupt data an error by default, and `corrupt_as_missing` delivers it as `Partial`. A warning should therefore be an error by default, and, under `corrupt_as_missing`, the rows libjpeg filled in should be marked missing; libjpeg does not say which they were. This is a proposal for the author.
- **Floating-point flags change; modes do not.** OpenJPEG's float arithmetic on corrupt data sets the denormal-operand flag (x86's DE, ARM's IDC), a status bit that ordinary arithmetic sets too. No library changed a mode, such as flush-to-zero, which would change BIL's float32 results.
- **libjpeg's memory needs no special care.** Destroying the decompressor on the error path frees all it allocated, every time.

## What it leaves

- **GDAL.** The assessment names GDAL beside a libjpeg-family library. GDAL has state of its own that the comparison should cover: its error handler stack, its configuration options and its driver registry. It is not installed in this workspace; a CI job that installs it is the next part of this experiment.
- **A fuzzer guided by coverage.** These mutations are blind. Coverage-guided fuzzing, such as cargo-fuzz, finds deeper paths; it cannot be fetched here. Section 10.3 requires fuzz testing only of BIL's own readers, and relies on isolation for C and C++ libraries, which this experiment does not replace: a library flaw that corrupts memory without crashing would pass these checks.
- **Other systems.** macOS's and Windows' libraries come in other versions, and the libjpeg 6.2 interface that Homebrew's libjpeg-turbo offers by default has other structure sizes.
- **The scan limit.** No corrupt file reached 500 progressive scans, so the monitor that stops one is in place but untested.

## Running it

```sh
cd experiments/exp3-c-errors
cargo run --release -- --check   # a tenth of the files, as CI runs it
cargo run --release              # all 5.2 million
```

It needs the runtimes of libjpeg-turbo 2.1.5 and OpenJPEG 2.5.0 (`libjpeg-turbo8` and `libopenjp2-7` on Ubuntu 24.04) and a C compiler. It is a workspace of its own, since it links libraries that only some systems have.
