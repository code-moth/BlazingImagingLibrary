Experiment 3 of the implementability assessment, section 8: containing C libraries' errors (H2). It asks whether corrupt files given to C libraries behind BIL's C layer can crash the process, end it, print, hang it, leak memory, or change state the whole process shares. It was run on 13 October 2026, against specification version 69, with libjpeg-turbo 2.1.5 and OpenJPEG 2.5.0, the versions Ubuntu 24.04 provides; and on 14 October 2026, against version 71, with GDAL 3.8.4, the version Ubuntu 24.04 provides, reading NITF and TIFF (the part for GDAL, below).

## Result

**Passed for libjpeg-turbo and OpenJPEG,** here and in CI on Linux on x86 and ARM, **and for GDAL** here, with one cap added to the layer that the run showed was needed (below). GDAL's part runs in CI too, against Ubuntu's own package, and passes on Linux on x86 and ARM.

| Corrupt files | Decoded | Decoded, with corrupt data recovered from | Errors | Refused by the limits |
| --- | --- | --- | --- | --- |
| 4,000,000 JPEG | 135,289 | 475,130 | 3,389,342 | 239 |
| 800,000 JPEG 2000 codestreams | 97,368 | 372 | 702,097 | 163 |
| 400,000 JP2 files | 47,055 | 289 | 352,656 | 0 |
| 400,000 NITF files, through GDAL | 88,059 | 16,405 | 295,535 | 1 |
| 4,000,000 TIFF files, through GDAL's GeoTIFF driver | 508,913 | 23,133 | 3,466,816 | 1,138 |

The full run for libjpeg-turbo and OpenJPEG took five and a half minutes on this machine's two cores, and GDAL's nine; the checks CI runs use a tenth as many files and take about 40 seconds and a minute. Over all 5.2 million files of the first three kinds, and the 4.4 million of the last two alike:

- **No crash, no exit.** Every worker finished, with status 0.
- **No hang.** The slowest file, a JPEG 2000 codestream, took under 0.6 s, against a limit of 5 s for one file.
- **Nothing printed.** Every worker's standard error stayed empty.
- **No leak.** Heap in use after each file, once its samples were freed, was exactly what it was before the file, after each worker's first 50 files: 0 bytes left over all of them.
- **No process state changed.** The locale; every signal's handler, flags and mask, real-time signals included, and the blocked signals; the floating-point modes; the file mode mask; the working directory; the environment; and the open file descriptors were the same after each worker's files as before. A check of the check makes three such changes (flush-to-zero, a signal handler, the file mode mask), finds each one, and finds each undone.
- **No read of memory never written, as far as this can show, in what BIL would deliver.** The first 20,000 files of each kind, decoded in two workers whose memory glibc fills with different bytes, gave the same outcomes, and the same samples wherever the library gave no warning. Every tenth file was also decoded twice in one worker, with the same result. A decode with warnings is compared by its outcome alone, since its samples are never delivered (section 3.3): GDAL showed why (below).
- **Limits before memory.** Headers that claim 65,000 × 65,000 pixels for a JPEG, a NITF file and a TIFF file, and 65,536 × 65,536 for a codestream and a JP2 file, are refused before decoding, with no heap left afterwards; so is a TIFF file whose one block would take 7.85 GB. A progressive JPEG with one scan repeated 600 times is stopped at the layer's limit of 500 scans. No corrupt file reached that limit.
- **Valid files still decode.** The twelve seed files of the first part decode: JPEG at 37.1 dB at worst, JPEG 2000 sample for sample. The eight of GDAL's part, NITF and TIFF made by GDAL itself, decode: the three coded with JPEG at 37.4 dB at worst, the others sample for sample.

**Without the layer,** libjpeg's own error handling, given a file that is not a JPEG, printed "Not a JPEG file: starts with 0x00 0x00" to standard error and ended the process with status 1. That is what the layer prevents. OpenJPEG with no handlers set printed nothing and returned: it reports only through the callbacks a caller sets. GDAL's default handler, given a NITF file and a TIFF file that are neither, printed "ERROR 4: ... not recognized as a supported file format." for each, and returned.

## GDAL's part

GDAL keeps state of its own for the whole process, which the comparison covers beside the process's: the error handler it would call, with its user data, so that a handler left pushed shows as well as a global one replaced; the calling thread's last error; the global and the thread's configuration options; the drivers registered, in order; the datasets open; the files in its memory file system; the block cache's size and use; and the number of shared objects loaded, which a plugin loaded from a search path would raise. The check of the check makes seven such changes, one of each kind, and finds each one and its undoing.

It was run here against GDAL 3.8.4 built from its source with GDAL's own copies of libtiff and libjpeg, since this workspace has no GDAL package and cannot fetch one; CI runs it against Ubuntu 24.04's libgdal-dev, the same version built against the system's libraries.

