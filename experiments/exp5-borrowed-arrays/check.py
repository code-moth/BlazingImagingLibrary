# SPDX-License-Identifier: LGPL-2.1-or-later
"""Experiment 5: borrowed NumPy arrays. Checks the extension module `bil_exp5`.

    cargo build --release -p bil-exp5
    python3 experiments/exp5-borrowed-arrays/check.py target/release

Copies the built library into a fresh folder under the name Python imports, then checks that a
NumPy array borrowed from a mapped file stays valid after its Source is closed and dropped, under
several threads; is read-only under Shared mapping and the caller's own under Private; is a
strided view for a decimation step; is copied when the zero-copy rule fails; that reads release
the interpreter lock; and that nothing leaks. Prints one line for each check, and exits 1 if any
fails.
"""

import gc
import os
import pathlib
import shutil
import sys
import tempfile
import threading
import time

import numpy as np

RESULTS = []


def report(name, ok, detail):
    RESULTS.append(ok)
    print(f"{name}: {detail}. Passed: **{ok}**\n")


def load(build):
    names = {"linux": "libbil_exp5.so", "darwin": "libbil_exp5.dylib", "win32": "bil_exp5.dll"}
    built = pathlib.Path(build) / names.get(sys.platform, "libbil_exp5.so")
    folder = pathlib.Path(tempfile.mkdtemp(prefix="bil_exp5_"))
    target = "bil_exp5.pyd" if sys.platform == "win32" else "bil_exp5.abi3.so"
    shutil.copy(built, folder / target)
    sys.path.insert(0, str(folder))
    import bil_exp5

    return bil_exp5


def settled(b):
    gc.collect()
    return b.live_mappings(), b.live_arrays(), b.live_exports()


