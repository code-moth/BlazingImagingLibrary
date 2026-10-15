// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The GeoTIFF reader against synthetic files whose contents are known by construction
//! (specification, section 16, "Test data"). A small writer here builds them from TIFF 6.0,
//! TIFF Technical Note 3 (the floating-point predictor), BigTIFF and GeoTIFF 1.0,
//! independently of the reader and of the `tiff` crate it uses.

#![cfg(feature = "geotiff")]

use std::io::Write;

use bil::fields::Value;
use bil::geotiff::{Error, GeoTiffSource, Scope};
use bil::types::{Interleave, SampleType, StorageOrder, Window};

// ---------------------------------------------------------------- values known by construction

/// The value of band `b` at row `r`, column `c`, as raw bits of `bits`.
fn bits_at(r: u64, c: u64, b: u64, bits: u32) -> u64 {
    let v = (r.wrapping_mul(0x9E37_79B9)
        ^ c.wrapping_mul(0x85EB_CA6B)
        ^ (b + 1).wrapping_mul(0xC2B2_AE35))
    .wrapping_mul(0x2545_F491_4F6C_DD1D)
        >> 3;
    if bits == 64 {
        v
    } else {
        v & ((1u64 << bits) - 1)
    }
}

/// The sample, as its bits: floats are made finite, from a formula of their own.
fn sample(t: SampleType, r: u64, c: u64, b: u64) -> u64 {
    match t {
        SampleType::F32 => {
            u64::from(((r as f32) * 1.5 - (c as f32) * 0.25 + b as f32 * 100.0).to_bits())
        }
        SampleType::F64 => ((r as f64) * 1.5 - (c as f64) * 0.25 + b as f64 * 100.0).to_bits(),
        t => bits_at(r, c, b, t.size() as u32 * 8),
    }
}

fn format_of(t: SampleType) -> u16 {
    match t {
        SampleType::I8 | SampleType::I16 | SampleType::I32 | SampleType::I64 => 2,
        SampleType::F32 | SampleType::F64 => 3,
        _ => 1,
    }
}

// ---------------------------------------------------------------- a writer for test files

#[derive(Clone)]
struct Img {
    rows: u64,
    cols: u64,
    bands: u64,
    t: SampleType,
    /// Rows and columns of a tile, or rows of a strip.
    tile: Option<(u64, u64)>,
    strip_rows: u64,
    planar: bool,
    /// 1 none, 8 Deflate, or another code whose data is written raw.
    compression: u16,
    predictor: u16,
    subfile: u32,
    /// Extra tags: code, TIFF type, count, values as little-endian bytes of that type.
    tags: Vec<(u16, u16, u64, Vec<u8>)>,
    /// Chunks never written, as in a sparse file: offset and byte count 0.
    sparse: Vec<usize>,
}

impl Img {
    fn new(rows: u64, cols: u64, bands: u64, t: SampleType) -> Img {
        Img {
            rows,
            cols,
            bands,
            t,
            tile: None,
            strip_rows: 3,
            planar: false,
            compression: 1,
            predictor: 1,
            subfile: 0,
            tags: Vec::new(),
            sparse: Vec::new(),
        }
    }

    fn chunk(&self) -> (u64, u64) {
        self.tile.unwrap_or((self.strip_rows, self.cols))
    }

