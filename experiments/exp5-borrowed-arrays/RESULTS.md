Experiment 5 of the implementability assessment, section 8: borrowed NumPy arrays. It asks whether a NumPy array that borrows a memory-mapped file stays valid after the Python object for its Source is gone, under several threads; whether it is read-only under `Shared` mapping; and whether anything leaks. It was run on 13 October 2026, against specification version 69.

## Result

**Passed,** here and in CI on Linux on x86 and ARM, Windows, and macOS on ARM, with Python 3.12, and with Python 3.11 on Linux, the oldest the stable interface allows.

- **The chain of references holds.** `numpy.asarray(fragment)` makes an array whose base is a buffer view of the Fragment, and the Fragment holds the file's mapping. Closing the Source and dropping it leaves every such array valid, with the mapping held by the arrays alone, and the mapping is unmapped when the last of them goes.
- **Read-only under `Shared`.** A borrowed array is not writeable, a write raises `ValueError`, and a request for a writable buffer is refused.
- **The caller's own under `Private`.** A borrowed array is writable, its changes are seen through the same Source, and neither the file nor a `Shared` Source of it changes.
- **`Off` reads the file into memory of BIL's own,** and every array is then a copy, as section 4 says.
- **Strides, not copies.** A Window with a `step` is a borrowed strided view. The buffer protocol grants a strided view only to a request that takes strides, and refuses requests for C, Fortran or any contiguity that the view does not meet.
- **The zero-copy rule's alignment.** Samples that do not start on a multiple of their size are copied once, with the same values.
- **Slices.** `pixel.raw` of float32 data is borrowed, as section 6 requires; of `U16` data it is converted once into a writable array of its own.
- **Threads.** Eight threads making 1,600 arrays, with their Sources closed or dropped under them: no failure, every kept array correct, nothing left once they go.
- **The interpreter lock is released during every read.** Another Python thread ran throughout the middle of each conversion of 16 million samples, which took 42 to 48 ms here.
- **Nothing leaks.** 50,000 Fragments and Slices made, viewed through NumPy and memoryview, and released: no object, view or mapping left, the Source's reference count back where it was, and resident memory unchanged on Linux, where it was measured here; CI measures it on Windows too.
- **Errors, not crashes.** Eight bad calls each raise an error. Fragments cannot be made, and the types cannot be changed, from Python.

## How it works

`bil_exp5` is a Python extension module of about 1,300 lines of Rust.

| Part | File | What it does |
| --- | --- | --- |
| The stable interface | `src/ffi.rs` | Declarations of the parts of Python's stable interface the module uses, by hand from CPython's headers: the object header, a module definition, type specifications and slots, method and attribute tables, and the buffer structure. On Windows the data symbols are imported through `python3.lib`. |
| The file in memory | `src/map.rs` | `mmap` on Linux and macOS, `MapViewOfFile` on Windows. `Shared` maps read-only, `Private` copy-on-write, and `Off` reads the file into memory. Held through an `Arc`, released with the last holder. |
| Sources and arrays | `src/lib.rs` | A Source of raw samples in native byte order. `read_fragment` and `read_slice` return objects that export the buffer protocol, each holding its memory: the mapping for a borrowed array, a buffer of its own for a copy. |
| Linking | `build.rs` | Nothing on Linux; `-undefined dynamic_lookup` on macOS; the stable interface's import library on Windows, with MSVC. |
| The checks | `check.py` | Copies the built library to a fresh folder under the name Python imports, and runs the checks above. |

- **No PyO3.** This workspace cannot reach crates.io, so the module declares the stable interface itself. The mechanism the experiment tests, a buffer view whose object keeps the memory alive, is the one any binding uses, and NumPy builds its array on it in the same way.
- **The lock released.** A read copies the Source's `Arc` of the mapping, releases the lock, reads, and takes the lock back before it makes any Python object, so another thread may close or drop the Source meanwhile.
- **One stable build.** The module is built once against the stable interface and loads in Python 3.11, 3.12 and 3.13 unchanged.

## What it shows for the design

- **Section 4 holds as written for `Shared` and `Off`.** No change is needed for borrowed arrays to outlive their Source.
- **`close()` can free everything but the borrowed memory.** Here a closed Source refuses reads, and arrays borrowed from it keep only the mapping. Section 4 says a borrowed array "keeps its Source open"; this is the narrower form, proposed for the author in the user manual's chapter 24, M3.
- **`Private` needs one more sentence.** Writes through one borrowed array are seen by every array borrowed from the same Source, since they share the copy-on-write pages. Section 4 says only that changes stay in the caller's memory. A caller who writes through a `Private` array while another thread reads the same pages makes a data race in its own code; BIL never writes them.
- **Two statements of the specification differ.** Section 12, rule 9 says "Borrowed arrays are read-only", while section 4 allows writes under `Private`. The code follows section 4.
- **The Python interface can be small.** About 190 lines declare all of the stable interface it needs. Whether the full Python interface is built on PyO3, as the implementability assessment assumes, or on declarations of BIL's own, is for the author; PyO3 is the larger and better tested route once CI can fetch it.

## What it leaves

- **The real interface.** `mapping` and `sample_type` are passed as text, `read_slice` takes no Slice name and makes only `pixel.raw.intensity`, and a Window is passed as loose numbers with no frame.
- **Byte order.** Only native byte order is read; swapped data would be converted, never borrowed, by section 4.
- **Free-threaded Python** has no stable interface and is not supported.
- **Files that change** while mapped are not supported by section 4, and are not tested.

## Running it

```sh
cargo build --release -p bil-exp5
python3 -I experiments/exp5-borrowed-arrays/check.py target/release
```

It needs NumPy. In CI, the step `Experiment 5, borrowed arrays` runs it on every system.
