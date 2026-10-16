// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The NITF reader against synthetic files whose contents are known by construction
//! (specification, section 16, "Test data"). A small writer here builds them, field by field,
//! from MIL-STD-2500A and C, independently of the reader's own definitions.

#![cfg(feature = "nitf")]

use bil::fields::{Handling, Value};
use bil::io::ReadAt;
use bil::nitf::{Error, NitfSource, Scope, SegmentKind, Version};
use bil::types::{Interleave, SampleType, Status, StorageOrder, Window};
use std::sync::Mutex;

// ---------------------------------------------------------------- a writer for test files

fn pad(s: &str, n: usize) -> Vec<u8> {
    assert!(s.len() <= n, "{s:?} is longer than {n}");
    let mut v = s.as_bytes().to_vec();
    v.resize(n, b' ');
    v
}

fn num(v: u64, n: usize) -> Vec<u8> {
    let s = format!("{v:0n$}");
    assert_eq!(s.len(), n, "{v} does not fit in {n} digits");
    s.into_bytes()
}

/// An extension: tag, length, data.
fn tre(tag: &str, data: &[u8]) -> Vec<u8> {
    let mut v = pad(tag, 6);
    v.extend(num(data.len() as u64, 5));
    v.extend_from_slice(data);
    v
}

/// The length field and data of an extension area, with its overflow field.
fn extension_area(data: &[u8], overflow: u64, len_digits: usize) -> Vec<u8> {
    if data.is_empty() && overflow == 0 {
        return num(0, len_digits);
    }
    let mut v = num(data.len() as u64 + 3, len_digits);
    v.extend(num(overflow, 3));
    v.extend_from_slice(data);
    v
}

#[derive(Clone)]
struct Img {
    rows: u64,
    cols: u64,
    bands: u64,
    pvtype: &'static str,
    nbpp: u64,
    abpp: u64,
    pjust: &'static str,
    imode: &'static str,
    block: (u64, u64),
    ic: &'static str,
    udid: Vec<u8>,
    ixshd: Vec<u8>,
    ixsofl: u64,
    data: Vec<u8>,
}

impl Img {
    fn new(rows: u64, cols: u64, bands: u64, pvtype: &'static str, nbpp: u64) -> Img {
        Img {
            rows,
            cols,
            bands,
            pvtype,
            nbpp,
            abpp: nbpp,
            pjust: "R",
            imode: "B",
            block: (rows, cols),
            ic: "NC",
            udid: Vec::new(),
            ixshd: Vec::new(),
            ixsofl: 0,
            data: Vec::new(),
        }
    }

    fn blocks(&self) -> (u64, u64) {
        (
            self.cols.div_ceil(self.block.1),
            self.rows.div_ceil(self.block.0),
        )
    }

    /// The NITF 2.1 image subheader.
    fn subheader_21(&self) -> Vec<u8> {
        let (nbpr, nbpc) = self.blocks();
        let mut h = Vec::new();
        h.extend(pad("IM", 2));
        h.extend(pad("IMAGE ONE", 10));
        h.extend(pad("20261014093015", 14));
        h.extend(pad("", 17));
        h.extend(pad("A test image", 80));
        h.extend(pad("U", 1));
        h.extend(pad("", 166)); // ISCLSY to ISCTLN
        h.extend(pad("0", 1));
        h.extend(pad("synthetic", 42));
        h.extend(num(self.rows, 8));
        h.extend(num(self.cols, 8));
        h.extend(pad(self.pvtype, 3));
        h.extend(pad(if self.bands == 1 { "MONO" } else { "MULTI" }, 8));
        h.extend(pad("VIS", 8));
        h.extend(num(self.abpp, 2));
        h.extend(pad(self.pjust, 1));
        h.extend(pad(" ", 1)); // ICORDS: none
        h.extend(pad("1", 1)); // NICOM
        h.extend(pad("one comment", 80));
        h.extend(pad(self.ic, 2));
        if self.ic != "NC" && self.ic != "NM" {
            h.extend(pad("N045", 4));
        }
        if self.bands < 10 {
            h.extend(num(self.bands, 1));
        } else {
            h.extend(pad("0", 1));
            h.extend(num(self.bands, 5));
        }
        for b in 0..self.bands {
            h.extend(pad(if self.bands == 1 { "" } else { "M" }, 2));
            h.extend(pad("", 6));
            h.extend(pad("N", 1));
            h.extend(pad("", 3));
            // The first band has a lookup table of 4 entries, to test the run.
            if b == 0 {
                h.extend(pad("1", 1));
                h.extend(num(4, 5));
                h.extend([1, 2, 3, 4]);
            } else {
                h.extend(pad("0", 1));
            }
        }
        h.extend(pad("0", 1));
        h.extend(pad(self.imode, 1));
        h.extend(num(nbpr, 4));
        h.extend(num(nbpc, 4));
        h.extend(num(self.block.1, 4));
        h.extend(num(self.block.0, 4));
        h.extend(num(self.nbpp, 2));
        h.extend(num(1, 3));
        h.extend(num(0, 3));
        h.extend(num(0, 10));
        h.extend(pad("1.0", 4));
        h.extend(extension_area(&self.udid, 0, 5));
        h.extend(extension_area(&self.ixshd, self.ixsofl, 5));
        h
    }

    /// The NITF 2.0 image subheader, with ICORDS "N" and IDATIM as DDhhmmssZMONYY.
    fn subheader_20(&self) -> Vec<u8> {
        let (nbpr, nbpc) = self.blocks();
        let mut h = Vec::new();
        h.extend(pad("IM", 2));
        h.extend(pad("OLD IMAGE", 10));
        h.extend(pad("14093015ZOCT97", 14));
        h.extend(pad("", 17));
        h.extend(pad("An old image", 80));
        h.extend(pad("U", 1));
        h.extend(pad("", 40 + 40 + 40 + 20 + 20));
        h.extend(pad("999998", 6));
        h.extend(pad("downgrade event", 40));
        h.extend(pad("0", 1));
        h.extend(pad("synthetic", 42));
        h.extend(num(self.rows, 8));
        h.extend(num(self.cols, 8));
        h.extend(pad(self.pvtype, 3));
        h.extend(pad("MONO", 8));
        h.extend(pad("VIS", 8));
        h.extend(num(self.abpp, 2));
        h.extend(pad(self.pjust, 1));
        h.extend(pad("N", 1));
        h.extend(pad("0", 1));
        h.extend(pad(self.ic, 2));
        h.extend(num(self.bands, 1));
        for _ in 0..self.bands {
            h.extend(pad("", 2 + 6));
            h.extend(pad("N", 1));
            h.extend(pad("", 3));
            h.extend(pad("0", 1));
        }
        h.extend(pad("0", 1));
        h.extend(pad(self.imode, 1));
        h.extend(num(nbpr, 4));
        h.extend(num(nbpc, 4));
        h.extend(num(self.block.1, 4));
        h.extend(num(self.block.0, 4));
        h.extend(num(self.nbpp, 2));
        h.extend(num(1, 3));
        h.extend(num(0, 3));
        h.extend(num(0, 10));
        h.extend(pad("1.0", 4));
        h.extend(extension_area(&self.udid, 0, 5));
        h.extend(extension_area(&self.ixshd, 0, 5));
        h
    }
}

