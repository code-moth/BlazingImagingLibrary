// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Prints what BIL's NITF reader finds in a file, one fact to a line, in the form
//! `testdata/nitf_reference.c` prints what GDAL finds, so that the two can be compared line for
//! line by `testdata/compare_nitf.py`.
//!
//!     cargo run --release --example nitf_dump -- FILE.ntf

use bil::fields::Field;
use bil::nitf::{Error, NitfSource, Scope, Version};
use bil::types::Window;

/// The header fields compared: those GDAL reports, as `NITF_` and the name. Each is printed
/// under GDAL's name, which for NITF 2.0's IID is IID1.
const FILE_FIELDS: &[&str] = &["OSTAID", "FTITLE", "FSCLAS", "ONAME"];
const IMAGE_FIELDS: &[(&str, &str)] = &[
    ("IID1", "IID1"),
    ("IID", "IID1"),
    ("ISORCE", "ISORCE"),
    ("IREP", "IREP"),
    ("ICAT", "ICAT"),
    ("ABPP", "ABPP"),
    ("PJUST", "PJUST"),
    ("ICORDS", "ICORDS"),
    ("IGEOLO", "IGEOLO"),
];

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// A field's stored text without its padding on the right, as GDAL gives it. What it decodes
/// to is checked by the reader's own tests; this compares where each field's bytes are.
fn shown(f: &Field) -> String {
    let text: String = f.stored.iter().map(|&b| char::from(b)).collect();
    text.trim_end_matches(' ').to_string()
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: nitf_dump FILE");
    let file = std::fs::File::open(&path).expect("cannot open the file");
    let source = match NitfSource::open(file) {
        Ok(s) => s,
        Err(e) => {
            println!("error {e}");
            return;
        }
    };
    let version = match source.version() {
        Version::Nitf20 => "NITF02.00",
        Version::Nitf21 => "NITF02.10",
        Version::Nsif10 => "NSIF01.00",
    };
    println!("version {version}");
    println!("images {}", source.image_count());
    for name in FILE_FIELDS {
        if let Ok(f) = source.read_field(Scope::Source, &format!("nitf.file_header.{name}")) {
            println!("file field {name}={}", shown(&f));
        }
    }
    let groups = source.list_field_groups(Scope::Source).unwrap();
    for g in groups.iter().skip(1) {
        println!("file extension {}", g.trim_start_matches("nitf."));
    }
    for i in 0..source.image_count() {
        let info = source.image_info(i).unwrap();
        let kind = match info.pixel_type {
            Some(p) => format!("{:?}", p.sample_type),
            None => "unsupported".into(),
        };
        println!(
            "image {i} size {} x {} x {} {kind}",
            info.rows, info.cols, info.bands
        );
        // How the pixels are stored, which GDAL does not report: shown only where files differ.
        println!(
            "image {i} layout IC {} IMODE {} NBPP {} blocks {} x {} of {} x {} significant {:?} shift {:?}",
            info.compression,
            info.imode,
            info.stored_bits,
            info.blocks_per_col,
            info.blocks_per_row,
            info.block_size.0,
            info.block_size.1,
            info.pixel_type.map(|p| p.significant_bits),
            info.pixel_type.map(|p| p.bit_shift),
        );
        for (name, label) in IMAGE_FIELDS {
            let field = format!("nitf.image_subheader.{name}");
            // A field the file leaves out is Missing with nothing stored; GDAL omits it.
            if let Ok(f) = source.read_field(Scope::Image(i), &field) {
                if !f.stored.is_empty() {
                    println!("image {i} field {label}={}", shown(&f));
                }
            }
        }
        for g in source
            .list_field_groups(Scope::Image(i))
            .unwrap()
            .iter()
            .skip(1)
        {
            println!("image {i} extension {}", g.trim_start_matches("nitf."));
        }
        // The codecs this build has; GDAL's reference program decodes the same.
        let decodable = match info.codec.as_deref() {
            None => true,
            Some(c) => c == "jpeg2000" && cfg!(feature = "openjpeg"),
        };
        let pixels = if !decodable {
            "compressed".to_string()
        } else if info.pixel_type.is_none() {
            "unsupported".to_string()
        } else if info.rows > u64::from(u32::MAX) || info.cols > u64::from(u32::MAX) {
            "too large to read whole".to_string()
        } else {
            let window = Window {
                row_start: 0,
                col_start: 0,
                row_count: info.rows as u32,
                col_count: info.cols as u32,
            };
            match source.read_fragment(i, window) {
                Ok(f) => {
                    // Band after band, as GDAL reads them one band at a time.
                    let size = f.pixel_type.sample_type.size();
                    let mut planes = Vec::with_capacity(f.data.len());
                    for b in 0..info.bands as usize {
                        for p in 0..(info.rows * info.cols) as usize {
                            let at = p * f.strides[1] + b * size;
                            planes.extend_from_slice(&f.data[at..at + size]);
                        }
                    }
                    // For a closer look at a difference: the pixels, band after band, as
                    // nitf_reference.c writes GDAL's.
                    if let Some(dir) = std::env::var_os("NITF_PIXELS_DIR") {
                        let name = std::path::Path::new(&path).file_name().unwrap();
                        let file = format!("{}.{i}.bil", name.to_string_lossy());
                        std::fs::write(std::path::Path::new(&dir).join(file), &planes).unwrap();
                        // One byte for each pixel: 1 where it was recorded, 0 where not.
                        if let Some(v) = &f.validity {
                            let mask: Vec<u8> = v.iter().map(|&x| u8::from(x)).collect();
                            let file = format!("{}.{i}.mask", name.to_string_lossy());
                            std::fs::write(std::path::Path::new(&dir).join(file), mask).unwrap();
                        }
                    }
                    let missing = f
                        .validity
                        .as_ref()
                        .map_or(0, |v| v.iter().filter(|&&x| !x).count());
                    if missing > 0 {
                        println!("image {i} missing {missing}");
                    }
                    format!("{:016x}", fnv(&planes))
                }
                // A compressed Image of a kind the codec does not yet read.
                Err(Error::Unsupported(m)) if info.codec.is_some() => format!("refused {m}"),
                Err(e) => format!("error {e}"),
            }
        };
        println!("image {i} pixels {pixels}");
    }
}
