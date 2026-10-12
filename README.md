# Blazing Imaging Library (BIL)

BIL extracts a processable subset of imaging sensor data, efficiently, from any format. It delivers that subset as native Fragments or as 2D float32 Slices, and locates every pixel in the world through one geocoding chain.

BIL is written in Rust, with a C-callable interface over it delivered as one shared library, and a Python interface. It is licensed under the LGPL, version 2.1 or later, with a static-linking exception.

**Status.** Design, and the first experiments. There is no usable library yet.

## What is here

| Path | Contents |
| --- | --- |
| `docs/specification/` | The specification, version 61: Markdown source and the rendered page |
| `docs/reviews/` | The two red team reviews of the specification |
| `docs/assessments/` | BIL by Example, and the implementability assessment |
| `docs/tools/` | The scripts that render the documents |
| `crates/bil/`, `crates/bil-sys/`, `python/` | The packages: early previews that hold the names |
| `testdata/` | The manifest of real test files and the script that fetches them |
| `experiments/exp1-fused-loop/` | Experiment 1: the fused loop, every Level in one pass, bitwise Window independence |
| `CLAUDE.md` | Working notes: the state of the work and the next step |

## Experiment 1

```sh
cargo run --release --bin exp1             # the full report, including speed
cargo run --release --bin exp1 -- --check  # correctness only, as CI runs it
```

The results are in `experiments/exp1-fused-loop/RESULTS.md`.

## License

GNU Lesser General Public License, version 2.1 or, at your option, any later version, with a static-linking exception. See `LICENSE` and `LINKING-EXCEPTION.md`.