def main():
    b = load(sys.argv[1] if len(sys.argv) > 1 else "target/release")
    work = pathlib.Path(tempfile.mkdtemp(prefix="bil_exp5_data_"))
    rows, cols = 1000, 1200
    rng = np.random.default_rng(5)
    u16 = rng.integers(0, 65536, size=(rows, cols), dtype=np.uint16)
    f32 = rng.standard_normal(size=(rows, cols)).astype(np.float32)
    u16_path, f32_path, odd_path = work / "u16.raw", work / "f32.raw", work / "odd.raw"
    u16.tofile(u16_path)
    f32.tofile(f32_path)
    # One byte before the samples, so that no U16 sample sits on an even address.
    odd_path.write_bytes(b"\0" + u16.tobytes())

    # 1. Borrowed, read-only, equal to the file.
    s = b.open_source(str(u16_path), rows, cols, "U16")
    f = s.read_fragment(100, 200, 300, 400)
    a = np.asarray(f)
    equal = np.array_equal(a, u16[100:400, 200:600])
    shared = f.is_borrowed and not a.flags.writeable and not a.flags.owndata
    try:
        a[0, 0] = 1
        refused = False
    except ValueError:
        refused = True
    m = memoryview(f)
    view_ok = m.readonly and m.shape == (300, 400) and m.strides == (cols * 2, 2) and m.format == "H"
    m.release()
    report(
        "Borrowed under Shared mapping",
        equal and shared and refused and view_ok,
        f"values equal to the file: {equal}; borrowed and read-only: {shared}; a write refused: "
        f"{refused}; the buffer's shape, strides and format as stated: {view_ok}",
    )

    # 2. A decimation step is a strided view, still borrowed.
    st = s.read_fragment(1, 2, 999, 1198, step=3)
    sa = np.asarray(st)
    strided = (
        st.is_borrowed
        and np.array_equal(sa, u16[1:1000:3, 2:1200:3])
        and sa.strides == (cols * 2 * 3, 2 * 3)
    )
    report("A step of 3 as strides", strided, f"borrowed, equal and strided: {strided}")

    # 3. The array outlives its Source: closed, then dropped.
    s.close()
    closed_refused = False
    try:
        s.read_fragment(0, 0, 1, 1)
    except ValueError:
        closed_refused = True
    del s, f, st
    gc.collect()
    alive = b.live_mappings() == 1 and np.array_equal(a, u16[100:400, 200:600])
    alive &= np.array_equal(sa, u16[1:1000:3, 2:1200:3])
    del a, sa
    after = settled(b)
    report(
        "Outliving the Source",
        closed_refused and alive and after == (0, 0, 0),
        f"a read after close refused: {closed_refused}; arrays valid with the Source closed and "
        f"dropped, the mapping held by them alone: {alive}; mappings, arrays and views left once "
        f"they go: {after}",
    )

    # 4. Private mapping: the caller's own copy, the file untouched.
    p = b.open_source(str(u16_path), rows, cols, "U16", mapping="Private")
    pa = np.asarray(p.read_fragment(0, 0, 10, 10))
    pa[:] = 7
    again = np.asarray(p.read_fragment(0, 0, 10, 10))
    on_disk = np.fromfile(u16_path, dtype=np.uint16).reshape(rows, cols)
    other = np.asarray(b.open_source(str(u16_path), rows, cols, "U16").read_fragment(0, 0, 10, 10))
    private = (
        pa.flags.writeable
        and (again == 7).all()
        and np.array_equal(on_disk, u16)
        and np.array_equal(other, u16[:10, :10])
    )
    del p, pa, again, other
    report(
        "Private mapping",
        private and settled(b) == (0, 0, 0),
        f"writable, changes seen through the same Source, the file and a Shared Source "
        f"unchanged: {private}",
    )

    # 5. Float32 Slices borrow; others are converted once into arrays of their own.
    fs = b.open_source(str(f32_path), rows, cols, "F32")
    sl = fs.read_slice(5, 6, 70, 80)
    borrowed_slice = sl.is_borrowed and np.array_equal(np.asarray(sl), f32[5:75, 6:86])
    us = b.open_source(str(u16_path), rows, cols, "U16").read_slice(5, 6, 70, 80, step=2)
    ua = np.asarray(us)
    converted = (
        not us.is_borrowed
        and ua.dtype == np.float32
        and ua.flags.writeable
        and np.array_equal(ua, u16[5:75:2, 6:86:2].astype(np.float32))
    )
    del fs, sl, us, ua
    report(
        "Slices",
        borrowed_slice and converted and settled(b) == (0, 0, 0),
        f"`pixel.raw` of float32 data borrowed: {borrowed_slice}; of U16 data converted once, "
        f"its own and writable: {converted}",
    )

    # 6. The zero-copy rule's alignment: misaligned samples are copied, with the same values.
    o = b.open_source(str(odd_path), rows, cols, "U16", offset=1)
    of = o.read_fragment(10, 20, 30, 40, step=2)
    oa = np.asarray(of)
    copied = not of.is_borrowed and np.array_equal(oa, u16[10:40:2, 20:60:2])
    del o, of, oa
    report(
        "Misaligned samples",
        copied and settled(b) == (0, 0, 0),
        f"copied rather than borrowed, values equal: {copied}",
    )

    # 7. Errors, not crashes.
    errors = 0
    for call in (
        lambda: b.open_source(str(u16_path), rows + 1, cols, "U16"),
        lambda: b.open_source(str(u16_path), rows, cols, "U32"),
        lambda: b.open_source(str(u16_path), rows, cols, mapping="Off"),
        lambda: b.open_source(str(work / "absent.raw"), 1, 1),
        lambda: b.open_source(str(u16_path), rows, cols).read_fragment(0, 0, rows + 1, 1),
        lambda: b.open_source(str(u16_path), rows, cols).read_fragment(0, 0, 0, 1),
        lambda: b.open_source(str(u16_path), rows, cols).read_fragment(-1, 0, 1, 1),
        lambda: b.open_source(str(u16_path), rows, cols).read_fragment(0, 0, 1, 1, step=0),
    ):
        try:
            call()
        except (ValueError, OSError):
            errors += 1
    try:
        b.Fragment()
        constructed = True
    except TypeError:
        constructed = False
    report(
        "Errors",
        errors == 8 and not constructed and settled(b) == (0, 0, 0),
        f"{errors} of 8 bad calls raised an error; a Fragment cannot be made from Python: "
        f"{not constructed}",
    )

    # 8. Threads: arrays made on many threads while their Sources are closed and dropped under
    # them, kept, checked and released.
    kept, failures, lock = [], [], threading.Lock()

    def worker(seed):
        r = np.random.default_rng(seed)
        try:
            for _ in range(200):
                src = b.open_source(str(u16_path), rows, cols, "U16")
                r0, c0 = int(r.integers(0, rows - 50)), int(r.integers(0, cols - 50))
                arr = np.asarray(src.read_fragment(r0, c0, 50, 50, step=int(r.integers(1, 4))))
                if r.integers(0, 2):
                    src.close()
                del src
                with lock:
                    kept.append((r0, c0, arr))
                    if len(kept) > 64:
                        kept.pop(int(r.integers(0, len(kept))))
        except Exception as e:  # noqa: BLE001, reported below
            failures.append(repr(e))

    threads = [threading.Thread(target=worker, args=(t,)) for t in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    wrong = 0
    for r0, c0, arr in kept:
        step = (arr.strides[1]) // 2
        wrong += not np.array_equal(arr, u16[r0 : r0 + 50 : step, c0 : c0 + 50 : step])
    survivors = len(kept)
    del kept, arr
    left = settled(b)
    report(
        "Threads",
        not failures and wrong == 0 and left == (0, 0, 0),
        f"8 threads × 200 reads, Sources closed or dropped under their arrays: {len(failures)} "
        f"failures; {survivors} arrays kept to the end, {wrong} wrong; left once they go: {left}",
    )

    # 9. Reads release the interpreter lock: another thread runs during a long conversion.
    big_rows, big_cols = 4000, 4000
    big = work / "big.raw"
    np.zeros((big_rows, big_cols), dtype=np.uint16).tofile(big)
    bs = b.open_source(str(big), big_rows, big_cols, "U16")
    ticks, stop = [], threading.Event()

    def ticker():
        while not stop.is_set():
            ticks.append(time.perf_counter())

    t = threading.Thread(target=ticker)
    t.start()
    time.sleep(0.02)
    spans = []
    for _ in range(5):
        start = time.perf_counter()
        bs.read_slice(0, 0, big_rows, big_cols)
        spans.append((start, time.perf_counter()))
    stop.set()
    t.join()
    # Ticks inside the middle of a read: none can happen if the read holds the lock.
    inside = sum(
        1
        for a0, a1 in spans
        for x in ticks
        if a0 + 0.25 * (a1 - a0) < x < a1 - 0.25 * (a1 - a0)
    )
    took = sum(a1 - a0 for a0, a1 in spans) / len(spans)
    del bs
    report(
        "The interpreter lock",
        inside > 0 and settled(b) == (0, 0, 0),
        f"another thread ran {inside} times in the middle half of five reads of "
        f"{took * 1000:.0f} ms each",
    )

    # 10. Leaks: many reads, made and released; the process's memory does not grow with them.
    s = b.open_source(str(u16_path), rows, cols, "U16")
    base_refs = sys.getrefcount(s)

    def churn(n):
        for i in range(n):
            arr = np.asarray(s.read_fragment(i % 900, i % 1100, 100, 100))
            m = memoryview(s.read_slice(0, 0, 8, 8))
            del arr, m

    churn(2000)
    gc.collect()
    before = rss()
    churn(50000)
    gc.collect()
    grown = rss() - before
    refs_back = sys.getrefcount(s) == base_refs
    del s
    left = settled(b)
    report(
        "Leaks",
        refs_back and left == (0, 0, 0) and (grown is None or grown < 8 * 2**20),
        f"50,000 Fragments and Slices made, viewed and released: the Source's references "
        f"back to {base_refs}: {refs_back}; left: {left}; resident memory grew by "
        f"{'unmeasured' if grown is None else f'{grown / 2**20:.1f} MB'}",
    )

    shutil.rmtree(work, ignore_errors=True)
    ok = all(RESULTS)
    print(f"All checks of borrowed arrays passed: **{ok}**")
    print(f"Python {sys.version.split()[0]}, NumPy {np.__version__}, {sys.platform}")
    return 0 if ok else 1


def rss():
    """Resident memory in bytes, where the system reports it cheaply."""
    try:
        if sys.platform == "linux":
            with open("/proc/self/statm") as f:
                return int(f.read().split()[1]) * os.sysconf("SC_PAGE_SIZE")
        if sys.platform == "darwin":
            import resource

            # Peak, in bytes on macOS: growth past the warm-up peak still shows.
            return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    except OSError:
        pass
    return None


if __name__ == "__main__":
    sys.exit(main())