/// A data extension segment: its subheader (NITF 2.1, TRE_OVERFLOW) and data.
fn overflow_des(item: u64, data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut h = Vec::new();
    h.extend(pad("DE", 2));
    h.extend(pad("TRE_OVERFLOW", 25));
    h.extend(num(1, 2));
    h.extend(pad("U", 1));
    h.extend(pad("", 166));
    h.extend(pad("UDID", 6));
    h.extend(num(item, 3));
    h.extend(num(0, 4));
    (h, data.to_vec())
}

struct File21 {
    images: Vec<Img>,
    des: Vec<(Vec<u8>, Vec<u8>)>,
    xhd: Vec<u8>,
}

impl File21 {
    fn bytes(&self) -> Vec<u8> {
        let subs: Vec<Vec<u8>> = self.images.iter().map(Img::subheader_21).collect();
        let mut tail = Vec::new();
        tail.extend(num(self.images.len() as u64, 3));
        for (s, i) in subs.iter().zip(&self.images) {
            tail.extend(num(s.len() as u64, 6));
            tail.extend(num(i.data.len() as u64, 10));
        }
        tail.extend(num(0, 3)); // NUMS
        tail.extend(num(0, 3)); // NUMX
        tail.extend(num(0, 3)); // NUMT
        tail.extend(num(self.des.len() as u64, 3));
        for (s, d) in &self.des {
            tail.extend(num(s.len() as u64, 4));
            tail.extend(num(d.len() as u64, 9));
        }
        tail.extend(num(0, 3)); // NUMRES
        tail.extend(num(0, 5)); // UDHDL
        tail.extend(extension_area(&self.xhd, 0, 5));
        let mut head = Vec::new();
        head.extend(pad("NITF", 4));
        head.extend(pad("02.10", 5));
        head.extend(pad("03", 2));
        head.extend(pad("BF01", 4));
        head.extend(pad("BIL TEST", 10));
        head.extend(pad("20261014093015", 14));
        head.extend(pad("A synthetic NITF file", 80));
        head.extend(pad("U", 1));
        head.extend(pad("", 166));
        head.extend(num(0, 5));
        head.extend(num(0, 5));
        head.extend(pad("0", 1));
        head.extend([0, 0, 0]);
        head.extend(pad("A. Person", 24));
        head.extend(pad("555 0100", 18));
        let hl = head.len() + 12 + 6 + tail.len();
        let total = hl
            + subs.iter().map(Vec::len).sum::<usize>()
            + self.images.iter().map(|i| i.data.len()).sum::<usize>()
            + self
                .des
                .iter()
                .map(|(s, d)| s.len() + d.len())
                .sum::<usize>();
        head.extend(num(total as u64, 12));
        head.extend(num(hl as u64, 6));
        head.extend(tail);
        for (s, i) in subs.iter().zip(&self.images) {
            head.extend(s);
            head.extend(&i.data);
        }
        for (s, d) in &self.des {
            head.extend(s);
            head.extend(d);
        }
        assert_eq!(head.len(), total);
        head
    }
}

/// A NITF 2.0 file of one image.
fn file_20(img: &Img) -> Vec<u8> {
    let sub = img.subheader_20();
    let mut head = Vec::new();
    head.extend(pad("NITF", 4));
    head.extend(pad("02.00", 5));
    head.extend(pad("01", 2));
    head.extend(pad("BF01", 4));
    head.extend(pad("BIL TEST", 10));
    head.extend(pad("14093015ZOCT97", 14));
    head.extend(pad("An old file", 80));
    head.extend(pad("U", 1));
    head.extend(pad("", 160));
    head.extend(pad("", 6)); // FSDWNG: no FSDEVT
    head.extend(num(0, 5));
    head.extend(num(0, 5));
    head.extend(pad("0", 1));
    head.extend(pad("A. Person", 27));
    head.extend(pad("", 18));
    let tail_len = 3 + 16 + 3 * 5 + 5 + 5;
    let hl = head.len() + 12 + 6 + tail_len;
    let total = hl + sub.len() + img.data.len();
    head.extend(num(total as u64, 12));
    head.extend(num(hl as u64, 6));
    head.extend(num(1, 3));
    head.extend(num(sub.len() as u64, 6));
    head.extend(num(img.data.len() as u64, 10));
    for _ in 0..5 {
        head.extend(num(0, 3));
    }
    head.extend(num(0, 5));
    head.extend(num(0, 5));
    assert_eq!(head.len(), hl);
    head.extend(sub);
    head.extend(&img.data);
    head
}

// ---------------------------------------------------------------- pixels known by construction

/// The value stored for band `b` at row `r` and column `c`, in `nbpp` bits.
fn value(r: u64, c: u64, b: u64, nbpp: u64) -> u64 {
    let v = (r.wrapping_mul(0x9E37_79B9)
        ^ c.wrapping_mul(0x85EB_CA6B)
        ^ (b + 1).wrapping_mul(0xC2B2_AE35))
    .wrapping_mul(0x2545_F491_4F6C_DD1D)
        >> 7;
    if nbpp == 64 {
        v
    } else {
        v & ((1u64 << nbpp) - 1)
    }
}

/// Appends `nbpp` bits of `v` to a bit stream, most significant first.
fn put_bits(bits: &mut Vec<bool>, v: u64, nbpp: u64) {
    for k in (0..nbpp).rev() {
        bits.push((v >> k) & 1 == 1);
    }
}

fn to_bytes(bits: &[bool]) -> Vec<u8> {
    bits.chunks(8)
        .map(|c| {
            c.iter()
                .enumerate()
                .fold(0u8, |b, (i, &x)| b | (u8::from(x) << (7 - i)))
        })
        .collect()
}

/// One block's stored bytes for `bands`, as IMODE says, with pad pixels where the block runs
/// past the Image.
fn block_bytes(img: &Img, brow: u64, bcol: u64, bands: &[u64]) -> Vec<u8> {
    let (bh, bw) = img.block;
    let sample = |r: u64, c: u64, b: u64| {
        let (rr, cc) = (brow * bh + r, bcol * bw + c);
        if rr < img.rows && cc < img.cols {
            value(rr, cc, b, img.nbpp)
        } else {
            0
        }
    };
    let mut out = Vec::new();
    match img.imode {
        "B" | "S" => {
            for &b in bands {
                let mut bits = Vec::new();
                for r in 0..bh {
                    for c in 0..bw {
                        put_bits(&mut bits, sample(r, c, b), img.nbpp);
                    }
                }
                out.extend(to_bytes(&bits));
            }
        }
        "P" => {
            let mut bits = Vec::new();
            for r in 0..bh {
                for c in 0..bw {
                    for &b in bands {
                        put_bits(&mut bits, sample(r, c, b), img.nbpp);
                    }
                }
            }
            out.extend(to_bytes(&bits));
        }
        _ => {
            let mut bits = Vec::new();
            for r in 0..bh {
                for &b in bands {
                    for c in 0..bw {
                        put_bits(&mut bits, sample(r, c, b), img.nbpp);
                    }
                }
            }
            out.extend(to_bytes(&bits));
        }
    }
    out
}

