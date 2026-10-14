// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.
//! With the `openjpeg` feature, links the OpenJPEG library the system or conda provides, and
//! where its header can be found, compiles the check that BIL's declarations of its structures
//! match the header (`src/layout.c`).
//!
//! The library is found in `BIL_OPENJPEG_LIB_DIR` where that is set, else in the usual places.
//! A system with only the runtime has `libopenjp2.so.7` but no plain name to link by, so the
//! versioned file is named where the plain one is missing. The header is found in
//! `BIL_OPENJPEG_INCLUDE`, else in `include/openjpeg-2.5` under the usual prefixes and under
//! `CONDA_PREFIX`. With `BIL_REQUIRE_LAYOUT_CHECK` set, as CI sets it, a header found but a check
//! that cannot be compiled fails the build. These are settings of the build only; nothing reads
//! them at run time.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(bil_sys_opj_layout)");
    println!("cargo:rerun-if-env-changed=BIL_OPENJPEG_LIB_DIR");
    println!("cargo:rerun-if-env-changed=BIL_OPENJPEG_INCLUDE");
    println!("cargo:rerun-if-env-changed=CONDA_PREFIX");
    println!("cargo:rerun-if-env-changed=CC");
    println!("cargo:rerun-if-env-changed=BIL_REQUIRE_LAYOUT_CHECK");
    println!("cargo:rerun-if-changed=src/layout.c");
    if env::var_os("CARGO_FEATURE_OPENJPEG").is_none() {
        return;
    }
    let conda = env::var_os("CONDA_PREFIX").map(PathBuf::from);
    let prefixes: Vec<PathBuf> = conda
        .iter()
        .cloned()
        .chain(["/usr", "/usr/local", "/opt/homebrew"].map(PathBuf::from))
        .collect();

    // The library.
    let mut lib_dirs: Vec<PathBuf> = env::var_os("BIL_OPENJPEG_LIB_DIR")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    for p in &prefixes {
        for sub in [
            "lib/x86_64-linux-gnu",
            "lib/aarch64-linux-gnu",
            "lib",
            "lib64",
            // conda on Windows
            "Library/lib",
        ] {
            lib_dirs.push(p.join(sub));
        }
    }
    let has = |f: &str| lib_dirs.iter().find(|d| d.join(f).exists()).cloned();
    let plain = ["libopenjp2.so", "libopenjp2.dylib", "openjp2.lib"];
    if let Some(dir) = plain.iter().find_map(|f| has(f)) {
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=dylib=openjp2");
    } else if let Some(dir) = has("libopenjp2.so.7") {
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=dylib:+verbatim=libopenjp2.so.7");
    } else {
        // Left to the linker's own search, which says what is missing if it fails.
        println!("cargo:rustc-link-lib=dylib=openjp2");
    }

    // The layout check, where a header and a C compiler are at hand.
    let include = env::var_os("BIL_OPENJPEG_INCLUDE")
        .map(PathBuf::from)
        .or_else(|| {
            prefixes
                .iter()
                .map(|p| p.join("include/openjpeg-2.5"))
                .find(|d| d.join("openjpeg.h").exists())
        });
    let Some(include) = include.filter(|d| d.join("openjpeg.h").exists()) else {
        println!("cargo:warning=openjpeg.h not found: the layout of OpenJPEG's structures is not checked");
        return;
    };
    // The check is compiled with the host's C compiler and linked into the tests, so it is made
    // only where the host is the target, and only on Unix; the structures are the same elsewhere.
    if env::var("TARGET").ok() != env::var("HOST").ok() || env::var_os("CARGO_CFG_UNIX").is_none() {
        println!("cargo:warning=the layout of OpenJPEG's structures is not checked in this build");
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cc = env::var("CC").unwrap_or_else(|_| "cc".into());
    let obj = out.join("layout.o");
    let lib = out.join("libbil_sys_layout.a");
    let ok = Command::new(&cc)
        .args([
            "-O2",
            "-fPIC",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-DOPJ_STATIC",
            "-c",
            "src/layout.c",
            "-o",
        ])
        .arg(&obj)
        .arg("-I")
        .arg(&include)
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&lib);
    if !ok
        || !Command::new("ar")
            .arg("crs")
            .arg(&lib)
            .arg(&obj)
            .status()
            .is_ok_and(|s| s.success())
    {
        // Where CI asks for the check, one that cannot be built fails the build; elsewhere a
        // missing compiler only leaves the check out.
        let what = format!(
            "the OpenJPEG layout check could not be compiled against {}",
            include.display()
        );
        if env::var_os("BIL_REQUIRE_LAYOUT_CHECK").is_some() {
            panic!("{what}");
        }
        println!("cargo:warning={what}: the layout of OpenJPEG's structures is not checked");
        return;
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=bil_sys_layout");
    println!("cargo:rustc-cfg=bil_sys_opj_layout");
}
