# Working notes for BIL

Read this first in every session. It records the state of the work, how it is done, and the next step. Update it at the end of every session.

## The arrangement

- **Claude writes BIL.** The specification's author leads, rules on every design decision, reviews each change, and supplies test data and other systems.
- **The specification is the source of truth.** `docs/specification/specification.md`, version 60. A design change is made there first, by the author's ruling, and the version number goes up.
- **Ask, don't assume, on design.** Where the specification leaves something open, propose and flag it. Do not decide it silently.

## Conventions

- **Naming.** Types and enum values are UpperCamelCase. Every other name is snake_case, including names passed as text: `color.hsv.hue`, `level3`, `stored2`, `bilinear`, `rational_polynomial`. C uses the prefixes `bil_`, `Bil` and `BIL_`.
- **American spelling** in identifiers, comments and documents.
- **License.** LGPL 2.1 or later, with a static-linking exception (`LINKING-EXCEPTION.md`, a draft pending legal review). BIL's own packages contain only LGPL-compatible builds. Warn the author if anything strays from that.
- **Uniform design.** One mechanism for each thing, with no special cases (specification section 1.6).
- **Not in BIL.** Registration, orthorectification and analysis are layers above BIL.
- **No reference to any earlier system by name.** The design is influenced by knowledge of previous requirements, and says no more than that.

## Rules for numeric code, from experiment 1

The specification requires agreement between Windows, instruction sets and machines to a tolerance: one unit in the last place for arithmetic, and each transcendental function's stated bound (section 5). Bitwise agreement is not required; the author ruled it is not a concern. These rules gave bitwise agreement in experiment 1 at little cost, so keep them where they cost nothing, and relax them where speed needs it, staying within the tolerance.

1. Only correctly rounded IEEE operations (add, sub, mul, div, sqrt) and exact ones (min, max, compare, select, floor).
2. No fused multiply-add.
3. Transcendental functions are BIL's own, built from rule 1's operations (see `kernels.rs`), never a library's.
4. The last values of a row go through the same vector code on a padded copy.
5. Vector methods carry their `target_feature`, and closures inside kernels are `#[inline(always)]`. Otherwise the compiler calls each instruction as a function, and the vector paths run slower than scalar. Check with `objdump` for calls to `_mm256_` or `_mm512_` symbols.

## State

- **Specification:** version 60. Pass 2 red team review: all 32 findings resolved (R29 in version 60). Version 58 closed the interface gaps; version 59 applied the red team rulings.
- **Documents in step with version 60:** the specification, the pass 2 review (review version 5) and BIL by Example (version 4, unaffected by version 60). The implementability assessment still lists U8 and U9 as blocking; both are now settled.
- **Experiment 1:** done. Correctness passed bitwise on scalar, AVX2 and AVX-512 here, and its check passes in CI on Linux x86, Linux ARM, Windows and macOS on ARM. The arithmetic runs at memory speed; the whole pass does not, because the prototype makes whole planes in fresh memory.
- **CI:** green on all four systems, and on the latest stable Rust.
- **Open decisions in the specification:** 9 (section 16).

## The workspace

- Linux on x86 with AVX2 and AVX-512, two cores, Rust 1.97. macOS, Windows and ARM only through CI.
- Package registries and GitHub are reachable. Most other sites are not, so real test imagery comes from the author. `static.rust-lang.org` is not reachable, so no other Rust toolchain can be installed here.
- **The toolchain is pinned** to 1.97.0 in `rust-toolchain.toml`. In a fresh workspace rustup tries to download it by that name and fails. The installed stable is the same 1.97.0, so expose it under the pinned name:
  `ln -s stable-x86_64-unknown-linux-gnu ~/.rustup/toolchains/1.97.0-x86_64-unknown-linux-gnu`
- **The working clone** is `/home/claude/blazingimaginglibrary`, the path the session expects.

## CI

- `.github/workflows/ci.yml` runs format, lint (warnings are errors), build and experiment 1's `--check` on `ubuntu-latest`, `ubuntu-24.04-arm`, `windows-latest` and `macos-latest`.
- A separate job lints on the latest stable Rust and may fail without failing the build. It reports lints a newer compiler adds, so the pin can be raised deliberately.
- Job logs need authentication this workspace does not have. Lint messages are raised as annotations, which can be read without it:
  `curl -s https://api.github.com/repos/code-moth/BlazingImagingLibrary/check-runs/JOB_ID/annotations`
- Code compiled only on some processors must be linted there too: two warnings appeared only on ARM, from `cfg(target_arch)` code.

## Next step

**Experiment 1b: the rolling buffer.** The same arithmetic as experiment 1, with rows streamed through every Level on a buffer of a few rows, and output written into the caller's buffers. It passes when the bitwise tests still pass unchanged and the whole pass is within a small factor of a copy into warm memory. Then add missing data and NaN skipping, with their own bitwise tests.

## After that

The remaining experiments of the implementability assessment, section 8: JPEG 2000 on real NITF files (needs the author's files), containing C library errors, packaging beside rasterio and pyproj (needs CI), and borrowed NumPy arrays.