    /// The chunks' bytes, in TIFF's order: by plane, then row of chunks, then column.
    fn chunks(&self, big: bool) -> Vec<Vec<u8>> {
        let (ch, cw) = self.chunk();
        let size = self.t.size();
        let planes = if self.planar { self.bands } else { 1 };
        let per = if self.planar { 1 } else { self.bands };
        let mut out = Vec::new();
        for plane in 0..planes {
            for cr in 0..self.rows.div_ceil(ch) {
                for cc in 0..self.cols.div_ceil(cw) {
                    // Strips end at the Image; tiles are padded to their size.
                    let h = if self.tile.is_some() {
                        ch
                    } else {
                        ch.min(self.rows - cr * ch)
                    };
                    let mut raw = Vec::new();
                    for r in 0..h {
                        let mut row: Vec<u64> = Vec::new();
                        for c in 0..cw {
                            for k in 0..per {
                                let (rr, cc2, b) = (
                                    cr * ch + r,
                                    cc * cw + c,
                                    if self.planar { plane } else { k },
                                );
                                row.push(if rr < self.rows && cc2 < self.cols {
                                    sample(self.t, rr, cc2, b)
                                } else {
                                    0
                                });
                            }
                        }
                        raw.extend(self.predict(&row, per as usize, big));
                    }
                    out.push(match self.compression {
                        8 => {
                            let mut e = flate2::write::ZlibEncoder::new(
                                Vec::new(),
                                flate2::Compression::default(),
                            );
                            e.write_all(&raw).unwrap();
                            e.finish().unwrap()
                        }
                        _ => raw,
                    });
                }
            }
        }
        let _ = size;
        out
    }

    /// One row's bytes in the file's order, after the predictor.
    fn predict(&self, row: &[u64], per: usize, big: bool) -> Vec<u8> {
        let size = self.t.size();
        let bytes = |v: u64| -> Vec<u8> {
            let le = &v.to_le_bytes()[..size];
            if big {
                le.iter().rev().copied().collect()
            } else {
                le.to_vec()
            }
        };
        match self.predictor {
            2 => {
                let mask = if size == 8 {
                    u64::MAX
                } else {
                    (1u64 << (size * 8)) - 1
                };
                row.iter()
                    .enumerate()
                    .flat_map(|(i, &v)| {
                        let prev = if i >= per { row[i - per] } else { 0 };
                        bytes(v.wrapping_sub(prev) & mask)
                    })
                    .collect()
            }
            3 => {
                // TN3: each value's bytes most significant first, regrouped by byte place,
                // then each byte less the one before it.
                let n = row.len();
                let mut planes = vec![0u8; n * size];
                for (i, &v) in row.iter().enumerate() {
                    let be = &v.to_be_bytes()[8 - size..];
                    for (k, &b) in be.iter().enumerate() {
                        planes[k * n + i] = b;
                    }
                }
                let mut out = planes.clone();
                for i in (per..planes.len()).rev() {
                    out[i] = planes[i].wrapping_sub(planes[i - per]);
                }
                out
            }
            _ => row.iter().flat_map(|&v| bytes(v)).collect(),
        }
    }
}

