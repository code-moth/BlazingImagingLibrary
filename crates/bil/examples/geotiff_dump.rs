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

/// A half-precision float's value, exactly.
fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((h >> 10) & 0x1F);
    let fraction = f32::from(h & 0x3FF);
    match exponent {
        0 => sign * fraction * 2f32.powi(-24),
        31 if fraction == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        e => sign * (1.0 + fraction / 1024.0) * 2f32.powi(e - 15),
    }
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
        let kind = info.pixel_type.map_or("unsupported".to_string(), |p| {
            format!("{:?}", p.sample_type)
        });
        println!(
            "image {i} size {} x {} x {} {kind}",
            info.rows, info.cols, info.bands
        );
        for level in &info.stored_levels {
            println!("image {i} overview {} x {}", level.rows, level.cols);
        }
        // How the samples are to be seen, which GDAL converts for some interpretations.
        let photometric = source
            .read_field(Scope::Image(i), "tiff.ifd.PhotometricInterpretation")
            .ok()
            .and_then(|f| match f.value() {
                bil::fields::Value::Integer(v) => Some(*v),
                _ => None,
            });
        println!("image {i} photometric {photometric:?}");
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
            format!(
                "refused {} is not decoded",
                info.codec.as_deref().unwrap_or("?")
            )
        } else if info
            .rows
            .saturating_mul(info.cols)
            .saturating_mul(u64::from(info.bands) * 8)
            > 1 << 28
        {
            // Read whole, as GDAL's program does, only where that fits easily in memory.
            "refused too large to read whole here".to_string()
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
                    // GDAL 3.8 reads float16 as float32; the same values, widened, for comparison.
                    if f.pixel_type.sample_type == bil::types::SampleType::F16 {
                        let wide: Vec<u8> = planes
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .flat_map(|h| half_to_f32(u16::from_ne_bytes(*h)).to_ne_bytes())
                            .collect();
                        println!("image {i} widened F32 {:016x}", fnv(&wide));
                    }
                    if let Some(v) = &f.validity {
                        println!("image {i} missing {}", v.iter().filter(|&&x| !x).count());
                    }
                    format!("{:016x}", fnv(&planes))
                }
                // What this reader does not read is a refusal with its reason; anything else is
                // an error.
                Err(e @ bil::geotiff::Error::Unsupported(_)) => format!("refused {e}"),
                Err(e) => format!("error {e}"),
            }
        };
        println!("image {i} pixels {pixels}");
    }
}