**How the layer leaves GDAL's state alone.** Every measure is GDAL's own call, scoped to the calling thread or the one dataset:

| GDAL state | What the layer does |
| --- | --- |
| Error handlers | Pushes its handler on the calling thread's stack for the call and pops it after (`CPLPushErrorHandlerEx`); the global handler is never set |
| The thread's last error | Saved before the call and put back after (`CPLErrorSetState`) |
| Configuration options | Sets the two it needs, no directory listing on open and no auxiliary metadata files, for the calling thread alone, and puts back the earlier values (`CPLSetThreadLocalConfigOption`); no environment variable is read or set (section 10.5) |
| Driver registry | Registers the four drivers it uses one by one (`GDALRegister_NITF` and the like), which loads no plugin, as `GDALAllRegister` would from a search path (section 10.5); opens each Source with only its driver allowed (`GDALOpenEx`) |
| Memory files and datasets | Hands the bytes to GDAL as one memory file, removed after the call; closes every dataset it opens |

Over the 4.4 million files nothing in that table changed, and the block cache was back at its starting use after every file.

**Two findings that changed the layer or the check**

- **Pixels never written, delivered with only warnings.** On a corrupt JPEG tile (TIFF file 2,861), libtiff warned "Improper JPEG strip/tile size" three times and gave up on the tile, and GDAL returned success with the tile's pixels taken from a block buffer it had allocated and never filled. valgrind traces the values to the allocation in `GDALRasterBlock::Internalize`. The two workers with differently filled memory found it: their samples differed. Under section 3.3 a warning is an error, and under `corrupt_as_missing` the whole block is missing, so BIL never delivers such pixels; the check therefore compares what BIL would deliver. This is the strongest case yet for M16.
- **A block far larger than its Image.** In the first full run, the operating system ended one worker (signal 9, from the kernel's out-of-memory killer) at TIFF file 1,197,389: 40 × 40 pixels of 14,974 bands, 24 MB and within the layer's limits, stored in one tile of 16 × 32,784 pixels, for which GDAL's GeoTIFF driver asked 7.85 GB at once. Linux granted it, overcommitting, and ended the process when it was used. Section 10.3 caps a library's working memory at twice `max_decoded_bytes` "where the library allows a cap"; GDAL allows none, but its working memory follows the block size it reports, so the layer now refuses a block whose size, over every band, exceeds that cap, before anything is read. The rerun passed, and the file is part of the limits check. Proposed as M22 in the user manual's chapter 24.

## How it works

| Part | File | What it does |
| --- | --- | --- |
| The C layer | `c/shim.c` | One function for each operation, returning a status whatever the library does. libjpeg's `error_exit` formats its message into the caller's buffer and `longjmp`s back to the function that called the library, which frees everything and returns; the jump crosses only C frames. Warnings are counted, never printed. A progress monitor stops a decode after 500 progressive scans. libjpeg's working memory is capped. OpenJPEG's callbacks copy the message, and it is told not to apply a JP2 file's palette, component mapping or channel definitions. Each header's claimed size is checked against the caller's limits before pixel memory is taken, and the decoded size again before the layer copies it. |
| Process state | `c/state.c` | A description of the state a library could change, compared before and after; the floating-point exception flags, reported apart; heap in use from glibc's `mallinfo2` |
| The harness | `src/main.rs` | Encodes the seed files with the libraries themselves, makes each corrupt file from its index by one to eight seeded mutations (bit flips, bytes after markers, cuts, repeats, insertions, splices between seeds), and runs workers, copies of itself, over ranges of indexes |
| Headers | `vendor/` | The libraries' headers, which this workspace's runtimes came without (`vendor/README.md`). The program checks at start that the OpenJPEG linked is 2.5.0; libjpeg checks its own structure sizes. |
| GDAL's layer | `c/gdal.c` | The same, for GDAL: one function for each operation, the state description of GDAL's own, and the seed files, made by GDAL itself. Built only with the `gdal` feature, where libgdal-dev is installed; `c/gdal_stub.c` stands in for it everywhere else, so the program builds on every system. |
| Shared | `c/layer.h` | The statuses, limits and description of a decode that both layers use |

- **A supervisor and its workers.** A worker prints each file's index before decoding it, so a crash or a hang names the file, which is made again from its index alone (`--dump CODEC INDEX FILE` writes it out). The supervisor watches each worker's status, signal, standard error and time.
- **Mutations for NITF.** NITF's headers are text, their lengths, counts and sizes written in digits, so NITF files are also changed by a digit changed in the headers and a run of digits set to all nines or all zeros. Every other format draws exactly as before, so its files are the same as in earlier runs.
- **Heap counted exactly.** glibc's count of heap in use includes blocks held in its per-thread caches and fast bins, so workers run with both turned off (`GLIBC_TUNABLES`), and the count then falls back exactly when memory is freed. With them on, the count wandered by tens of kilobytes either way.

## What it shows for the design

- **A C layer can contain libjpeg's fatal errors,** as H2 of the assessment proposes: the `setjmp` is in the C function that calls the library, so no `longjmp` crosses BIL's Rust, which section 2 forbids.
- **Warnings are corrupt data.** About one corrupt JPEG file in eight decodes "successfully" with libjpeg warnings, its recovery from corrupt data, such as a file cut short and filled with gray. Section 3.3 makes corrupt data an error by default, and `corrupt_as_missing` delivers it as `Partial`. A warning should therefore be an error by default, and, under `corrupt_as_missing`, the block (Fragment) should be marked missing, since libjpeg does not say which rows it filled in. Proposed as M16 in the user manual's chapter 24, and accepted by the author in specification version 70 (section 3.3).
- **A fourth limit.** Section 10.3 named three limits. A progressive JPEG can hold thousands of scans, each a pass over the whole image, so a limit on scans, here 500, belongs beside them, and so does a cap on a library's working memory beside the output, here twice `max_decoded_bytes` for libjpeg. Proposed as M17, with a refusal reported as an error, here told apart only for the checks; accepted in version 70 (section 10.3).
- **Palettes as indices.** A JP2 palette expanded by OpenJPEG adds components after the header is checked, which would break "fails before any memory is taken" (section 10.3). The layer turns that off, and with it OpenJPEG's handling of the component mapping (`cmap`) and the channel definitions (`cdef`), which say, for one, which component is alpha. Since version 70 (section 10.3), as proposed in M18, the driver delivers indices for `palette.lookup`, and applies or reports `cmap` and `cdef` itself.
- **Floating-point flags change; modes do not.** OpenJPEG's float arithmetic on corrupt files raised the denormal, underflow and inexact flags on x86, status bits that ordinary arithmetic sets too. libjpeg raised none. No library changed a mode, such as flush-to-zero, which would change BIL's float32 results.
- **GDAL can be contained by its own calls, scoped to the thread and the dataset,** as the table above shows. Two things remain process-wide and are shared with a caller who uses the same GDAL: the driver registry, to which registering adds, and the block cache, in which BIL's reads take space and evict the caller's blocks until BIL closes its datasets. How BIL's GDAL driver should register drivers in a GDAL it shares with the caller is a question for the author, proposed as M23.
- **GDAL's fatal errors cannot be contained in the process.** An error GDAL reports as `CE_Fatal` calls `abort()` after the handler returns, and the handler cannot jump out, since the jump would cross C++ frames. No corrupt file reached one. Where that risk matters, isolation (section 10.3) is the answer.
- **The error counter moves.** GDAL counts every error a thread reports (`CPLGetErrorCounter`), and nothing puts a count back. A caller that watches the counter sees BIL's errors; the last error itself is restored.
- **libjpeg's decoder needs no special care for memory:** destroying it on the error path frees all it allocated, every time. Its memory destination for encoding does: once its buffer has grown, the caller's pointer is stale until compression finishes, so the layer does not free it on an error.

## What it leaves

- **GDAL's other formats and its JPEG 2000.** GDAL's part reads NITF, uncompressed and with JPEG, and TIFF, uncompressed and with deflate, LZW and JPEG. BIL does not use GDAL's JPEG 2000 (section 10), so no JPEG 2000 driver is registered.
- **Threads.** Each worker decodes on one thread. GDAL's thread-local measures are exactly what concurrent reads need, but concurrent reads are not tested here.
- **A fuzzer guided by coverage.** These mutations are blind. Coverage-guided fuzzing, such as cargo-fuzz, reaches deeper paths; it cannot be fetched here. Section 10.3 requires fuzz testing only of BIL's own readers, and relies on isolation for C and C++ libraries, which this experiment does not replace: a flaw that corrupts memory without crashing would pass these checks.
- **Other systems.** macOS's and Windows' libraries come in other versions, and the libjpeg 6.2 interface that Homebrew's libjpeg-turbo offers by default has other structure sizes.

## Running it

```sh
cd experiments/exp3-c-errors
cargo run --release -- --check                          # a tenth of the files, as CI runs it
cargo run --release                                     # all 5.2 million
cargo run --release --features gdal -- --gdal --check   # GDAL's part, as CI runs it
cargo run --release --features gdal -- --gdal           # all 4.4 million
```

It needs the runtimes of libjpeg-turbo 2.1.5 and OpenJPEG 2.5.0 (`libjpeg-turbo8` and `libopenjp2-7` on Ubuntu 24.04) and a C compiler; GDAL's part needs libgdal-dev, whose `gdal-config` the build asks for GDAL's headers and library. It is a workspace of its own, since it links libraries that only some systems have.