/// Writes a TIFF file holding these IFDs in this order; BigTIFF where `bigtiff`.
fn write(images: &[Img], big: bool, bigtiff: bool) -> Vec<u8> {
    let u16b = |v: u16| {
        if big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let u32b = |v: u32| {
        if big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let u64b = |v: u64| {
        if big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let mut f: Vec<u8> = Vec::new();
    f.extend(if big { b"MM" } else { b"II" });
    if bigtiff {
        f.extend(u16b(43));
        f.extend(u16b(8));
        f.extend(u16b(0));
        f.extend(u64b(0)); // first IFD, patched below
    } else {
        f.extend(u16b(42));
        f.extend(u32b(0));
    }
    let mut next_field = if bigtiff { 8 } else { 4 };
    for img in images {
        let chunks = img.chunks(big);
        let mut offsets = Vec::new();
        let mut counts = Vec::new();
        for (k, c) in chunks.iter().enumerate() {
            if img.sparse.contains(&k) {
                offsets.push(0);
                counts.push(0);
            } else {
                offsets.push(f.len() as u64);
                counts.push(c.len() as u64);
                f.extend(c);
            }
        }
        // Entries: (code, type, count, little-endian value bytes).
        let short = |v: &[u16]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
        let long = |v: &[u64]| -> (u16, Vec<u8>) {
            if bigtiff {
                (16, v.iter().flat_map(|x| x.to_le_bytes()).collect())
            } else {
                (
                    4,
                    v.iter().flat_map(|&x| (x as u32).to_le_bytes()).collect(),
                )
            }
        };
        let mut e: Vec<(u16, u16, u64, Vec<u8>)> = vec![
            (254, 4, 1, img.subfile.to_le_bytes().to_vec()),
            (256, 4, 1, (img.cols as u32).to_le_bytes().to_vec()),
            (257, 4, 1, (img.rows as u32).to_le_bytes().to_vec()),
            (
                258,
                3,
                img.bands,
                short(&vec![img.t.size() as u16 * 8; img.bands as usize]),
            ),
            (259, 3, 1, short(&[img.compression])),
            (262, 3, 1, short(&[if img.bands == 3 { 2 } else { 1 }])),
            (277, 3, 1, short(&[img.bands as u16])),
            (284, 3, 1, short(&[if img.planar { 2 } else { 1 }])),
            (
                339,
                3,
                img.bands,
                short(&vec![format_of(img.t); img.bands as usize]),
            ),
        ];
        if img.predictor != 1 {
            e.push((317, 3, 1, short(&[img.predictor])));
        }
        let (otype, obytes) = long(&offsets);
        let (ctype, cbytes) = long(&counts);
        match img.tile {
            Some((h, w)) => {
                e.push((322, 4, 1, (w as u32).to_le_bytes().to_vec()));
                e.push((323, 4, 1, (h as u32).to_le_bytes().to_vec()));
                e.push((324, otype, offsets.len() as u64, obytes));
                e.push((325, ctype, counts.len() as u64, cbytes));
            }
            None => {
                e.push((273, otype, offsets.len() as u64, obytes));
                e.push((278, 4, 1, (img.strip_rows as u32).to_le_bytes().to_vec()));
                e.push((279, ctype, counts.len() as u64, cbytes));
            }
        }
        e.extend(img.tags.iter().cloned());
        e.sort_by_key(|x| x.0);
        // The IFD, with values too long for an entry placed after it.
        let at = f.len() as u64;
        if bigtiff {
            f[next_field..next_field + 8].copy_from_slice(&u64b(at));
        } else {
            f[next_field..next_field + 4].copy_from_slice(&u32b(at as u32));
        }
        let (entry, inline, count_bytes) = if bigtiff { (20, 8, 8) } else { (12, 4, 2) };
        let ifd_len = count_bytes + e.len() * entry + inline;
        let mut extra: Vec<u8> = Vec::new();
        let extra_at = at + ifd_len as u64;
        let mut ifd: Vec<u8> = Vec::new();
        if bigtiff {
            ifd.extend(u64b(e.len() as u64));
        } else {
            ifd.extend(u16b(e.len() as u16));
        }
        for (code, typ, count, le) in &e {
            ifd.extend(u16b(*code));
            ifd.extend(u16b(*typ));
            if bigtiff {
                ifd.extend(u64b(*count));
            } else {
                ifd.extend(u32b(*count as u32));
            }
            // Each value in the file's byte order.
            let width = match typ {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                _ => 8,
            };
            let mut v: Vec<u8> = Vec::new();
            for chunk in le.chunks(width) {
                if big {
                    v.extend(chunk.iter().rev())
                } else {
                    v.extend(chunk)
                }
            }
            if v.len() <= inline {
                v.resize(inline, 0);
                ifd.extend(v);
            } else {
                let off = extra_at + extra.len() as u64;
                if bigtiff {
                    ifd.extend(u64b(off))
                } else {
                    ifd.extend(u32b(off as u32))
                }
                extra.extend(v);
                if extra.len() % 2 == 1 {
                    extra.push(0);
                }
            }
        }
        next_field = (at as usize) + ifd.len();
        ifd.extend(vec![0u8; inline]);
        f.extend(ifd);
        f.extend(extra);
    }
    f
}

fn whole(img: &Img) -> Window {
    Window {
        row_start: 0,
        col_start: 0,
        row_count: img.rows as u32,
        col_count: img.cols as u32,
    }
}

fn check(source: &GeoTiffSource<Vec<u8>>, image: usize, img: &Img, w: Window) {
    let f = source.read_fragment(image, w).unwrap();
    let size = img.t.size();
    for r in 0..u64::from(w.row_count) {
        for c in 0..u64::from(w.col_count) {
            for b in 0..img.bands {
                let at = r as usize * f.strides[0]
                    + c as usize * f.strides[1]
                    + b as usize * f.strides[2];
                let want = sample(img.t, w.row_start + r, w.col_start + c, b).to_ne_bytes();
                let want = if cfg!(target_endian = "little") {
                    &want[..size]
                } else {
                    &want[8 - size..]
                };
                assert_eq!(
                    &f.data[at..at + size],
                    want,
                    "{:?} row {} col {} band {b}",
                    img.t,
                    w.row_start + r,
                    w.col_start + c
                );
            }
        }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn random_windows(source: &GeoTiffSource<Vec<u8>>, img: &Img, rng: &mut Rng, n: usize) {
    check(source, 0, img, whole(img));
    for _ in 0..n {
        let r = rng.below(img.rows);
        let c = rng.below(img.cols);
        let w = Window {
            row_start: r,
            col_start: c,
            row_count: 1 + rng.below(img.rows - r) as u32,
            col_count: 1 + rng.below(img.cols - c) as u32,
        };
        check(source, 0, img, w);
    }
}

// ---------------------------------------------------------------- the tests

#[test]
fn every_layout_reads_exactly() {
    let types = [
        SampleType::U8,
        SampleType::I8,
        SampleType::U16,
        SampleType::I16,
        SampleType::U32,
        SampleType::I32,
        SampleType::U64,
        SampleType::I64,
        SampleType::F32,
        SampleType::F64,
    ];
    let mut rng = Rng(11);
    let mut cases = 0;
    for t in types {
        for (tile, planar) in [
            (None, false),
            (None, true),
            (Some((8, 16)), false),
            (Some((16, 16)), true),
        ] {
            for (compression, predictor) in
                [(1, 1), (8, 1), (8, if format_of(t) == 3 { 3 } else { 2 })]
            {
                for (big, bigtiff) in [(false, false), (true, false), (false, true)] {
                    // 21 × 37: partial strips and tiles on the right and at the bottom.
                    let mut img = Img::new(21, 37, 3, t);
                    img.tile = tile;
                    img.planar = planar;
                    img.compression = compression;
                    img.predictor = predictor;
                    let source = GeoTiffSource::open(write(&[img.clone()], big, bigtiff))
                        .unwrap_or_else(|e| panic!("{t:?} {tile:?} {planar} {compression} {predictor} {big} {bigtiff}: {e}"));
                    let info = source.image_info(0).unwrap();
                    assert_eq!((info.rows, info.cols, info.bands), (21, 37, 3));
                    assert_eq!(info.pixel_type.unwrap().sample_type, t);
                    random_windows(&source, &img, &mut rng, 6);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 10 * 4 * 3 * 3);
}

#[test]
fn overviews_are_stored_levels_and_masks_are_not_images() {
    let full = Img::new(40, 60, 1, SampleType::U8);
    let mut o1 = Img::new(20, 30, 1, SampleType::U8);
    o1.subfile = 1;
    let mut o2 = Img::new(10, 15, 1, SampleType::U8);
    o2.subfile = 1;
    let mut mask = Img::new(40, 60, 1, SampleType::U8);
    mask.subfile = 4;
    let second = Img::new(5, 7, 2, SampleType::U16);
    // Overviews out of order, a mask between them, and a second Image after.
    let source =
        GeoTiffSource::open(write(&[full, o2, mask, o1, second.clone()], false, false)).unwrap();
    assert_eq!(source.image_count(), 2);
    let levels = &source.image_info(0).unwrap().stored_levels;
    let named: Vec<(&str, u64, u64, (f64, f64))> = levels
        .iter()
        .map(|l| (l.name.as_str(), l.rows, l.cols, l.scale))
        .collect();
    assert_eq!(
        named,
        [
            ("stored1", 20, 30, (2.0, 2.0)),
            ("stored2", 10, 15, (4.0, 4.0))
        ]
    );
    let info = source.image_info(1).unwrap();
    assert_eq!((info.rows, info.cols, info.bands, info.ifd), (5, 7, 2, 4));
    let f = source.read_fragment(1, whole(&second)).unwrap();
    assert_eq!(f.data.len(), 5 * 7 * 2 * 2);
    check(&source, 1, &second, whole(&second));
}

#[test]
fn storage_lists_chunks_in_file_order() {
    let mut img = Img::new(20, 30, 2, SampleType::U16);
    img.tile = Some((16, 16));
    img.planar = true;
    let source = GeoTiffSource::open(write(&[img], false, false)).unwrap();
    let s = source.describe_storage(0).unwrap();
    assert_eq!(
        (s.block_size, s.interleave, s.order),
        ((16, 16), Interleave::Band, StorageOrder::Known)
    );
    assert_eq!(s.blocks.len(), 8);
    assert!(s
        .blocks
        .windows(2)
        .all(|w| w[0].offset + w[0].length == w[1].offset));
    assert_eq!(s.blocks[0].components, 0..1);
    assert_eq!(s.blocks[4].components, 1..2);
    assert_eq!((s.blocks[3].block_row, s.blocks[3].block_col), (1, 1));
    assert!(s.blocks.iter().all(|b| b.length == 16 * 16 * 2));
}

#[test]
fn nodata_marks_pixels_whose_every_sample_holds_it() {
    for (t, text) in [
        (SampleType::U8, "0"),
        (SampleType::I16, "-5.0"),
        (SampleType::F32, "100.5"),
        (SampleType::F32, "nan"),
    ] {
        let mut img = Img::new(6, 6, 2, t);
        let mut v = text.as_bytes().to_vec();
        v.push(0);
        img.tags.push((42113, 2, v.len() as u64, v));
        let source = GeoTiffSource::open(write(&[img.clone()], false, false)).unwrap();
        assert_eq!(source.image_info(0).unwrap().nodata.as_deref(), Some(text));
        let f = source.read_fragment(0, whole(&img)).unwrap();
        let size = t.size();
        let expect: Vec<bool> = f
            .data
            .chunks_exact(f.strides[1])
            .map(|p| {
                !p.chunks_exact(size).all(|s| match t {
                    SampleType::U8 => s[0] == 0,
                    SampleType::I16 => i16::from_ne_bytes([s[0], s[1]]) == -5,
                    _ if text == "nan" => f32::from_ne_bytes(s.try_into().unwrap()).is_nan(),
                    _ => f32::from_ne_bytes(s.try_into().unwrap()) == 100.5,
                })
            })
            .collect();
        assert_eq!(
            f.validity.clone().unwrap_or_else(|| vec![true; 36]),
            expect,
            "{t:?} {text}"
        );
    }
    // A value no pixel holds marks nothing.
    let mut img = Img::new(4, 4, 1, SampleType::U16);
    img.tags.push((42113, 2, 6, b"70000\0".to_vec()));
    let source = GeoTiffSource::open(write(&[img.clone()], false, false)).unwrap();
    assert_eq!(source.read_fragment(0, whole(&img)).unwrap().validity, None);
}

/// A pixel with one sample holding the value and another not is not missing.
#[test]
fn nodata_in_one_sample_only_is_not_missing() {
    // Two bands of U8 whose values differ at every pixel, so no pixel holds 0 in both, while
    // some hold 0 in one.
    let mut img = Img::new(16, 16, 2, SampleType::U8);
    img.tags.push((42113, 2, 2, b"0\0".to_vec()));
    let source = GeoTiffSource::open(write(&[img.clone()], false, false)).unwrap();
    let f = source.read_fragment(0, whole(&img)).unwrap();
    let both = f
        .data
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|p| p[0] == 0 && p[1] == 0)
        .count();
    let missing = f
        .validity
        .as_ref()
        .map_or(0, |v| v.iter().filter(|&&x| !x).count());
    assert_eq!(missing, both);
}

#[test]
fn tags_and_geotiff_keys_are_fields() {
    let mut img = Img::new(4, 5, 1, SampleType::U8);
    let le = |v: &[u16]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    // GTModelType 2, GeographicType 4326 inline; a semi-major axis in the doubles; a citation in
    // the text.
    let dir = [
        1u16, 1, 0, 4, 1024, 0, 1, 2, 1026, 34737, 6, 0, 2048, 0, 1, 4326, 2057, 34736, 1, 0,
    ];
    img.tags.push((34735, 3, dir.len() as u64, le(&dir)));
    let doubles: Vec<u8> = [6_378_137.0f64, 0.5]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    img.tags.push((34736, 12, 2, doubles));
    img.tags.push((34737, 2, 8, b"WGS 84|\0".to_vec()));
    img.tags.push((
        33550,
        12,
        3,
        [0.25f64, 0.5, 0.0]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect(),
    ));
    img.tags.push((65000, 3, 1, le(&[7])));
    img.tags.push((305, 2, 4, b"BIL\0".to_vec()));
    for big in [false, true] {
        let source = GeoTiffSource::open(write(&[img.clone()], big, false)).unwrap();
        assert_eq!(
            source.list_field_groups(Scope::Source).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            source.list_field_groups(Scope::Image(0)).unwrap(),
            ["tiff.ifd", "geotiff.keys"]
        );
        let f = |n: &str| source.read_field(Scope::Image(0), n).unwrap();
        assert_eq!(f("tiff.ifd.ImageWidth").value(), &Value::Integer(5));
        assert_eq!(f("tiff.ifd.Software").value(), &Value::Text("BIL".into()));
        assert_eq!(f("tiff.ifd.Software").stored, b"BIL\0");
        assert_eq!(f("tiff.ifd.65000").value(), &Value::Integer(7));
        assert_eq!(
            f("tiff.ifd.65000").stored,
            if big { vec![0, 7] } else { vec![7, 0] }
        );
        assert_eq!(
            f("tiff.ifd.ModelPixelScaleTag").value(),
            &Value::List(vec![
                Value::Float(0.25),
                Value::Float(0.5),
                Value::Float(0.0)
            ])
        );
        assert_eq!(
            f("geotiff.keys.GTModelTypeGeoKey").value(),
            &Value::Integer(2)
        );
        assert_eq!(
            f("geotiff.keys.GeographicTypeGeoKey").value(),
            &Value::Integer(4326)
        );
        assert_eq!(
            f("geotiff.keys.GTCitationGeoKey").value(),
            &Value::Text("WGS 84".into())
        );
        assert_eq!(
            f("geotiff.keys.GeogSemiMajorAxisGeoKey").value(),
            &Value::Float(6_378_137.0)
        );
        assert!(matches!(
            source.read_field(Scope::Image(0), "tiff.ifd.Nothing"),
            Err(Error::NotFound(_))
        ));
    }
}

#[test]
fn unsupported_compression_is_reported_not_decoded() {
    let mut img = Img::new(4, 4, 1, SampleType::U8);
    img.compression = 5;
    let source = GeoTiffSource::open(write(&[img.clone()], false, false)).unwrap();
    let info = source.image_info(0).unwrap();
    assert_eq!(
        (info.codec.as_deref(), info.decodable),
        (Some("lzw"), false)
    );
    assert!(matches!(
        source.read_fragment(0, whole(&img)),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn claims_beyond_the_file_are_refused() {
    let img = Img::new(8, 8, 1, SampleType::U8);
    let mut f = write(std::slice::from_ref(&img), false, false);
    // Cut into the pixel data, which the offsets still claim.
    f.drain(8..20);
    assert!(GeoTiffSource::open(f).is_err());
    assert!(matches!(
        GeoTiffSource::open(b"GIF89a....".to_vec()),
        Err(Error::NotTiff)
    ));
    let source = GeoTiffSource::open(write(&[img], false, false)).unwrap();
    let w = Window {
        row_start: 7,
        col_start: 0,
        row_count: 2,
        col_count: 1,
    };
    assert!(matches!(
        source.read_fragment(0, w),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn a_chain_that_loops_is_corrupt() {
    let img = Img::new(2, 2, 1, SampleType::U8);
    let mut f = write(&[img], false, false);
    // Point the only IFD's next-IFD field back at itself.
    let at = u32::from_le_bytes(f[4..8].try_into().unwrap()) as usize;
    let n = u16::from_le_bytes([f[at], f[at + 1]]) as usize;
    let next = at + 2 + 12 * n;
    f[next..next + 4].copy_from_slice(&(at as u32).to_le_bytes());
    assert!(matches!(GeoTiffSource::open(f), Err(Error::Corrupt(_))));
}

/// Seeded corruption of valid files: every one opens or is refused, and every read delivers or
/// fails, without a panic.
#[test]
fn corrupt_files_are_errors_never_panics() {
    let mut a = Img::new(20, 24, 2, SampleType::U16);
    a.tile = Some((16, 16));
    a.compression = 8;
    a.predictor = 2;
    a.tags.push((42113, 2, 2, b"0\0".to_vec()));
    let mut o = Img::new(10, 12, 2, SampleType::U16);
    o.subfile = 1;
    let mut b = Img::new(9, 11, 1, SampleType::F32);
    b.planar = true;
    b.compression = 8;
    b.predictor = 3;
    let dir = [1u16, 1, 0, 1, 2048, 0, 1, 4326];
    b.tags.push((
        34735,
        3,
        8,
        dir.iter().flat_map(|x| x.to_le_bytes()).collect(),
    ));
    let seeds = [write(&[a, o], false, false), write(&[b], true, true)];
    let count: u64 = std::env::var("BIL_CORRUPT_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let mut rng = Rng(2026);
    let (mut opened, mut refused) = (0, 0);
    for _ in 0..count {
        let mut d = seeds[rng.below(2) as usize].clone();
        for _ in 0..1 + rng.below(4) {
            if d.is_empty() {
                d.push(0);
            }
            let at = rng.below(d.len() as u64) as usize;
            match rng.below(5) {
                0 => d[at] ^= 1 << rng.below(8),
                1 => d[at] = rng.next() as u8,
                2 => d[at] = [0, 0xFF, 0x7F, 0x80][rng.below(4) as usize],
                3 => d.truncate(at),
                _ => {
                    let n = rng.below(16) as usize;
                    let run: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                    d.splice(at..at, run);
                }
            }
        }
        let size = d.len() as u64;
        let Ok(source) = GeoTiffSource::open(d) else {
            refused += 1;
            continue;
        };
        opened += 1;
        for i in 0..source.image_count() {
            for g in source.list_field_groups(Scope::Image(i)).unwrap() {
                let _ = source.read_field_group(Scope::Image(i), &g);
            }
            let _ = source.describe_storage(i);
            let info = source.image_info(i).unwrap().clone();
            let bytes = u128::from(info.rows) * u128::from(info.cols) * u128::from(info.bands) * 8;
            if bytes <= 64 * u128::from(size) {
                let w = Window {
                    row_start: 0,
                    col_start: 0,
                    row_count: info.rows as u32,
                    col_count: info.cols as u32,
                };
                let _ = source.read_fragment(i, w);
            }
        }
    }
    eprintln!("corrupt files: {opened} opened, {refused} refused");
    assert!(
        opened > 500 && refused > 500,
        "{opened} opened, {refused} refused"
    );
}

/// A chunk of no bytes was never written, as in a sparse file: its pixels are 0 and marked
/// missing, and the others read as written (section 4).
#[test]
fn chunks_never_written_are_missing() {
    let mut img = Img::new(20, 30, 1, SampleType::U16);
    img.tile = Some((16, 16));
    img.sparse = vec![1, 2];
    let source = GeoTiffSource::open(write(std::slice::from_ref(&img), false, false)).unwrap();
    let f = source.read_fragment(0, whole(&img)).unwrap();
    let valid = f.validity.clone().unwrap();
    for r in 0..20u64 {
        for c in 0..30u64 {
            let chunk = (r / 16) * 2 + c / 16;
            let p = (r * 30 + c) as usize;
            let v = u16::from_ne_bytes([f.data[2 * p], f.data[2 * p + 1]]);
            if chunk == 1 || chunk == 2 {
                assert!(!valid[p] && v == 0, "row {r} col {c}");
            } else {
                assert!(valid[p], "row {r} col {c}");
                assert_eq!(u64::from(v), sample(SampleType::U16, r, c, 0));
            }
        }
    }
    // Storage lists only the chunks that were written.
    assert_eq!(source.describe_storage(0).unwrap().blocks.len(), 2);
}
