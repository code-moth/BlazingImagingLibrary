# Working notes for BIL

Read this first in every session. It records the state of the work, how it is done, and the next step. Update it at the end of every session.

## The arrangement

- **Claude writes BIL.** The specification's author leads, rules on every design decision, reviews each change, and supplies test data and other systems.
- **The specification is the source of truth.** `docs/specification/specification.md`, version 67. A design change is made there first, by the author's ruling, and the version number goes up.
- **Ask, don't assume, on design.** Where the specification leaves something open, propose and flag it. Do not decide it silently.

## Conventions

- **Naming.** Types and enum values are UpperCamelCase. Every other name is snake_case, including names passed as text: `color.hsv.hue`, `level3`, `stored2`, `bilinear`, `rational_polynomial`. C uses the prefixes `bil_`, `Bil` and `BIL_`.
- **American spelling** in identifiers, comments and documents.
- **License.** LGPL 2.1 or later, with a static-linking exception (`LINKING-EXCEPTION.md`). The author closed the license review; none is planned. BIL's own packages contain only LGPL-compatible builds. Warn the author if anything strays from that. Everything in a shipped build must be compatible with LGPL 2.1 itself, so no package becomes LGPL 3 by accident; LGPL 3 code such as NITRO, SIX, libheif or hidefix is allowed only in test-only code or as an optional plugin packaged separately. `deny.toml` checks what ships, `deny-tests.toml` everything else. Apache-2.0 alone is allowed in what ships, by the author's ruling, accepting the risk that the Free Software Foundation holds it incompatible with LGPL 2.1.
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

- **Specification:** version 67. Pass 2 red team review: all 32 findings resolved (R29 in version 60). Version 58 closed the interface gaps; version 59 applied the red team rulings.
- **Documents in step with version 67:** the specification, the pass 2 review (review version 5), BIL by Example (version 6) and the implementability assessment (version 4). The top-level README draws on the flyer and the specification.
- **Flyer:** `flyer.pdf` and `flyer.html` at the top level, page images in `docs/flyer/`, rendered by `docs/tools/render_flyer.py` with IBM Plex fonts cloned from github.com/IBM/plex (fonts are not in the repository). The original is the author's Design artifact; the contact line reads "Issues and discussion on GitHub" until the author gives one.
- **Experiment 1:** done. Correctness passed bitwise on scalar, AVX2 and AVX-512 here, and its check passes in CI on Linux x86, Linux ARM, Windows and macOS on ARM. The arithmetic runs at memory speed; the whole pass does not, because the prototype makes whole planes in fresh memory.
- **Experiment 1b:** done (`RESULTS-1b.md`). `src/rolling.rs`: `Pass::new` plans a read once, `Pass::run(img, isa, ReadOptions, out)` fills the caller's Slices with nothing allocated per read; a ring of three rows a Level; `halve_row` reads even and odd columns with vector shuffles. Same bits as experiment 1 on every test and every Window of 25 small Images; 3 to 5 times faster on the fastest instruction set. `pixel.raw` 1.7 times a warm copy at Level 0; `complex.polar` and `color.hsv` 1.4 to 1.7 times their own arithmetic at Level 0. **Missing data** (a validity mask on `Image`): one weight row a Level shared by every plane; `halve_weighted`, `halve_row_weighted`, `halve_over`; `missing_weights` `Equal` (default) or `PixelCount` (version 67); `missing_value` applied last; bitwise against `testing::masked_reference`, a float64 check of the rule, and `PixelCount` = pyramid(data) / pyramid(mask). 1.2 to 1.8 times an unmasked read. **Other kinds** (`Func::reduce`): `class.labels` keeps every second value, `depth.sensor.range` takes the least present value of each 3 × 3 block (`min3_present`, `min3_row`), `pixel.mask` gives `valid` and `valid_fraction` from the averaged mask; `LevelMethod::Decimate` keeps for every kind. Each checked against its definition. Proposals awaiting the author: one 2D average rather than rescaling after each pass, stored NaN counted as missing (tested: identical to masking the pixel), `valid_fraction` weighted by the filter, and `valid` following the averaged mask beside kinds that are not averaged. Its check runs in CI.
- **CI:** green on all four systems, and on the latest stable Rust, including experiment 1b's check.
- **Open decisions in the specification:** 2 (section 16): drivers for the added sensor cases, and dictionary contents. Version 61 settled test data, registry names and Tier 1's libraries. Version 62 settled interpreter packaging (CPython from the machine, none shipped). Version 63 closed the license review, put CPython for Rust, C and C++ callers in a helper process whose round trip is benchmarked and watched, and added full geocoding accuracy (section 13.9: covariance by source and kind, carried through the chain, never dropped). Version 64 settled the mapped format (section 10.2). Version 65 applied the checking agent's findings: wording of the interpreter, a folder for attaching to a decoder, the decoder's lock file in containment, an accuracy column in section 3.3, `Contribution` records, and LGPL 3 libraries marked as test references or optional plugins. Version 66 recorded the author's speed ruling in section 12: as fast as each function's arithmetic allows, measured against a copy and against that arithmetic. Version 67 added `missing_weights` (`Equal` by default, or `PixelCount`) to `SliceOptions` and section 5, by the author's ruling.
- **SIX is LGPL 3,** so under the shipped-build rule it is assessed when Tier 2 starts (section 10.6). The specification allows it only as an optional plugin or a test reference; the other route is reading SICD and SIDD without it, through a Rust or scripted reader.
- **Tier 1 readers:** GeoTIFF through the Rust `tiff` crate with BIL's own GeoTIFF keys; NITF through a native Rust reader using field definitions. GDAL and NITRO (LGPL 3.0) are test references only.
- **Packages:** `crates/bil` 0.0.1 holds the vector layer and kernels (experiment 1 now uses them from there); `crates/bil-sys` 0.0.1 holds the name; `python/` is `bil` 0.0.1 for PyPI. The author publishes with `.github/workflows/publish.yml`, which needs a `CARGO_REGISTRY_TOKEN` secret and PyPI trusted publishing for environment `pypi`.
- **Test data:** 22 real NITF files pinned from GDAL's download cache, among them 8 NITF 2.1 files and `jpeg2000/p0_01a.ntf`, the one JPEG 2000 NITF the cache holds. Four wanted NITF 2.0 files are not in the cache (listed at the end of the manifest). `testdata/manifest.toml` lists real files and their checksums; `testdata/fetch.py` downloads them. The CI job `test data` fetches them, since this workspace cannot, and reports unpinned checksums as annotations to copy into the manifest.