/// The stored blocks, in the standard's order: by block, or under IMODE S by band then block.
fn blocks_in_order(img: &Img) -> Vec<Vec<u8>> {
    let (nbpr, nbpc) = img.blocks();
    let all: Vec<u64> = (0..img.bands).collect();
    let mut v = Vec::new();
    if img.imode == "S" {
        for b in 0..img.bands {
            for br in 0..nbpc {
                for bc in 0..nbpr {
                    v.push(block_bytes(img, br, bc, &[b]));
                }
            }
        }
    } else {
        for br in 0..nbpc {
            for bc in 0..nbpr {
                v.push(block_bytes(img, br, bc, &all));
            }
        }
    }
    v
}

fn with_data(mut img: Img) -> Img {
    img.data = blocks_in_order(&img).concat();
    img
}

/// The native bytes BIL should deliver for one sample.
fn native(v: u64, t: SampleType) -> Vec<u8> {
    match t {
        SampleType::ComplexF32 => {
            let (re, im) = ((v >> 32) as u32, v as u32);
            [re.to_ne_bytes(), im.to_ne_bytes()].concat()
        }
        _ => {
            let n = t.size();
            let b = v.to_ne_bytes();
            if cfg!(target_endian = "little") {
                b[..n].to_vec()
            } else {
                b[8 - n..].to_vec()
            }
        }
    }
}

fn check_window(source: &NitfSource<Vec<u8>>, image: usize, img: &Img, w: Window) {
    let f = source.read_fragment(image, w).unwrap();
    let t = f.pixel_type.sample_type;
    for r in 0..u64::from(w.row_count) {
        for c in 0..u64::from(w.col_count) {
            for b in 0..img.bands {
                let at = r as usize * f.strides[0]
                    + c as usize * f.strides[1]
                    + b as usize * f.strides[2];
                let want = native(value(w.row_start + r, w.col_start + c, b, img.nbpp), t);
                assert_eq!(
                    &f.data[at..at + t.size()],
                    &want[..],
                    "{} {} bits, IMODE {}: row {}, column {}, band {b}",
                    img.pvtype,
                    img.nbpp,
                    img.imode,
                    w.row_start + r,
                    w.col_start + c
                );
            }
        }
    }
}

/// A small generator, for Windows and corruptions that can be made again.
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

// ---------------------------------------------------------------- the tests

#[test]
fn every_imode_and_sample_layout_reads_exactly() {
    let layouts = [
        ("INT", 8, SampleType::U8),
        ("INT", 16, SampleType::U16),
        ("INT", 12, SampleType::U16),
        ("B", 1, SampleType::U8),
        ("SI", 16, SampleType::I16),
        ("INT", 32, SampleType::U32),
        ("R", 32, SampleType::F32),
        ("R", 64, SampleType::F64),
        ("C", 64, SampleType::ComplexF32),
    ];
    let mut rng = Rng(7);
    for (pvtype, nbpp, sample_type) in layouts {
        for imode in ["B", "P", "R", "S"] {
            // 37 × 45 pixels in blocks of 16 × 16: partial blocks on the right and at the
            // bottom.
            let mut img = Img::new(37, 45, 3, pvtype, nbpp);
            img.imode = imode;
            img.block = (16, 16);
            let img = with_data(img);
            let file = File21 {
                images: vec![img.clone()],
                des: vec![],
                xhd: vec![],
            }
            .bytes();
            let source = NitfSource::open(file).unwrap();
            let info = source.image_info(0).unwrap();
            assert_eq!(info.pixel_type.unwrap().sample_type, sample_type);
            assert_eq!((info.rows, info.cols, info.bands), (37, 45, 3));
            assert_eq!(info.block_size, (16, 16));
            let whole = Window {
                row_start: 0,
                col_start: 0,
                row_count: 37,
                col_count: 45,
            };
            check_window(&source, 0, &img, whole);
            for _ in 0..20 {
                let row_start = rng.below(37);
                let col_start = rng.below(45);
                let w = Window {
                    row_start,
                    col_start,
                    row_count: 1 + rng.below(37 - row_start) as u32,
                    col_count: 1 + rng.below(45 - col_start) as u32,
                };
                check_window(&source, 0, &img, w);
            }
        }
    }
}

