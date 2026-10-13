// SPDX-License-Identifier: LGPL-2.1-or-later
//! Builds the C layer (`c/shim.c`, `c/state.c`) against the headers in `vendor/`, and links the
//! libjpeg-turbo and OpenJPEG runtimes the system provides.
//!
//! The C compiler is `CC`, or `cc`. A system with only the runtimes, and no development
//! packages, has `libjpeg.so.8` and `libopenjp2.so.7` but no unversioned names to link by, so
//! the versioned file is named where the plain one is missing.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed");
}

/// How to link `lib`: by its plain name where `lib<name>.so` exists or no versioned file does,
/// else by the versioned file itself.
fn link_name(lib: &str, versioned: &str) -> String {
    let dirs = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib",
        "/usr/lib64",
    ];
    let has = |f: &str| dirs.iter().any(|d| Path::new(d).join(f).exists());
    if !has(&format!("lib{lib}.so")) && has(versioned) {
        format!("dylib:+verbatim={versioned}")
    } else {
        format!("dylib={lib}")
    }
}

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cc = env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut objects = Vec::new();
    for src in ["c/shim.c", "c/state.c"] {
        println!("cargo:rerun-if-changed={src}");
        let obj = out.join(Path::new(src).with_extension("o").file_name().unwrap());
        run(Command::new(&cc)
            .args([
                "-O2",
                "-fPIC",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-Ivendor/libjpeg-turbo-2.1.5",
                "-Ivendor/openjpeg-2.5.0",
                "-c",
                src,
                "-o",
            ])
            .arg(&obj));
        objects.push(obj);
    }
    let lib = out.join("libbilshim.a");
    let _ = std::fs::remove_file(&lib);
    run(Command::new("ar").arg("crs").arg(&lib).args(&objects));
    println!("cargo:rerun-if-changed=vendor");
    println!("cargo:rerun-if-env-changed=CC");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=bilshim");
    println!("cargo:rustc-link-lib={}", link_name("jpeg", "libjpeg.so.8"));
    println!(
        "cargo:rustc-link-lib={}",
        link_name("openjp2", "libopenjp2.so.7")
    );
}
