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
        // Any half whose exponent's top bit is clear: finite, of either sign, subnormals too.
        SampleType::F16 => bits_at(r, c, b, 16) & 0xBFFF,
        t => bits_at(r, c, b, t.size() as u32 * 8),
    }
}

fn format_of(t: SampleType) -> u16 {
    match t {
        SampleType::I8 | SampleType::I16 | SampleType::I32 | SampleType::I64 => 2,
        SampleType::F16 | SampleType::F32 | SampleType::F64 => 3,
        SampleType::ComplexI8 | SampleType::ComplexI16 => 5,
        SampleType::ComplexF32 | SampleType::ComplexF64 => 6,
        _ => 1,
    }
}

/// The sample a reader delivers for `img`: as `sample`, or for a width of its own (M43) the
/// stored bits, sign-extended where signed, and widened to the type's size.
fn expected(img: &Img, r: u64, c: u64, b: u64) -> u64 {
    let v = sample(img.t, r, c, b);
    match img.bits {
        Some(bits) => {
            let v = v & ((1u64 << bits) - 1);
            let size = img.t.size() * 8;
            if format_of(img.t) == 2 && v >> (bits - 1) & 1 == 1 {
                (v | !0u64 << bits) & if size == 64 { !0 } else { (1u64 << size) - 1 }
            } else {
                v
            }
        }
        None => v,
    }
}

/// PackBits (TIFF 6.0, section 9): runs of three or more as runs, the rest as literals.
fn packbits(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let mut run = 1;
        while i + run < raw.len() && raw[i + run] == raw[i] && run < 128 {
            run += 1;
        }
        if run >= 3 {
            out.push((1 - run as i32) as u8);
            out.push(raw[i]);
            i += run;
        } else {
            let start = i;
            while i < raw.len() && i - start < 128 {
                if i + 2 < raw.len() && raw[i] == raw[i + 1] && raw[i] == raw[i + 2] {
                    break;
                }
                i += 1;
            }
            out.push((i - start - 1) as u8);
            out.extend_from_slice(&raw[start..i]);
        }
    }
    out
}

// ---------------------------------------------------------------- a writer for test files

/// Samples of a width of their own: a stream of bits, most significant first, whatever the
/// file's byte order, each row starting on a byte.
#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    used: u32,
}

