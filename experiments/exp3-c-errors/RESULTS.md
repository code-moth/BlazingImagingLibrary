Experiment 3 of the implementability assessment, section 8: containing C libraries' errors (H2). It asks whether corrupt files given to C libraries behind BIL's C layer can crash the process, end it, print, hang it, leak memory, or change state the whole process shares. It was run on 13 October 2026, against specification version 69, with libjpeg-turbo 2.1.5 and OpenJPEG 2.5.0, the versions Ubuntu 24.04 provides.

## Result

**Passed for libjpeg-turbo and OpenJPEG,** here and in CI on Linux on x86 and ARM. **GDAL is still open:** the assessment names it beside a libjpeg-family library, and it is not installed in this workspace (below).

| Corrupt files | Decoded | Decoded, with corrupt data recovered from | Errors | Refused by the limits |
| --- | --- | --- | --- | --- |
| 4,000,000 JPEG | 135,289 | 475,130 | 3,389,342 | 239 |
| 800,000 JPEG 2000 codestreams | 97,368 | 372 | 702,097 | 163 |
| 400,000 JP2 files | 47,055 | 289 | 352,656 | 0 |

The full run took five and a half minutes on this machine's two cores; the check CI runs uses a tenth as many files and takes about 40 seconds. Over all 5.2 million files:

- **No crash, no exit.** Every worker finished, with status 0.
- **No hang.** The slowest file, a JPEG 2000 codestream, took under 0.6 s, against a limit of 5 s for one file.
- **Nothing printed.** Every worker's standard error stayed empty.
- **No leak.** Heap in use after each file, once its samples were freed, was exactly what it was before the file, after each worker's first 50 files: 0 bytes left over all of them.
- **No process state changed.** The locale; every signal's handler, flags and mask, real-time signals included, and the blocked signals; the floating-point modes; the file mode mask; the working directory; the environment; and the open file descriptors were the same after each worker's files as before. A check of the check makes three such changes (flush-to-zero, a signal handler, the file mode mask), finds each one, and finds each undone.
- **No read of memory never written, as far as this can show.** The first 20,000 files of each kind, decoded in two workers whose memory glibc fills with different bytes, gave the same outcomes and samples. Every tenth file was also decoded twice in one worker, with the same result.
- **Limits before memory.** Headers that claim 65,000 × 65,000 pixels for a JPEG and 65,536 × 65,536 for a codestream and a JP2 file are refused before decoding, with no heap left afterwards. A progressive JPEG with one scan repeated 600 times is stopped at the layer's limit of 500 scans. No corrupt file reached that limit.
- **Valid files still decode.** The twelve seed files decode: JPEG at 37.1 dB at worst, JPEG 2000 sample for sample.

**Without the layer,** libjpeg's own error handling, given a file that is not a JPEG, printed "Not a JPEG file: starts with 0x00 0x00" to standard error and ended the process with status 1. That is what the layer prevents. OpenJPEG with no handlers set printed nothing and returned: it reports only through the callbacks a caller sets.

## How it works

| Part | File | What it does |
| --- | --- | --- |
| The C layer | `c/shim.c` | One function for each operation, returning a status whatever the library does. libjpeg's `error_exit` formats its message into the caller's buffer and `longjmp`s back to the function that called the library, which frees everything and returns; the jump crosses only C frames. Warnings are counted, never printed. A progress monitor stops a decode after 500 progressive scans. libjpeg's working memory is capped. OpenJPEG's callbacks copy the message, and it is told not to apply a JP2 file's palette, component mapping or channel definitions. Each header's claimed size is checked against the caller's limits before pixel memory is taken, and the decoded size again before the layer copies it. |
| Process state | `c/state.c` | A description of the state a library could change, compared before and after; the floating-point exception flags, reported apart; heap in use from glibc's `mallinfo2` |
| The harness | `src/main.rs` | Encodes the seed files with the libraries themselves, makes each corrupt file from its index by one to eight seeded mutations (bit flips, bytes after markers, cuts, repeats, insertions, splices between seeds), and runs workers, copies of itself, over ranges of indexes |
| Headers | `vendor/` | The libraries' headers, which this workspace's runtimes came without (`vendor/README.md`). The program checks at start that the OpenJPEG linked is 2.5.0; libjpeg checks its own structure sizes. |