## The workspace

- Linux on x86 with AVX2 and AVX-512, two cores, Rust 1.97. macOS, Windows and ARM only through CI.
- **Only GitHub is reliably reachable.** In some sessions crates.io, PyPI and every other site fail to resolve, so build with `--offline` and add no dependency that is not already in `Cargo.lock`. Real test files are fetched by CI or supplied by the author. `static.rust-lang.org` is not reachable, so no other Rust toolchain can be installed here.
- **The toolchain is pinned** to 1.97.0 in `rust-toolchain.toml`. In a fresh workspace rustup tries to download it by that name and fails. The installed stable is the same 1.97.0, so expose it under the pinned name:
  `ln -s stable-x86_64-unknown-linux-gnu ~/.rustup/toolchains/1.97.0-x86_64-unknown-linux-gnu`
- **The working clone** is `/home/claude/blazingimaginglibrary`, the path the session expects.

## CI

- `.github/workflows/ci.yml` runs format, lint (warnings are errors), build, and the `--check` of experiments 1 and 1b on `ubuntu-latest`, `ubuntu-24.04-arm`, `windows-latest` and `macos-latest`. Further jobs check licenses and sources with cargo-deny (`deny.toml` for what ships, `deny-tests.toml` for everything), build the crates and the Python package, and fetch the test data.
- Every source file starts with an SPDX license line.
- A separate job lints on the latest stable Rust and may fail without failing the build. It reports lints a newer compiler adds, so the pin can be raised deliberately.
- Job logs need authentication this workspace does not have. Lint messages are raised as annotations, which can be read without it:
  `curl -s https://api.github.com/repos/code-moth/BlazingImagingLibrary/check-runs/JOB_ID/annotations`
- Code compiled only on some processors must be linted there too: two warnings appeared only on ARM, from `cfg(target_arch)` code.

## Next step

`binomial5`, then threads by bands of rows, then `decimate` reading only the pixels kept when a request asks for coarser Levels alone. Keep the inlining check after every kernel: array `map` closures inside kernels were not inlined into AVX-512 code (range ran 3 times slower) until written out. Speed rule (author, version 66): as fast as each function's arithmetic allows; measure against a copy and against the arithmetic alone.

## Checking

Before pushing, run the `bil-checker` agent (`.claude/agents/bil-checker.md`) over everything written or changed: it checks style and agreement with the specification and these notes, and reports without editing. Fix what it finds, or tell the author why not.

## After that

The remaining experiments of the implementability assessment, section 8: JPEG 2000 on real NITF files (needs the author's files), containing C library errors, packaging beside rasterio and pyproj (needs CI), and borrowed NumPy arrays.