impl BitWriter {
    fn put(&mut self, v: u64, bits: u32) {
        for k in (0..bits).rev() {
            if self.used.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = (v >> k & 1) as u8;
            *self.bytes.last_mut().unwrap() |= bit << (7 - self.used % 8);
            self.used += 1;
        }
    }
}

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
    /// 1 none, 8 Deflate, 5 LZW, 32773 PackBits, or another code whose data is written raw.
    compression: u16,
    /// Stored bits of a sample, where not the type's own (M43).
    bits: Option<u32>,
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
            bits: None,
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
                        let mut packed = BitWriter::default();
                        for c in 0..cw {
                            for k in 0..per {
                                let (rr, cc2, b) = (
                                    cr * ch + r,
                                    cc * cw + c,
                                    if self.planar { plane } else { k },
                                );
                                let v = if rr < self.rows && cc2 < self.cols {
                                    sample(self.t, rr, cc2, b)
                                } else {
                                    0
                                };
                                match self.bits {
                                    Some(bits) => packed.put(v, bits),
                                    None => row.push(v),
                                }
                            }
                        }
                        if self.bits.is_some() {
                            raw.extend(packed.bytes);
                        } else {
                            raw.extend(self.predict(&row, per as usize, big));
                        }
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
                        5 => weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
                            .encode(&raw)
                            .unwrap(),
                        32773 => packbits(&raw),
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
        // Each number in the file's byte order: a complex sample's two parts apart.
        let number = self.t.number_size();
        let bytes = |v: u64| -> Vec<u8> {
            let le = &v.to_le_bytes()[..size];
            if big {
                le.chunks(number)
                    .flat_map(|n| n.iter().rev().copied())
                    .collect()
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
                short(&vec![
                    img.bits.unwrap_or(img.t.size() as u32 * 8) as u16;
                    img.bands as usize
                ]),
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
        // A test's own tag replaces the default of the same code.
        e.retain(|x| !img.tags.iter().any(|t| t.0 == x.0));
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
    check_fragment(&source.read_fragment(image, w).unwrap(), img, w);
}

/// A Fragment against the samples of `img` in `w`, in `img`'s own rows and columns.
fn check_fragment(f: &bil::types::Fragment, img: &Img, w: Window) {
    assert_eq!(f.window, w);
    let size = img.t.size();
    for r in 0..u64::from(w.row_count) {
        for c in 0..u64::from(w.col_count) {
            for b in 0..img.bands {
                let at = r as usize * f.strides[0]
                    + c as usize * f.strides[1]
                    + b as usize * f.strides[2];
                let want = expected(img, w.row_start + r, w.col_start + c, b).to_ne_bytes();
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
        SampleType::F16,
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
            let predictor = if format_of(t) == 3 { 3 } else { 2 };
            for (compression, predictor) in
                [(1, 1), (8, 1), (8, predictor), (5, predictor), (32773, 1)]
            {
                for (big, bigtiff) in [(false, false), (true, false), (false, true)] {
                    // 21 × 37: partial strips and tiles on the right and at the bottom; one to
                    // four bands in turn.
                    let bands = 1 + cases % 4;
                    let mut img = Img::new(21, 37, bands, t);
                    img.tile = tile;
                    img.planar = planar;
                    img.compression = compression;
                    img.predictor = predictor;
                    let source = GeoTiffSource::open(write(&[img.clone()], big, bigtiff))
                        .unwrap_or_else(|e| panic!("{t:?} {tile:?} {planar} {compression} {predictor} {big} {bigtiff}: {e}"));
                    let info = source.image_info(0).unwrap();
                    assert_eq!(
                        (info.rows, info.cols, u64::from(info.bands)),
                        (21, 37, bands)
                    );
                    assert_eq!(info.pixel_type.unwrap().sample_type, t);
                    random_windows(&source, &img, &mut rng, 6);
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 11 * 4 * 5 * 3);
}

/// A stored level is read as it stands: the pixels whose centers fall inside a full-resolution
/// Window, an overview covering its Image's extent (section 5; M46, M47).
#[test]
fn stored_levels_read_the_pixels_whose_centers_fall_inside() {
    let full = Img::new(40, 60, 2, SampleType::U16);
    let mut o1 = Img::new(20, 30, 2, SampleType::U16);
    o1.subfile = 1;
    o1.tile = Some((16, 16));
    let mut o2 = Img::new(10, 15, 2, SampleType::U16);
    o2.subfile = 1;
    o2.compression = 8;
    let source =
        GeoTiffSource::open(write(&[full.clone(), o1.clone(), o2.clone()], true, false)).unwrap();
    let all = whole(&full);
    let level0 = source.read_level_fragment(0, "level0", all).unwrap();
    check_fragment(&level0, &full, all);
    check_fragment(
        &source.read_level_fragment(0, "stored1", all).unwrap(),
        &o1,
        whole(&o1),
    );
    check_fragment(
        &source.read_level_fragment(0, "stored2", all).unwrap(),
        &o2,
        whole(&o2),
    );
    // Counting each full-resolution pixel's center as a whole number, stored1's pixel i has its
    // center at 0.5 + 2i. Rows 3 to 10 span 2.5 to 10.5 and hold rows 1 to 4; columns 5 and 6
    // span 4.5 to 6.5 and hold column 2 only, at 4.5.
    let w = Window {
        row_start: 3,
        col_start: 5,
        row_count: 8,
        col_count: 2,
    };
    let f = source.read_level_fragment(0, "stored1", w).unwrap();
    let want = Window {
        row_start: 1,
        col_start: 2,
        row_count: 4,
        col_count: 1,
    };
    check_fragment(&f, &o1, want);
    // A Window holding no center gives no pixels.
    let thin = Window {
        row_start: 0,
        col_start: 0,
        row_count: 1,
        col_count: 60,
    };
    assert_eq!(
        source
            .read_level_fragment(0, "stored2", thin)
            .unwrap()
            .window
            .row_count,
        0
    );
    for bad in ["stored3", "stored0", "level1", "stored01"] {
        assert!(matches!(
            source.read_level_fragment(0, bad, all),
            Err(Error::NotFound(_))
        ));
    }
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
        assert!(source.info(0).unwrap().may_have_missing);
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
        // Stored bytes in the file's byte order, as tags' are.
        let axis = 6_378_137.0f64;
        assert_eq!(
            f("geotiff.keys.GeogSemiMajorAxisGeoKey").stored,
            if big {
                axis.to_be_bytes()
            } else {
                axis.to_le_bytes()
            }
        );
        assert_eq!(
            f("geotiff.keys.GTModelTypeGeoKey").stored,
            if big { vec![0, 2] } else { vec![2, 0] }
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
    img.compression = 50000;
    let source = GeoTiffSource::open(write(&[img.clone()], false, false)).unwrap();
    let info = source.image_info(0).unwrap();
    assert_eq!(
        (info.codec.as_deref(), info.decodable),
        (Some("zstd"), false)
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
        Err(Error::Unrecognized(_))
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
    assert!(source.info(0).unwrap().may_have_missing);
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

/// Samples come back as stored whatever the photometric interpretation (section 4, M39):
/// WhiteIsZero is not inverted, floats included.
#[test]
fn white_is_zero_samples_come_back_as_stored() {
    for t in [
        SampleType::U8,
        SampleType::U16,
        SampleType::U32,
        SampleType::F32,
    ] {
        let mut img = Img::new(5, 9, 1, t);
        img.tags.push((262, 3, 1, 0u16.to_le_bytes().to_vec()));
        let source = GeoTiffSource::open(write(std::slice::from_ref(&img), false, false)).unwrap();
        check(&source, 0, &img, whole(&img));
    }
}

#[test]
fn extra_samples_of_unspecified_meaning_are_kept() {
    // RGB with a fourth sample marked unspecified: all four come back.
    let mut img = Img::new(5, 9, 4, SampleType::U8);
    img.tags.push((262, 3, 1, 2u16.to_le_bytes().to_vec()));
    img.tags.push((338, 3, 1, 0u16.to_le_bytes().to_vec()));
    let source = GeoTiffSource::open(write(std::slice::from_ref(&img), false, false)).unwrap();
    check(&source, 0, &img, whole(&img));
}

/// An interpretation the `tiff` crate does not know, or none, in the first IFD: listed and read,
/// not refused at open (M39, M42). A predictor TIFF does not define: listed, its reads refused.
#[test]
fn the_first_ifd_is_listed_however_the_tiff_crate_judges_it() {
    let mut cielab = Img::new(5, 9, 3, SampleType::U8);
    cielab.tags.push((262, 3, 1, 8u16.to_le_bytes().to_vec()));
    let mut unknown = Img::new(5, 9, 1, SampleType::U16);
    unknown
        .tags
        .push((262, 3, 1, 999u16.to_le_bytes().to_vec()));
    for img in [cielab, unknown] {
        for (big, bigtiff) in [(false, false), (true, false), (true, true)] {
            let source =
                GeoTiffSource::open(write(std::slice::from_ref(&img), big, bigtiff)).unwrap();
            check(&source, 0, &img, whole(&img));
        }
    }
    let mut odd = Img::new(5, 9, 1, SampleType::U8);
    odd.tags.push((317, 3, 1, 7u16.to_le_bytes().to_vec()));
    let source = GeoTiffSource::open(write(std::slice::from_ref(&odd), false, false)).unwrap();
    assert!(source.image_info(0).unwrap().unsupported.is_some());
    assert!(matches!(
        source.read_fragment(0, whole(&odd)),
        Err(Error::Unsupported(_))
    ));
}

/// Integers of widths with no type of their own widen, with `significant_bits` the stored
/// width; complex samples map to the complex types (M43).
#[test]
fn other_widths_widen_and_complex_samples_map() {
    let mut rng = Rng(5);
    for (t, bits) in [
        (SampleType::U8, 1),
        (SampleType::U8, 4),
        (SampleType::I8, 7),
        (SampleType::U16, 12),
        (SampleType::I16, 12),
        (SampleType::U32, 24),
        (SampleType::I32, 24),
        (SampleType::U32, 31),
        (SampleType::U64, 40),
        (SampleType::I64, 63),
    ] {
        for planar in [false, true] {
            for (compression, big) in [(1, false), (1, true), (8, true), (5, false)] {
                let mut img = Img::new(13, 19, 2, t);
                img.bits = Some(bits);
                img.planar = planar;
                img.compression = compression;
                let source =
                    GeoTiffSource::open(write(std::slice::from_ref(&img), big, false)).unwrap();
                let pt = source.image_info(0).unwrap().pixel_type.unwrap();
                assert_eq!((pt.sample_type, pt.significant_bits), (t, bits));
                random_windows(&source, &img, &mut rng, 4);
            }
        }
    }
    for t in [
        SampleType::ComplexI8,
        SampleType::ComplexI16,
        SampleType::ComplexF32,
    ] {
        for big in [false, true] {
            let img = Img::new(7, 11, 2, t);
            let source =
                GeoTiffSource::open(write(std::slice::from_ref(&img), big, false)).unwrap();
            check(&source, 0, &img, whole(&img));
        }
    }
    // A 24-bit float has no type: listed, its reads refused.
    let mut img = Img::new(4, 4, 1, SampleType::F32);
    img.bits = Some(24);
    let source = GeoTiffSource::open(write(std::slice::from_ref(&img), false, false)).unwrap();
    assert!(source.image_info(0).unwrap().pixel_type.is_none());
    assert!(matches!(
        source.read_fragment(0, whole(&img)),
        Err(Error::Unsupported(_))
    ));
}

/// Text is read from its own bytes, as Latin-1, every string kept (M36, M39).
#[test]
fn text_is_latin1_and_every_string_is_kept() {
    let mut img = Img::new(4, 4, 1, SampleType::U8);
    img.tags.push((270, 2, 9, b"caf\xE9\0two\0".to_vec()));
    img.tags.push((305, 2, 3, b"ab\0".to_vec()));
    let source = GeoTiffSource::open(write(std::slice::from_ref(&img), true, false)).unwrap();
    let f = source
        .read_field(Scope::Image(0), "tiff.ifd.ImageDescription")
        .unwrap();
    assert_eq!(
        f.value(),
        &Value::List(vec![Value::Text("café".into()), Value::Text("two".into())])
    );
    assert_eq!(f.stored, b"caf\xE9\0two\0");
    let f = source
        .read_field(Scope::Image(0), "tiff.ifd.Software")
        .unwrap();
    assert_eq!(f.value(), &Value::Text("ab".into()));
}

/// GDAL_NODATA rounds to float16 as to float32 (M37).
#[test]
fn nodata_rounds_to_float16() {
    // 0.1 is not a float16; its nearest is 0x2E66. Write every sample as that.
    let mut img = Img::new(2, 2, 1, SampleType::F16);
    img.tags.push((42113, 2, 4, b"0.1\0".to_vec()));
    let mut bytes = write(std::slice::from_ref(&img), false, false);
    let source = GeoTiffSource::open(bytes.clone()).unwrap();
    let blocks = source.describe_storage(0).unwrap().blocks;
    for b in &blocks {
        for k in (b.offset..b.offset + b.length).step_by(2) {
            bytes[k as usize..k as usize + 2].copy_from_slice(&0x2E66u16.to_le_bytes());
        }
    }
    let source = GeoTiffSource::open(bytes).unwrap();
    let f = source.read_fragment(0, whole(&img)).unwrap();
    assert_eq!(f.validity, Some(vec![false; 4]));
}

#[test]
fn layouts_that_cannot_be_counted_are_corrupt_not_panics() {
    // Behind a valid first Image, so the second IFD's layout is BIL's own to check.
    let first = Img::new(2, 2, 1, SampleType::U8);
    let open_with = |tags: Vec<(u16, u16, u64, Vec<u8>)>, tile: Option<(u64, u64)>, bands| {
        let mut second = Img::new(2, 2, bands, SampleType::U8);
        second.tile = tile;
        second.planar = bands > 1;
        second.tags = tags;
        GeoTiffSource::open(write(&[first.clone(), second], false, false))
    };
    let huge = u32::MAX.to_le_bytes().to_vec();
    // Chunks of 1 × 1 over 2³² − 1 square pixels in two planes: more than a u64 counts.
    let r = open_with(
        vec![(256, 4, 1, huge.clone()), (257, 4, 1, huge)],
        Some((1, 1)),
        2,
    );
    assert!(matches!(r, Err(Error::Corrupt(_))), "{:?}", r.err());
    // A SampleFormat beyond 16 bits is not truncated into a valid one.
    let r = open_with(vec![(339, 4, 1, 65_537u32.to_le_bytes().to_vec())], None, 1);
    assert!(matches!(r, Err(Error::Corrupt(_))), "{:?}", r.err());
    // BitsPerSample with no values.
    let r = open_with(vec![(258, 3, 0, Vec::new())], None, 1);
    assert!(r.is_err());
}

#[test]
fn a_key_listed_twice_carries_its_place() {
    let mut img = Img::new(2, 2, 1, SampleType::U8);
    let dir = [1u16, 1, 0, 2, 1024, 0, 1, 1, 1024, 0, 1, 2];
    let le: Vec<u8> = dir.iter().flat_map(|x| x.to_le_bytes()).collect();
    img.tags.push((34735, 3, dir.len() as u64, le));
    let source = GeoTiffSource::open(write(&[img], false, false)).unwrap();
    let f = |n: &str| source.read_field(Scope::Image(0), n);
    assert_eq!(
        f("geotiff.keys.GTModelTypeGeoKey[1]").unwrap().value(),
        &Value::Integer(1)
    );
    assert_eq!(
        f("geotiff.keys.GTModelTypeGeoKey[2]").unwrap().value(),
        &Value::Integer(2)
    );
    assert!(f("geotiff.keys.GTModelTypeGeoKey").is_err());
}

/// Sets a LONG or SHORT tag's value in a little-endian classic file's first IFD.
fn patch_tag(file: &mut [u8], tag: u16, value: u32) {
    let ifd = u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize;
    let count = u16::from_le_bytes([file[ifd], file[ifd + 1]]) as usize;
    for k in 0..count {
        let e = ifd + 2 + 12 * k;
        if u16::from_le_bytes([file[e], file[e + 1]]) == tag {
            file[e + 8..e + 12].copy_from_slice(&value.to_le_bytes());
            return;
        }
    }
    panic!("no tag {tag}");
}

/// A chunk whose declared size its stored bytes cannot make is refused before memory is taken
/// for it, not decoded into a buffer the size of the claim.
#[test]
fn a_chunk_larger_than_its_bytes_can_make_is_refused_first() {
    for compression in [1, 8, 5, 32773] {
        let mut img = Img::new(1, 1, 4, SampleType::U64);
        img.tile = Some((16, 16));
        img.compression = compression;
        let mut file = write(std::slice::from_ref(&img), false, false);
        patch_tag(&mut file, 322, 1 << 30);
        patch_tag(&mut file, 323, 1 << 30);
        let source = GeoTiffSource::open(file).unwrap();
        assert!(matches!(
            source.read_fragment(0, whole(&img)),
            Err(Error::Corrupt(_))
        ));
    }
}

/// A file cut inside its IFD reads its end, not the placeholder the `tiff` crate is given.
#[test]
fn a_cut_ifd_is_corrupt_or_refused_never_filled_in() {
    let mut img = Img::new(3, 3, 1, SampleType::U8);
    img.tags.push((270, 2, 40, vec![b'x'; 40]));
    let file = write(std::slice::from_ref(&img), false, false);
    for cut in 1..40 {
        let short = file[..file.len() - cut].to_vec();
        if let Ok(source) = GeoTiffSource::open(short) {
            // Whatever it lists came from the file.
            let _ = source.read_field_group(Scope::Image(0), "tiff.ifd");
        }
    }
}

#[test]
fn the_source_interface_opens_tiff_and_reads_its_levels() {
    let full = Img::new(40, 60, 2, SampleType::U16);
    let mut o1 = Img::new(20, 30, 2, SampleType::U16);
    o1.subfile = 1;
    o1.tile = Some((16, 16));
    let file = write(&[full.clone(), o1], false, false);
    let source = bil::OpenOptions::default().open_from(file.clone()).unwrap();
    assert_eq!((source.driver(), source.image_count()), ("geotiff", 1));
    let image = source.image(0).unwrap();
    let info = image.info().unwrap();
    assert_eq!(
        (info.row_count, info.col_count, info.component_count),
        (40, 60, 2)
    );
    assert_eq!(info.frame_count, bil::types::FrameCount::Known(1));
    assert_eq!(
        (info.storage.interleave, info.storage.order),
        (Interleave::Pixel, StorageOrder::Known)
    );
    assert!(info.storage.blocks.is_empty() && info.relations.is_empty());
    // Integers with no no-data value and every chunk written: nothing can be missing.
    assert!(!info.may_have_missing);
    assert_eq!(source.list_images().unwrap(), [info]);
    let levels = image.list_stored_levels().unwrap();
    assert_eq!((levels.len(), levels[0].name.as_str()), (1, "stored1"));
    // A stored level's Storage is its own: its chunk size, and its blocks marked with it.
    let s = image.describe_storage("stored1").unwrap();
    assert_eq!((s.block_size, s.blocks.len()), ((16, 16), 4));
    assert!(s.blocks.iter().all(|b| b.level == 1));
    assert!(image
        .describe_storage("level0")
        .unwrap()
        .blocks
        .iter()
        .all(|b| b.level == 0));
    assert!(matches!(
        image.describe_storage("stored2"),
        Err(Error::NotFound(_))
    ));
    let f = image.read_fragment(whole(&full), "level0").unwrap();
    check_fragment(&f, &full, whole(&full));
    assert!(matches!(source.image(1), Err(Error::NotFound(_))));
    // M41's limits come from OpenOptions: a chain of two IFDs is refused at one.
    let one = bil::OpenOptions {
        max_directories: 1,
        ..Default::default()
    };
    assert!(matches!(
        one.open_from(file.clone()),
        Err(Error::Unsupported(_))
    ));
    let two = bil::OpenOptions {
        max_directories: 2,
        ..Default::default()
    };
    assert!(two.open_from(file).is_ok());
}

/// Representations and component names follow Photometric and the tags beside it (section 3,
/// "Representations").
#[test]
fn representations_follow_photometric() {
    use bil::types::{ColorModel, LabEncoding, Representation as R, BT601};
    let tag = |code: u16, v: &[u16]| {
        (
            code,
            3u16,
            v.len() as u64,
            v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>(),
        )
    };
    let case = |bands: u64, t: SampleType, tags: Vec<(u16, u16, u64, Vec<u8>)>| {
        let mut img = Img::new(4, 5, bands, t);
        img.tags = tags;
        let source = GeoTiffSource::open(write(&[img], false, false)).unwrap();
        let info = source.info(0).unwrap();
        (info.representation, info.component_names)
    };
    let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        case(1, SampleType::U8, vec![]),
        (
            R::Intensity {
                white_is_zero: false
            },
            names(&["intensity"])
        )
    );
    assert_eq!(
        case(1, SampleType::U16, vec![tag(262, &[0])]).0,
        R::Intensity {
            white_is_zero: true
        }
    );
    assert_eq!(
        case(3, SampleType::U8, vec![]),
        (R::Color(ColorModel::Rgb), names(&["red", "green", "blue"]))
    );
    assert_eq!(
        case(4, SampleType::U16, vec![]),
        (
            R::Spectral,
            names(&["component1", "component2", "component3", "component4"])
        )
    );
    assert_eq!(
        case(5, SampleType::U8, vec![tag(262, &[2]), tag(338, &[2, 0])]),
        (
            R::Color(ColorModel::Rgb),
            names(&["red", "green", "blue", "alpha", "component5"])
        )
    );
    // Two gray samples and an alpha: spectral, alpha named (M58).
    assert_eq!(
        case(3, SampleType::U8, vec![tag(262, &[1]), tag(338, &[2])]),
        (R::Spectral, names(&["component1", "component2", "alpha"]))
    );
    assert_eq!(
        case(3, SampleType::U8, vec![tag(262, &[8])]),
        (
            R::Color(ColorModel::Lab {
                encoding: LabEncoding::Cie
            }),
            names(&["lightness", "green_red", "blue_yellow"])
        )
    );
    assert_eq!(
        case(4, SampleType::U8, vec![tag(262, &[5])]).1,
        names(&["cyan", "magenta", "yellow", "black"])
    );
    assert_eq!(
        case(3, SampleType::U8, vec![tag(262, &[6])]).0,
        R::Color(ColorModel::YCbCr {
            coefficients: BT601
        })
    );
    assert_eq!(
        case(1, SampleType::U8, vec![tag(262, &[3])]).1,
        names(&["index"])
    );
    assert_eq!(
        case(2, SampleType::U16, vec![tag(262, &[32803])]),
        (R::Vector, names(&["component1", "component2"]))
    );
    assert_eq!(
        case(1, SampleType::ComplexF32, vec![]),
        (R::Complex, names(&["complex"]))
    );
}

/// Chunks are decoded once and cut for each Window; the cache's limits hold (section 4).
#[test]
fn the_cache_decodes_a_chunk_once() {
    let mut img = Img::new(40, 40, 2, SampleType::U16);
    img.tile = Some((16, 16));
    img.compression = 8;
    let file = write(std::slice::from_ref(&img), false, false);
    let source = GeoTiffSource::open(file.clone()).unwrap();
    let w = |r, c| Window {
        row_start: r,
        col_start: c,
        row_count: 8,
        col_count: 8,
    };
    // Two Windows inside the first tile: one decode, one hit.
    check(&source, 0, &img, w(0, 0));
    check(&source, 0, &img, w(4, 4));
    let r = source.describe_cache();
    assert_eq!((r.misses, r.hits, r.fragment_count), (1, 1, 1));
    assert_eq!(r.bytes, 16 * 16 * 2 * 2);
    // A Window across four tiles finds one cached.
    check(&source, 0, &img, w(12, 12));
    let r = source.describe_cache();
    assert_eq!((r.misses, r.hits, r.fragment_count), (4, 2, 4));
    // The whole Image, then a ceiling of one tile's bytes: one held, and none past it.
    check(&source, 0, &img, whole(&img));
    assert_eq!(source.describe_cache().fragment_count, 9);
    source.set_cache_bytes(16 * 16 * 4);
    let r = source.describe_cache();
    assert_eq!((r.fragment_count, r.held_past_ceiling), (1, 0));
    // A ceiling below one tile: the tile is held alone, and that is reported.
    source.set_cache_bytes(100);
    // Lowering the ceiling below the one block held keeps it, and warns.
    assert_eq!(source.describe_cache().held_past_ceiling, 1);
    let tile = Window {
        row_start: 0,
        col_start: 0,
        row_count: 16,
        col_count: 16,
    };
    check(&source, 0, &img, tile);
    let r = source.describe_cache();
    assert_eq!((r.fragment_count, r.held_past_ceiling), (1, 2));
    // The tile held past the ceiling serves a read of part of it.
    let before = source.describe_cache();
    check(&source, 0, &img, w(4, 4));
    let after = source.describe_cache();
    assert_eq!((after.hits, after.misses), (before.hits + 1, before.misses));
    // A Window needing fewer rows than such a tile holds decodes only those, keeps nothing,
    // and leaves what the cache holds as it was (M67).
    check(&source, 0, &img, w(16, 0));
    let r = source.describe_cache();
    assert_eq!((r.fragment_count, r.held_past_ceiling), (1, 2));
    source.clear_cache();
    assert_eq!(source.describe_cache().bytes, 0);
    // Off: the same pixels, nothing kept, every chunk a miss.
    let off = GeoTiffSource::open(file).unwrap();
    off.set_cache_size(0);
    check(&off, 0, &img, whole(&img));
    check(&off, 0, &img, w(4, 4));
    let r = off.describe_cache();
    assert_eq!((r.fragment_count, r.hits, r.misses), (0, 0, 10));
    // Through the Source interface too.
    let source = bil::OpenOptions::default()
        .open_from(write(std::slice::from_ref(&img), false, false))
        .unwrap();
    source.set_cache_size(3);
    let f = source
        .image(0)
        .unwrap()
        .read_fragment(whole(&img), "level0")
        .unwrap();
    check_fragment(&f, &img, whole(&img));
    let r = source.describe_cache();
    assert_eq!((r.fragment_count, r.max_fragments, r.misses), (3, 3, 9));
}