- **A supervisor and its workers.** A worker prints each file's index before decoding it, so a crash or a hang names the file, which is made again from its index alone. The supervisor watches each worker's status, signal, standard error and time.
- **Heap counted exactly.** glibc's count of heap in use includes blocks held in its per-thread caches and fast bins, so workers run with both turned off (`GLIBC_TUNABLES`), and the count then falls back exactly when memory is freed. With them on, the count wandered by tens of kilobytes either way.

## What it shows for the design

- **A C layer can contain libjpeg's fatal errors,** as H2 of the assessment proposes: the `setjmp` is in the C function that calls the library, so no `longjmp` crosses BIL's Rust, which section 2 forbids.
- **Warnings are corrupt data.** About one corrupt JPEG file in eight decodes "successfully" with libjpeg warnings, its recovery from corrupt data, such as a file cut short and filled with gray. Section 3.3 makes corrupt data an error by default, and `corrupt_as_missing` delivers it as `Partial`. A warning should therefore be an error by default, and, under `corrupt_as_missing`, the block (Fragment) should be marked missing, since libjpeg does not say which rows it filled in. Proposed as M16 in the user manual's chapter 24, and accepted by the author in specification version 70 (section 3.3).
- **A fourth limit.** Section 10.3 named three limits. A progressive JPEG can hold thousands of scans, each a pass over the whole image, so a limit on scans, here 500, belongs beside them, and so does a cap on a library's working memory beside the output, here twice `max_decoded_bytes` for libjpeg. Proposed as M17, with a refusal reported as an error, here told apart only for the checks; accepted in version 70 (section 10.3).
- **Palettes as indices.** A JP2 palette expanded by OpenJPEG adds components after the header is checked, which would break "fails before any memory is taken" (section 10.3). The layer turns that off, and with it OpenJPEG's handling of the component mapping (`cmap`) and the channel definitions (`cdef`), which say, for one, which component is alpha. Since version 70 (section 10.3), as proposed in M18, the driver delivers indices for `palette.lookup`, and applies or reports `cmap` and `cdef` itself.
- **Floating-point flags change; modes do not.** OpenJPEG's float arithmetic on corrupt files raised the denormal, underflow and inexact flags on x86, status bits that ordinary arithmetic sets too. libjpeg raised none. No library changed a mode, such as flush-to-zero, which would change BIL's float32 results.
- **libjpeg's decoder needs no special care for memory:** destroying it on the error path frees all it allocated, every time. Its memory destination for encoding does: once its buffer has grown, the caller's pointer is stale until compression finishes, so the layer does not free it on an error.

## What it leaves

- **GDAL.** GDAL has state of its own that the comparison should cover: its error handler stack, its configuration options and its driver registry. A CI job that installs it is the next part of this experiment.
- **A fuzzer guided by coverage.** These mutations are blind. Coverage-guided fuzzing, such as cargo-fuzz, reaches deeper paths; it cannot be fetched here. Section 10.3 requires fuzz testing only of BIL's own readers, and relies on isolation for C and C++ libraries, which this experiment does not replace: a flaw that corrupts memory without crashing would pass these checks.
- **Other systems.** macOS's and Windows' libraries come in other versions, and the libjpeg 6.2 interface that Homebrew's libjpeg-turbo offers by default has other structure sizes.

## Running it

```sh
cd experiments/exp3-c-errors
cargo run --release -- --check   # a tenth of the files, as CI runs it
cargo run --release              # all 5.2 million
```

It needs the runtimes of libjpeg-turbo 2.1.5 and OpenJPEG 2.5.0 (`libjpeg-turbo8` and `libopenjp2-7` on Ubuntu 24.04) and a C compiler. It is a workspace of its own, since it links libraries that only some systems have.
