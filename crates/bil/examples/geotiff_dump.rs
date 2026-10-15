// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Prints what BIL's GeoTIFF reader finds in a file, one fact to a line, in the form
//! `testdata/geotiff_reference.c` prints what GDAL finds, so that `testdata/compare_geotiff.py`
//! can compare them.
//!
//!     cargo run --release --example geotiff_dump -- FILE.tif

use bil::geotiff::{GeoTiffSource, Scope};
use bil::types::Window;

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: geotiff_dump FILE");
    let file = std::fs::File::open(&path).expect("cannot open the file");
    let source = match GeoTiffSource::open(file) {
        Ok(s) => s,
        Err(e) => {
            println!("error {e}");
            return;
        }
    };
    println!("images {}", source.image_count());
    for i in 0..source.image_count() {
        let info = source.image_info(i).unwrap();
        let kind = info
            .pixel_type
            .map_or("unsupported".to_string(), |p| format!("{:?}", p.sample_type));
        println!("image {i} size {} x {} x {} {kind}", info.rows, info.cols, info.bands);
        for level in &info.stored_levels {
            println!("image {i} overview {} x {}", level.rows, level.cols);
        }
        // Shown only where files differ.
        println!(
            "image {i} layout compression {} planar {} tiled {} block {} x {} nodata {:?} groups {:?}",
            info.compression,
            info.planar,
            info.tiled,
            info.block_size.0,
            info.block_size.1,
            info.nodata,
            source.list_field_groups(Scope::Image(i)).unwrap_or_default(),
        );
        let pixels = if info.pixel_type.is_none() {
            format!("refused {}", info.unsupported.clone().unwrap_or_default())
        } else if !info.decodable {
            format!("refused {} is not decoded", info.codec.as_deref().unwrap_or("?"))
        } else if info.rows > u64::from(u32::MAX) || info.cols > u64::from(u32::MAX) {
            "refused too large to read whole".to_string()
        } else {
            let window = Window {
                row_start: 0,
                col_start: 0,
                row_count: info.rows as u32,
                col_count: info.cols as u32,
            };
            match source.read_fragment(i, window) {
                Ok(f) => {
                    let size = f.pixel_type.sample_type.size();
                    let mut planes = Vec::with_capacity(f.data.len());
                    for b in 0..info.bands as usize {
                        for p in 0..(info.rows * info.cols) as usize {
                            let at = p * f.strides[1] + b * size;
                            planes.extend_from_slice(&f.data[at..at + size]);
                        }
                    }
                    if let Some(v) = &f.validity {
                        println!("image {i} missing {}", v.iter().filter(|&&x| !x).count());
                    }
                    format!("{:016x}", fnv(&planes))
                }
                Err(e) => format!("error {e}"),
            }
        };
        println!("image {i} pixels {pixels}");
    }
}
