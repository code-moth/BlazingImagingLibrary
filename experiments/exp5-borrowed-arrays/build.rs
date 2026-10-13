// SPDX-License-Identifier: LGPL-2.1-or-later
//! Links the extension module the way each system expects.
//!
//! On Linux the interpreter's symbols are found when Python loads the module, so nothing is
//! needed. macOS needs to be told to leave them for then. Windows links against the stable
//! interface's import library, `python3.lib`, found beside the Python named by `PYTHON` or found
//! on the path.

use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PYTHON");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match os.as_str() {
        "macos" => {
            println!("cargo:rustc-cdylib-link-arg=-undefined");
            println!("cargo:rustc-cdylib-link-arg=dynamic_lookup");
        }
        "windows" => {
            let python = env::var("PYTHON").unwrap_or_else(|_| "python".into());
            let libs = Command::new(&python)
                .args([
                    "-c",
                    "import os, sys; print(os.path.join(sys.base_prefix, 'libs'))",
                ])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
            match libs {
                Some(dir) => {
                    // The library itself is named on the declarations in src/ffi.rs.
                    println!("cargo:rustc-link-search=native={dir}");
                }
                None => println!(
                    "cargo:warning=no Python found to link against; set PYTHON to its path"
                ),
            }
        }
        _ => {}
    }
}