#[test]
fn significant_bits_and_justification() {
    let mut img = Img::new(4, 4, 1, "INT", 16);
    img.abpp = 11;
    img.pjust = "L";
    let file = File21 {
        images: vec![with_data(img)],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    let p = source.image_info(0).unwrap().pixel_type.unwrap();
    assert_eq!((p.significant_bits, p.bit_shift), (11, 5));
}

#[test]
fn many_bands_use_xbands() {
    let img = with_data(Img::new(3, 5, 12, "INT", 8));
    let file = File21 {
        images: vec![img.clone()],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    assert_eq!(source.image_info(0).unwrap().bands, 12);
    check_window(
        &source,
        0,
        &img,
        Window {
            row_start: 0,
            col_start: 0,
            row_count: 3,
            col_count: 5,
        },
    );
}

#[test]
fn storage_lists_blocks_in_file_order() {
    for imode in ["B", "S"] {
        let mut img = Img::new(20, 30, 2, "INT", 8);
        img.imode = imode;
        img.block = (10, 16);
        let file = File21 {
            images: vec![with_data(img)],
            des: vec![],
            xhd: vec![],
        }
        .bytes();
        let source = NitfSource::open(file).unwrap();
        let s = source.describe_storage(0).unwrap();
        assert_eq!(s.block_size, (10, 16));
        assert_eq!(s.interleave, Interleave::Band);
        assert_eq!(s.order, StorageOrder::Known);
        let data = source.image_info(0).unwrap().data;
        if imode == "B" {
            assert_eq!(s.blocks.len(), 4);
            assert!(s
                .blocks
                .iter()
                .all(|b| b.components == (0..2) && b.length == 320));
        } else {
            assert_eq!(s.blocks.len(), 8);
            assert_eq!(s.blocks[4].components, 1..2);
            assert!(s.blocks.iter().all(|b| b.length == 160));
        }
        assert_eq!(s.blocks[0].offset, data.0);
        assert!(s
            .blocks
            .windows(2)
            .all(|w| w[0].offset + w[0].length == w[1].offset));
        assert_eq!((s.blocks[1].block_row, s.blocks[1].block_col), (0, 1));
    }
}

/// A masked image: blocks stored out of order, one never recorded, and pad pixels.
#[test]
fn masks_mark_missing_blocks_and_pad_pixels() {
    let mut img = Img::new(20, 20, 2, "INT", 8);
    img.block = (10, 10);
    img.ic = "NM";
    let mut blocks = blocks_in_order(&img);
    // Pad pixels, 0 in every band, at the first block's first row.
    for c in 0..10 {
        blocks[0][c] = 0;
        blocks[0][100 + c] = 0;
    }
    // Block 1 is never recorded; the others are stored last first.
    let order = [3usize, 2, 0];
    let mut offsets = [u32::MAX; 4];
    let mut stored: Vec<u8> = Vec::new();
    for &k in &order {
        offsets[k] = stored.len() as u32;
        stored.extend(&blocks[k]);
    }
    let table_len = 10 + 1 + 4 * 4;
    let mut data = Vec::new();
    data.extend((table_len as u32).to_be_bytes());
    data.extend(4u16.to_be_bytes()); // BMRLNTH
    data.extend(0u16.to_be_bytes()); // TMRLNTH
    data.extend(8u16.to_be_bytes()); // TPXCDLNTH
    data.push(0); // TPXCD
    for o in offsets {
        data.extend(o.to_be_bytes());
    }
    data.extend(stored);
    img.data = data;
    let file = File21 {
        images: vec![img.clone()],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    let w = Window {
        row_start: 0,
        col_start: 0,
        row_count: 20,
        col_count: 20,
    };
    let f = source.read_fragment(0, w).unwrap();
    let valid = f.validity.unwrap();
    for r in 0..20u64 {
        for c in 0..20u64 {
            let block = (r / 10) * 2 + c / 10;
            // A pixel whose stored samples are all 0, by chance or as written, is a pad pixel.
            let zero = (r == 0 && c < 10) || (value(r, c, 0, 8) == 0 && value(r, c, 1, 8) == 0);
            let expect = block != 1 && !zero;
            assert_eq!(valid[(r * 20 + c) as usize], expect, "row {r}, column {c}");
            if expect {
                for b in 0..2 {
                    let at = (r * 40 + c * 2 + b) as usize;
                    assert_eq!(u64::from(f.data[at]), value(r, c, b, 8));
                }
            }
        }
    }
    let s = source.describe_storage(0).unwrap();
    assert_eq!(s.blocks.len(), 3);
    let at: Vec<(u32, u32)> = s
        .blocks
        .iter()
        .map(|b| (b.block_row, b.block_col))
        .collect();
    assert_eq!(at, [(1, 1), (1, 0), (0, 0)]);
}

#[test]
fn header_fields_through_their_definitions() {
    let rpc: Vec<u8> = {
        let mut d = b"1".to_vec();
        d.extend(b"0003.25"); // ERR_BIAS
        d.extend(b"0001.50"); // ERR_RAND
        d.extend(b"000512");
        d.extend(b"00640");
        d.extend(b"+38.8895");
        d.extend(b"-077.0352");
        d.extend(b"+0100");
        d.extend(b"000512");
        d.extend(b"00640");
        d.extend(b"+00.0500");
        d.extend(b"+000.0600");
        d.extend(b"+0500");
        for k in 0..80 {
            d.extend(format!("{:+012.6}", f64::from(k) / 10.0).as_bytes());
        }
        d
    };
    assert_eq!(rpc.len(), 1041);
    let mut img = with_data(Img::new(4, 4, 1, "INT", 8));
    img.ixshd = [
        tre("RPC00B", &rpc),
        tre("VNDRTA", b"abc"),
        tre("VNDRTA", b"defg"),
    ]
    .concat();
    let file = File21 {
        images: vec![img],
        des: vec![],
        xhd: tre("FILETA", b"xyz"),
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    assert_eq!(source.version(), Version::Nitf21);
    assert_eq!(
        source.list_field_groups(Scope::Source).unwrap(),
        ["nitf.file_header", "nitf.FILETA"]
    );
    assert_eq!(
        source.list_field_groups(Scope::Image(0)).unwrap(),
        [
            "nitf.image_subheader",
            "nitf.RPC00B",
            "nitf.VNDRTA[1]",
            "nitf.VNDRTA[2]"
        ]
    );
    let title = source
        .read_field(Scope::Source, "nitf.file_header.FTITLE")
        .unwrap();
    assert_eq!(title.value(), &Value::Text("A synthetic NITF file".into()));
    assert_eq!(title.definition.unwrap().provides, Some("common.title"));
    let class = source
        .read_field(Scope::Source, "nitf.file_header.FSCLAS")
        .unwrap();
    assert_eq!(class.handling(), Handling::Marking);
    assert_eq!(
        source
            .read_field(Scope::Source, "nitf.file_header.ONAME")
            .unwrap()
            .handling(),
        Handling::Identity
    );
    let Value::DateTime(t) = source
        .read_field(Scope::Image(0), "nitf.image_subheader.IDATIM")
        .unwrap()
        .value()
        .clone()
    else {
        panic!()
    };
    assert_eq!(
        (t.year, t.month, t.day, t.hour, t.minute),
        (2026, 10, 14, 9, 30)
    );
    // Repeats carry their number; the first band's lookup table is a run of its own.
    let lut = source
        .read_field(Scope::Image(0), "nitf.image_subheader.LUTD[1][1]")
        .unwrap();
    assert_eq!(lut.value(), &Value::Bytes(vec![1, 2, 3, 4]));
    assert_eq!(
        source
            .read_field(Scope::Image(0), "nitf.image_subheader.ICOM[1]")
            .unwrap()
            .value(),
        &Value::Text("one comment".into())
    );
    // A defined extension, with units; an undefined one as its bytes.
    let g = source
        .read_field_group(Scope::Image(0), "nitf.RPC00B")
        .unwrap();
    assert_eq!(g.status, Status::Complete);
    let lat = g.get("LAT_OFF").unwrap();
    assert_eq!(lat.value(), &Value::Float(38.8895));
    assert_eq!(lat.unit(), Some("deg"));
    assert_eq!(g.get("LINE_OFF").unwrap().value(), &Value::Integer(512));
    assert!(g.get("SAMP_DEN_COEFF[20]").is_some());
    let v = source
        .read_field(Scope::Image(0), "nitf.VNDRTA[2].CEDATA")
        .unwrap();
    assert_eq!(v.value(), &Value::Bytes(b"defg".to_vec()));
    // Unknown names are errors.
    assert!(matches!(
        source.read_field(Scope::Image(0), "nitf.image_subheader.NOSUCH"),
        Err(Error::NotFound(_))
    ));
    // A held group is the same object until released.
    let a = source
        .read_field_group(Scope::Image(0), "nitf.RPC00B")
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&a, &g));
    source.release_metadata(Scope::Image(0), "nitf.RPC00B");
    let b = source
        .read_field_group(Scope::Image(0), "nitf.RPC00B")
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&a, &b));
}

#[test]
fn extensions_that_overflow_into_a_data_extension_segment() {
    let mut img = with_data(Img::new(4, 4, 1, "INT", 8));
    img.ixshd = tre("FIRST", b"1");
    img.ixsofl = 1;
    let file = File21 {
        images: vec![img],
        des: vec![overflow_des(
            1,
            &[tre("SECOND", b"22"), tre("THIRD", b"333")].concat(),
        )],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    assert_eq!(
        source.list_field_groups(Scope::Image(0)).unwrap(),
        [
            "nitf.image_subheader",
            "nitf.FIRST",
            "nitf.SECOND",
            "nitf.THIRD"
        ]
    );
    assert_eq!(
        source
            .read_field(Scope::Image(0), "nitf.THIRD.CEDATA")
            .unwrap()
            .value(),
        &Value::Bytes(b"333".to_vec())
    );
    assert_eq!(source.segments()[1].kind, SegmentKind::DataExtension);
}

/// Bytes that record every read made of them.
struct Recorded {
    bytes: Vec<u8>,
    reads: Mutex<Vec<(u64, usize)>>,
}

impl ReadAt for Recorded {
    fn size(&self) -> std::io::Result<u64> {
        self.bytes.size()
    }
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        self.reads.lock().unwrap().push((offset, buf.len()));
        self.bytes.read_exact_at(buf, offset)
    }
}

#[test]
fn opening_reads_only_what_describes_the_images() {
    let mut img = with_data(Img::new(4, 4, 1, "INT", 8));
    img.ixshd = tre("FIRST", b"1");
    img.ixsofl = 1;
    let file = File21 {
        images: vec![img],
        des: vec![overflow_des(1, &tre("SECOND", b"22"))],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(Recorded {
        bytes: file,
        reads: Mutex::new(Vec::new()),
    })
    .unwrap();
    let des = source.segments()[1];
    let (start, end) = (des.subheader.0, des.data.0 + des.data.1);
    let touches = |reads: &[(u64, usize)]| {
        reads
            .iter()
            .any(|&(at, len)| at < end && at + len as u64 > start)
    };
    let data = source.image_info(0).unwrap().data;
    let reads_pixels = |reads: &[(u64, usize)]| {
        reads
            .iter()
            .any(|&(at, len)| at < data.0 + data.1 && at + len as u64 > data.0)
    };
    // Opening: no overflow segment, no pixels (section 14.3, rule 1).
    {
        let reads = source_reads(&source);
        assert!(!touches(&reads) && !reads_pixels(&reads), "{reads:?}");
    }
    // Asking for the image's groups reads the overflow segment.
    assert_eq!(
        source.list_field_groups(Scope::Image(0)).unwrap(),
        ["nitf.image_subheader", "nitf.FIRST", "nitf.SECOND"]
    );
    assert!(touches(&source_reads(&source)));
}

fn source_reads(source: &NitfSource<Recorded>) -> Vec<(u64, usize)> {
    source.reader().reads.lock().unwrap().clone()
}

#[test]
fn a_defined_field_the_file_leaves_out_is_missing() {
    let file = File21 {
        images: vec![with_data(Img::new(4, 4, 1, "INT", 8))],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    // ICORDS is blank, so IGEOLO is not in the file; COMRAT is only for compressed data.
    for name in ["IGEOLO", "COMRAT", "ICOM[2]"] {
        let f = source
            .read_field(Scope::Image(0), &format!("nitf.image_subheader.{name}"))
            .unwrap();
        assert_eq!(f.value(), &Value::Missing, "{name}");
        assert!(f.definition.is_some());
    }
    // A defined name with the wrong count of repeat numbers is unknown.
    for name in [
        "nitf.image_subheader.NOSUCH",
        "nitf.image_subheader.IGEOLO[2]",
        "nitf.image_subheader.ICOM",
        "nitf.image_subheader.LUTD[1]",
        "nitf.image_subheader.ICOM[x]",
    ] {
        assert!(
            matches!(
                source.read_field(Scope::Image(0), name),
                Err(Error::NotFound(_))
            ),
            "{name}"
        );
    }
    assert!(matches!(
        source.read_field(Scope::Source, "nitf.file_header.FTITLE[2]"),
        Err(Error::NotFound(_))
    ));
}

/// Found by the checking agent: blocks so large that their size in bits overflowed.
#[test]
fn blocks_too_large_to_count_are_corrupt() {
    let mut img = Img::new(16, 16, 30, "R", 64);
    img.imode = "P";
    let img = with_data(img);
    let mut file = File21 {
        images: vec![img],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let replace = |d: &mut Vec<u8>, from: &[u8], to: &[u8]| {
        let at = d.windows(from.len()).position(|w| w == from).unwrap();
        d[at..at + to.len()].copy_from_slice(to);
    };
    // NROWS and NCOLS 99,999,999, in one block whose size of 0 means the whole Image.
    replace(&mut file, b"0000001600000016", b"9999999999999999");
    replace(&mut file, b"0001000100160016", b"0001000100000000");
    assert!(matches!(NitfSource::open(file), Err(Error::Corrupt(_))));
}

/// Found by the checking agent: only spaces pad a tag, so a tag padded with a tab is damage.
#[test]
fn a_tag_padded_with_anything_but_spaces_is_refused() {
    let mut img = with_data(Img::new(4, 4, 1, "INT", 8));
    img.ixshd = [tre("GOOD", b"ok"), b"ABC\t\t\t00002xy".to_vec()].concat();
    let file = File21 {
        images: vec![img],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    assert_eq!(
        source.list_field_groups(Scope::Image(0)).unwrap(),
        ["nitf.image_subheader", "nitf.GOOD"]
    );
    let g = source
        .read_field_group(Scope::Image(0), "nitf.image_subheader")
        .unwrap();
    assert_eq!(g.status, Status::Partial);
}

#[test]
fn a_damaged_extension_area_leaves_the_rest_readable() {
    let mut img = with_data(Img::new(4, 4, 1, "INT", 8));
    // The second extension's length is not a number.
    let mut area = tre("GOOD", b"ok");
    area.extend(b"BAD   x1234");
    img.ixshd = area;
    let file = File21 {
        images: vec![img.clone()],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    assert_eq!(
        source.list_field_groups(Scope::Image(0)).unwrap(),
        ["nitf.image_subheader", "nitf.GOOD"]
    );
    let g = source
        .read_field_group(Scope::Image(0), "nitf.image_subheader")
        .unwrap();
    assert_eq!(g.status, Status::Partial);
    assert!(g.problem.as_ref().unwrap().contains("BAD"));
    // The pixels do not depend on it.
    check_window(
        &source,
        0,
        &img,
        Window {
            row_start: 0,
            col_start: 0,
            row_count: 4,
            col_count: 4,
        },
    );
}

#[test]
fn nitf_20_files() {
    let mut img = Img::new(9, 7, 1, "INT", 16);
    img.block = (4, 4);
    let img = with_data(img);
    let source = NitfSource::open(file_20(&img)).unwrap();
    assert_eq!(source.version(), Version::Nitf20);
    // ICORDS "N": no IGEOLO. The downgrade event is present after 999998.
    let g = source
        .read_field_group(Scope::Image(0), "nitf.image_subheader")
        .unwrap();
    assert!(g.get("IGEOLO").is_none());
    assert_eq!(
        g.get("ISDEVT").unwrap().value(),
        &Value::Text("downgrade event".into())
    );
    let Value::DateTime(t) = g.get("IDATIM").unwrap().value().clone() else {
        panic!()
    };
    assert_eq!((t.year, t.month, t.day), (1997, 10, 14));
    check_window(
        &source,
        0,
        &img,
        Window {
            row_start: 0,
            col_start: 0,
            row_count: 9,
            col_count: 7,
        },
    );
}

#[test]
fn several_images() {
    let a = with_data(Img::new(5, 6, 1, "INT", 8));
    let mut b = Img::new(7, 3, 2, "INT", 16);
    b.imode = "P";
    let b = with_data(b);
    let file = File21 {
        images: vec![a.clone(), b.clone()],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    assert_eq!(source.image_count(), 2);
    for (i, img) in [&a, &b].into_iter().enumerate() {
        check_window(
            &source,
            i,
            img,
            Window {
                row_start: 0,
                col_start: 0,
                row_count: img.rows as u32,
                col_count: img.cols as u32,
            },
        );
    }
    assert!(source.image_info(2).is_err());
}

#[test]
fn compressed_blocks_are_left_to_a_codec() {
    let mut img = Img::new(8, 8, 1, "INT", 8);
    img.ic = "C8";
    img.data = vec![0xFF, 0x4F, 0xFF, 0x51, 0, 0];
    let file = File21 {
        images: vec![img],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    let info = source.image_info(0).unwrap();
    assert_eq!(info.codec.as_deref(), Some("jpeg2000"));
    let s = source.describe_storage(0).unwrap();
    assert_eq!(s.blocks.len(), 1);
    assert_eq!((s.blocks[0].offset, s.blocks[0].length), info.data);
    let read = source.read_fragment(
        0,
        Window {
            row_start: 0,
            col_start: 0,
            row_count: 1,
            col_count: 1,
        },
    );
    // A codestream cut short: an error with the codec, unsupported without it.
    if cfg!(feature = "openjpeg") {
        assert!(matches!(read, Err(Error::Corrupt(_))));
    } else {
        assert!(matches!(read, Err(Error::Unsupported(_))));
    }
}

#[test]
fn claims_beyond_the_file_are_refused() {
    // Uncompressed data that claims more pixels than the file holds (section 10.3).
    let mut img = with_data(Img::new(16, 16, 1, "INT", 8));
    img.data.truncate(200);
    let file = File21 {
        images: vec![img],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    assert!(matches!(NitfSource::open(file), Err(Error::Corrupt(_))));
    // A Window outside the Image.
    let file = File21 {
        images: vec![with_data(Img::new(4, 4, 1, "INT", 8))],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = NitfSource::open(file).unwrap();
    let w = Window {
        row_start: 3,
        col_start: 0,
        row_count: 2,
        col_count: 4,
    };
    assert!(matches!(
        source.read_fragment(0, w),
        Err(Error::NotFound(_))
    ));
    // Not NITF.
    assert!(matches!(
        NitfSource::open(b"GIF89a....".to_vec()),
        Err(Error::Unrecognized(_))
    ));
}

/// Seeded corruption of valid files, as experiment 3 does to C libraries: every corrupt file
/// opens or is refused, and every read delivers or fails, without a panic. Section 10.3 asks for
/// coverage-guided fuzzing of BIL's own readers before release; this is the check that runs
/// everywhere.
#[test]
fn corrupt_files_are_errors_never_panics() {
    let mut masked = Img::new(20, 24, 2, "INT", 12);
    masked.block = (8, 8);
    masked.imode = "S";
    let masked_data = {
        let blocks = blocks_in_order(&masked);
        let n = blocks.len();
        let mut d = Vec::new();
        d.extend(((10 + 2 + 4 * n) as u32).to_be_bytes());
        d.extend(4u16.to_be_bytes());
        d.extend(0u16.to_be_bytes());
        d.extend(12u16.to_be_bytes());
        d.extend([0, 0]);
        let mut at = 0u32;
        for b in &blocks {
            d.extend(at.to_be_bytes());
            at += b.len() as u32;
        }
        d.extend(blocks.concat());
        d
    };
    masked.ic = "NM";
    masked.data = masked_data;
    let mut plain = Img::new(13, 17, 3, "INT", 8);
    plain.block = (8, 8);
    plain.imode = "P";
    plain.ixshd = [tre("RPC00B", &[b'0'; 1041]), tre("OTHER", b"zz")].concat();
    let seeds = [
        File21 {
            images: vec![masked, with_data(plain)],
            des: vec![overflow_des(1, &tre("MORE", b"m"))],
            xhd: tre("FILETA", b"x"),
        }
        .bytes(),
        file_20(&with_data(Img::new(6, 6, 1, "INT", 1))),
    ];
    let mut rng = Rng(2026);
    let (mut opened, mut refused, mut read) = (0, 0, 0);
    // 20,000 by default; BIL_CORRUPT_FILES sets more for a longer run.
    let count: u64 = std::env::var("BIL_CORRUPT_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    for _ in 0..count {
        let mut d = seeds[rng.below(2) as usize].clone();
        for _ in 0..1 + rng.below(4) {
            if d.is_empty() {
                d.push(b'N');
            }
            let len = d.len() as u64;
            let at = rng.below(len) as usize;
            match rng.below(6) {
                0 => d[at] ^= 1 << rng.below(8),
                1 => d[at] = rng.next() as u8,
                // NITF's numbers are digits: change one, or fill a run with nines or zeros.
                2 | 3 => {
                    let digits: Vec<usize> =
                        (0..d.len()).filter(|&i| d[i].is_ascii_digit()).collect();
                    if !digits.is_empty() {
                        let i = digits[rng.below(digits.len() as u64) as usize];
                        if rng.below(2) == 0 {
                            d[i] = b'0' + rng.below(10) as u8;
                        } else {
                            let fill = if rng.below(2) == 0 { b'9' } else { b'0' };
                            let mut j = i;
                            while j < d.len() && d[j].is_ascii_digit() {
                                d[j] = fill;
                                j += 1;
                            }
                        }
                    }
                }
                4 => d.truncate(at),
                _ => {
                    let n = rng.below(32) as usize;
                    let run: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                    d.splice(at..at, run);
                }
            }
        }
        let size = d.len();
        let Ok(source) = NitfSource::open(d) else {
            refused += 1;
            continue;
        };
        opened += 1;
        for scope in
            std::iter::once(Scope::Source).chain((0..source.image_count()).map(Scope::Image))
        {
            for g in source.list_field_groups(scope).unwrap() {
                if let Ok(group) = source.read_field_group(scope, &g) {
                    for f in &group.fields {
                        let _ = f.value();
                    }
                }
            }
        }
        for i in 0..source.image_count() {
            let info = source.image_info(i).unwrap().clone();
            let _ = source.describe_storage(i);
            // Only Windows small enough to make: the reader allocates what the Window asks.
            let bytes = u128::from(info.rows) * u128::from(info.cols) * u128::from(info.bands) * 8;
            if bytes <= 64 * size as u128 {
                let w = Window {
                    row_start: 0,
                    col_start: 0,
                    row_count: info.rows as u32,
                    col_count: info.cols as u32,
                };
                if source.read_fragment(i, w).is_ok() {
                    read += 1;
                }
            }
        }
    }
    eprintln!("corrupt files: {opened} opened, {refused} refused, {read} Images read whole");
    assert!(
        opened > 1000 && refused > 1000,
        "the mutations should both break and spare files"
    );
}

// ---------------------------------------------------------------- JPEG 2000 images

fn jpeg2000_fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/jpeg2000/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The samples `tests/data/jpeg2000/make_fixtures.c` wrote.
#[cfg(feature = "openjpeg")]
fn jpeg2000_value(x: i64, y: i64, c: i64, prec: u32, signed: bool) -> i64 {
    let m = 1i64 << prec;
    let v = (x * 7 + y * 13 + c * 31 + x * y) % m;
    if signed {
        v - m / 2
    } else {
        v
    }
}

fn jpeg2000_image(
    fixture: &str,
    rows: u64,
    cols: u64,
    bands: u64,
    pvtype: &'static str,
    nbpp: u64,
) -> Img {
    let mut img = Img::new(rows, cols, bands, pvtype, nbpp);
    img.ic = "C8";
    img.imode = "B";
    img.data = jpeg2000_fixture(fixture);
    img
}

fn open_one(img: Img) -> NitfSource<Vec<u8>> {
    let file = File21 {
        images: vec![img],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    NitfSource::open(file).unwrap()
}

#[cfg(feature = "openjpeg")]
#[test]
fn jpeg2000_images_decode_through_windows() {
    let cases = [
        ("gray8.j2k", 29, 37, 1, "INT", 8, SampleType::U8, 8, false),
        ("gray8.jp2", 29, 37, 1, "INT", 8, SampleType::U8, 8, false),
        ("rgb8.j2k", 19, 23, 3, "INT", 8, SampleType::U8, 8, false),
        (
            "gray16.j2k",
            17,
            21,
            1,
            "INT",
            16,
            SampleType::U16,
            16,
            false,
        ),
        (
            "signed12.j2k",
            18,
            20,
            1,
            "SI",
            12,
            SampleType::I16,
            12,
            true,
        ),
    ];
    for (name, rows, cols, bands, pvtype, nbpp, t, prec, signed) in cases {
        let mut img = jpeg2000_image(name, rows, cols, bands, pvtype, nbpp);
        // The NITF blocks describe the codestream's 16 × 16 tiles where it has them.
        if name.starts_with("gray8") {
            img.block = (16, 16);
        }
        let source = open_one(img);
        assert_eq!(
            source.image_info(0).unwrap().codec.as_deref(),
            Some("jpeg2000")
        );
        let windows = [
            Window {
                row_start: 0,
                col_start: 0,
                row_count: rows as u32,
                col_count: cols as u32,
            },
            Window {
                row_start: 3,
                col_start: 5,
                row_count: 11,
                col_count: 13,
            },
            Window {
                row_start: rows - 1,
                col_start: cols - 1,
                row_count: 1,
                col_count: 1,
            },
        ];
        for w in windows {
            let f = source.read_fragment(0, w).unwrap();
            assert_eq!(f.pixel_type.sample_type, t, "{name}");
            assert_eq!(f.pixel_type.bit_shift, 0);
            assert!(f.validity.is_none());
            for r in 0..u64::from(w.row_count) {
                for c in 0..u64::from(w.col_count) {
                    for b in 0..bands {
                        let at = r as usize * f.strides[0]
                            + c as usize * f.strides[1]
                            + b as usize * f.strides[2];
                        let got = match t {
                            SampleType::U8 => i64::from(f.data[at]),
                            SampleType::U16 => {
                                i64::from(u16::from_ne_bytes([f.data[at], f.data[at + 1]]))
                            }
                            SampleType::I16 => {
                                i64::from(i16::from_ne_bytes([f.data[at], f.data[at + 1]]))
                            }
                            _ => unreachable!(),
                        };
                        let (x, y) = ((w.col_start + c) as i64, (w.row_start + r) as i64);
                        assert_eq!(
                            got,
                            jpeg2000_value(x, y, b as i64, prec, signed),
                            "{name} {x},{y},{b}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(feature = "openjpeg")]
#[test]
fn jpeg2000_that_disagrees_with_its_subheader_is_an_error() {
    // Size, bands and signedness each differ from the codestream's.
    let cases = [
        jpeg2000_image("gray8.j2k", 29, 36, 1, "INT", 8),
        jpeg2000_image("rgb8.j2k", 19, 23, 1, "INT", 8),
        jpeg2000_image("gray16.j2k", 17, 21, 1, "INT", 8),
        jpeg2000_image("signed12.j2k", 18, 20, 1, "INT", 12),
    ];
    for img in cases {
        let source = open_one(img);
        let w = Window {
            row_start: 0,
            col_start: 0,
            row_count: 1,
            col_count: 1,
        };
        let err = source.read_fragment(0, w).err().unwrap();
        assert!(matches!(err, Error::Corrupt(_)), "{err}");
    }
    let mut img = jpeg2000_image("gray8.j2k", 29, 37, 1, "INT", 8);
    img.data[0] = 0;
    let source = open_one(img);
    let w = Window {
        row_start: 0,
        col_start: 0,
        row_count: 1,
        col_count: 1,
    };
    assert!(matches!(source.read_fragment(0, w), Err(Error::Corrupt(_))));
}

/// Without a codec, a compressed Image's pixels are not read, and the error says why.
#[test]
fn compressed_images_without_their_codec_are_unsupported() {
    let mut img = jpeg2000_image("gray8.j2k", 29, 37, 1, "INT", 8);
    img.ic = "C3";
    let source = open_one(img);
    let w = Window {
        row_start: 0,
        col_start: 0,
        row_count: 1,
        col_count: 1,
    };
    assert!(matches!(
        source.read_fragment(0, w),
        Err(Error::Unsupported(_))
    ));
    #[cfg(not(feature = "openjpeg"))]
    {
        let source = open_one(jpeg2000_image("gray8.j2k", 29, 37, 1, "INT", 8));
        assert!(matches!(
            source.read_fragment(0, w),
            Err(Error::Unsupported(_))
        ));
    }
}

/// A JPEG 2000 Image behind a mask table of one block, with a pad value: every pixel whose
/// sample is 0 is a pad pixel.
#[cfg(feature = "openjpeg")]
#[test]
fn jpeg2000_behind_a_mask_table() {
    let code = jpeg2000_fixture("gray8.j2k");
    let masked = |offset: u32| {
        let mut img = jpeg2000_image("gray8.j2k", 29, 37, 1, "INT", 8);
        img.ic = "M8";
        let mut data = Vec::new();
        data.extend(15u32.to_be_bytes()); // IMDATOFF
        data.extend(4u16.to_be_bytes()); // BMRLNTH
        data.extend(0u16.to_be_bytes()); // TMRLNTH
        data.extend(8u16.to_be_bytes()); // TPXCDLNTH
        data.push(0); // TPXCD
        data.extend(offset.to_be_bytes());
        data.extend(&code);
        img.data = data;
        open_one(img)
    };
    let w = Window {
        row_start: 0,
        col_start: 0,
        row_count: 29,
        col_count: 37,
    };
    let f = masked(0).read_fragment(0, w).unwrap();
    let valid = f.validity.unwrap();
    for y in 0..29i64 {
        for x in 0..37i64 {
            let v = jpeg2000_value(x, y, 0, 8, false);
            assert_eq!(i64::from(f.data[(y * 37 + x) as usize]), v);
            assert_eq!(valid[(y * 37 + x) as usize], v != 0, "{x},{y}");
        }
    }
    // The only block never recorded: every pixel missing.
    let f = masked(u32::MAX).read_fragment(0, w).unwrap();
    assert!(f.validity.unwrap().iter().all(|&v| !v));
}

#[cfg(feature = "openjpeg")]
#[test]
fn jpeg2000_limits_are_stated() {
    let w = Window {
        row_start: 0,
        col_start: 0,
        row_count: 1,
        col_count: 1,
    };
    // A mask table placing several blocks (M34).
    let mut img = jpeg2000_image("gray8.j2k", 29, 37, 1, "INT", 8);
    img.ic = "M8";
    img.block = (16, 16);
    let code = img.data.clone();
    let mut data = Vec::new();
    data.extend((10u32 + 4 * 6).to_be_bytes());
    data.extend(4u16.to_be_bytes());
    data.extend(0u16.to_be_bytes());
    data.extend(0u16.to_be_bytes());
    for _ in 0..6 {
        data.extend(0u32.to_be_bytes());
    }
    data.extend(code);
    img.data = data;
    assert!(matches!(
        open_one(img).read_fragment(0, w),
        Err(Error::Unsupported(_))
    ));
    // A JP2 file with channel definitions, which are neither applied nor reported yet.
    let mut img = jpeg2000_image("gray8.jp2", 29, 37, 1, "INT", 8);
    img.data = jp2_with_cdef(&img.data);
    assert!(matches!(
        open_one(img).read_fragment(0, w),
        Err(Error::Unsupported(_))
    ));
    // An empty Window reads nothing.
    let source = open_one(jpeg2000_image("gray8.j2k", 29, 37, 1, "INT", 8));
    let empty = Window {
        row_start: 5,
        col_start: 5,
        row_count: 0,
        col_count: 3,
    };
    assert!(source.read_fragment(0, empty).unwrap().data.is_empty());
}

/// A JP2 file with a channel definition box added to its header box.
#[cfg(feature = "openjpeg")]
fn jp2_with_cdef(jp2: &[u8]) -> Vec<u8> {
    let at = jp2.windows(4).position(|w| w == b"jp2h").unwrap() - 4;
    let len = u32::from_be_bytes(jp2[at..at + 4].try_into().unwrap()) as usize;
    let cdef = [0, 0, 0, 16, b'c', b'd', b'e', b'f', 0, 1, 0, 0, 0, 0, 0, 1];
    let mut out = jp2[..at].to_vec();
    out.extend(((len + cdef.len()) as u32).to_be_bytes());
    out.extend(&jp2[at + 4..at + len]);
    out.extend(cdef);
    out.extend(&jp2[at + len..]);
    out
}

/// A JPEG 2000 Image's resolution levels are its stored levels; a Window of one reads the same
/// samples as the whole level does there (section 5; M46, M48).
#[cfg(feature = "openjpeg")]
#[test]
fn jpeg2000_resolution_levels_are_stored_levels() {
    for (name, block) in [
        ("gray8.j2k", (16, 16)),
        ("gray8.jp2", (16, 16)),
        ("rgb8.j2k", (19, 23)),
    ] {
        let bands = if name.starts_with("rgb") { 3 } else { 1 };
        let (rows, cols) = if bands == 3 { (19, 23) } else { (29, 37) };
        let mut img = jpeg2000_image(name, rows, cols, bands, "INT", 8);
        img.block = block;
        let source = open_one(img);
        let levels = &source.image_info(0).unwrap().stored_levels;
        // make_fixtures.c writes three resolutions: two levels below full resolution.
        let sizes: Vec<_> = levels
            .iter()
            .map(|l| (l.name.as_str(), l.rows, l.cols))
            .collect();
        assert_eq!(
            sizes,
            [
                ("stored1", rows.div_ceil(2), cols.div_ceil(2)),
                ("stored2", rows.div_ceil(4), cols.div_ceil(4))
            ],
            "{name}"
        );
        let all = Window {
            row_start: 0,
            col_start: 0,
            row_count: rows as u32,
            col_count: cols as u32,
        };
        for (k, level) in levels.iter().enumerate() {
            // Aligned, by the ruling on M48; the same blocks as level0, marked with the level.
            assert_eq!(level.aligned, Some(true), "{name}");
            let storage = source.describe_level_storage(0, &level.name).unwrap();
            let s = 1u32 << (k + 1);
            assert_eq!(
                storage.block_size,
                ((block.0 as u32).div_ceil(s), (block.1 as u32).div_ceil(s)),
                "{name}"
            );
            let level0 = source.describe_storage(0).unwrap();
            assert_eq!(storage.blocks.len(), level0.blocks.len());
            for (a, b) in storage.blocks.iter().zip(&level0.blocks) {
                assert_eq!(
                    (a.level, a.offset, a.length),
                    (k as u32 + 1, b.offset, b.length)
                );
            }
            let whole = source.read_level_fragment(0, &level.name, all).unwrap();
            assert_eq!(
                (whole.window.row_count, whole.window.col_count),
                (level.rows as u32, level.cols as u32)
            );
            // Rows 5 to 16 and columns 3 to 14 hold the samples at multiples of 2^k there.
            let w = Window {
                row_start: 5,
                col_start: 3,
                row_count: 12,
                col_count: 12,
            };
            let part = source.read_level_fragment(0, &level.name, w).unwrap();
            let s = 1u64 << (k + 1);
            assert_eq!(part.window.row_start, 5u64.div_ceil(s), "{name}");
            assert_eq!(part.window.col_start, 3u64.div_ceil(s), "{name}");
            assert_eq!(
                u64::from(part.window.row_count),
                17u64.div_ceil(s) - 5u64.div_ceil(s)
            );
            for r in 0..part.window.row_count as usize {
                for c in 0..part.window.col_count as usize {
                    let at = r * part.strides[0] + c * part.strides[1];
                    let from = (part.window.row_start as usize + r) * whole.strides[0]
                        + (part.window.col_start as usize + c) * whole.strides[1];
                    assert_eq!(
                        part.data[at..at + part.strides[1]],
                        whole.data[from..from + whole.strides[1]],
                        "{name} {} {r} {c}",
                        level.name
                    );
                }
            }
        }
        assert!(source.read_level_fragment(0, "stored3", all).is_err());
    }
}

#[test]
fn the_source_interface_opens_nitf_and_names_what_it_tried() {
    let a = with_data(Img::new(5, 6, 1, "INT", 8));
    let b = with_data(Img::new(7, 3, 2, "INT", 16));
    let file = File21 {
        images: vec![a, b],
        des: vec![],
        xhd: vec![],
    }
    .bytes();
    let source = bil::OpenOptions::default().open_from(file).unwrap();
    assert_eq!((source.driver(), source.image_count()), ("nitf", 2));
    let image = source.image(1).unwrap();
    assert_eq!(image.size().unwrap(), (7, 3));
    assert_eq!(image.component_count().unwrap(), 2);
    assert!(image.list_stored_levels().unwrap().is_empty());
    let w = Window {
        row_start: 0,
        col_start: 0,
        row_count: 7,
        col_count: 3,
    };
    let f = image.read_fragment(w, "level0").unwrap();
    assert_eq!(f.data.len(), 7 * 3 * 2 * 2);
    assert_eq!(f, source.as_nitf().unwrap().read_fragment(1, w).unwrap());
    assert!(matches!(
        image.read_fragment(w, "stored1"),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        image.describe_storage("stored1"),
        Err(Error::NotFound(_))
    ));
    assert!(source.as_geotiff().is_none());
    // Bytes no driver recognizes: the error names every driver tried.
    match bil::OpenOptions::default().open_from(b"GIF89a....".to_vec()) {
        Err(Error::Unrecognized(tried)) => assert_eq!(tried, ["nitf", "geotiff"]),
        other => panic!("{:?}", other.err()),
    }
}
