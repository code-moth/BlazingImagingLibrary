// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Tests of JPEG 2000 decoding through OpenJPEG, on the codestreams in `tests/data/jpeg2000`.

#![cfg(feature = "openjpeg")]

use bil::codec::jpeg2000::{decode, read_header, Error, Format, Header, Request};
use bil::io::ReadAt;

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/jpeg2000/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The samples `make_fixtures.c` wrote.
fn value(x: i64, y: i64, c: i64, prec: u32, signed: bool) -> i32 {
    let m = 1i64 << prec;
    let v = (x * 7 + y * 13 + c * 31 + x * y) % m;
    (if signed { v - m / 2 } else { v }) as i32
}

fn accept(_: &Header) -> Result<(), String> {
    Ok(())
}

fn check_whole(name: &str, format: Format, w: u32, h: u32, nc: usize, prec: u32, signed: bool) {
    let bytes = fixture(name);
    let (header, image) = decode(
        &bytes,
        0,
        bytes.len() as u64,
        format,
        Request::default(),
        &accept,
    )
    .unwrap();
    assert_eq!(
        (header.width(), header.height()),
        (u64::from(w), u64::from(h))
    );
    assert_eq!(header.components.len(), nc);
    assert_eq!(image.component_count(), nc);
    for c in 0..nc {
        let comp = image.component(c).unwrap();
        assert_eq!(
            (comp.width, comp.height, comp.precision, comp.signed),
            (w, h, prec, signed)
        );
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let got = comp.data[(y * w as i64 + x) as usize];
                assert_eq!(
                    got,
                    value(x, y, c as i64, prec, signed),
                    "{name} {c} {x},{y}"
                );
            }
        }
    }
}

#[test]
fn every_fixture_decodes_without_loss() {
    check_whole("gray8.j2k", Format::Codestream, 37, 29, 1, 8, false);
    check_whole("gray8.jp2", Format::Jp2, 37, 29, 1, 8, false);
    check_whole("rgb8.j2k", Format::Codestream, 23, 19, 3, 8, false);
    check_whole("gray16.j2k", Format::Codestream, 21, 17, 1, 16, false);
    check_whole("signed12.j2k", Format::Codestream, 20, 18, 1, 12, true);
}

#[test]
fn a_codestream_is_read_where_it_lies() {
    let mut bytes = vec![0xAAu8; 123];
    let code = fixture("gray8.j2k");
    bytes.extend_from_slice(&code);
    bytes.extend_from_slice(&[0x55; 77]);
    let (_, image) = decode(
        &bytes,
        123,
        code.len() as u64,
        Format::Codestream,
        Request::default(),
        &accept,
    )
    .unwrap();
    assert_eq!(
        image.component(0).unwrap().data[40],
        value(3, 1, 0, 8, false)
    );
}

#[test]
fn an_area_decodes_only_that_area() {
    let bytes = fixture("gray8.j2k");
    let request = Request {
        area: Some((5, 7, 30, 20)),
        reduce: 0,
    };
    let (_, image) = decode(
        &bytes,
        0,
        bytes.len() as u64,
        Format::Codestream,
        request,
        &accept,
    )
    .unwrap();
    let comp = image.component(0).unwrap();
    assert_eq!((comp.width, comp.height), (25, 13));
    for y in 0..13i64 {
        for x in 0..25i64 {
            assert_eq!(
                comp.data[(y * 25 + x) as usize],
                value(x + 5, y + 7, 0, 8, false)
            );
        }
    }
}

#[test]
fn an_area_outside_the_image_is_refused() {
    let bytes = fixture("gray8.j2k");
    let request = Request {
        area: Some((0, 0, 38, 29)),
        reduce: 0,
    };
    let err = decode(
        &bytes,
        0,
        bytes.len() as u64,
        Format::Codestream,
        request,
        &accept,
    )
    .err()
    .unwrap();
    assert!(matches!(err, Error::Refused(_)), "{err}");
}

#[test]
fn a_reduced_decode_halves_the_image() {
    let bytes = fixture("gray8.j2k");
    let request = Request {
        area: None,
        reduce: 1,
    };
    let (_, image) = decode(
        &bytes,
        0,
        bytes.len() as u64,
        Format::Codestream,
        request,
        &accept,
    )
    .unwrap();
    let comp = image.component(0).unwrap();
    assert_eq!((comp.width, comp.height), (19, 15));
}

#[test]
fn the_check_sees_the_header_and_may_refuse() {
    let bytes = fixture("rgb8.j2k");
    let seen = std::cell::Cell::new(0);
    let check = |h: &Header| {
        seen.set(h.components.len());
        Err("three bands where one was declared".to_string())
    };
    let err = decode(
        &bytes,
        0,
        bytes.len() as u64,
        Format::Codestream,
        Request::default(),
        &check,
    )
    .err()
    .unwrap();
    assert_eq!(seen.get(), 3);
    assert_eq!(
        err,
        Error::Refused("three bands where one was declared".into())
    );
}

#[test]
fn the_wrong_format_is_an_error() {
    let bytes = fixture("gray8.j2k");
    let err = decode(
        &bytes,
        0,
        bytes.len() as u64,
        Format::Jp2,
        Request::default(),
        &accept,
    );
    assert!(matches!(err, Err(Error::Corrupt(_))));
}

/// Every cut and every damaged byte of a codestream gives a decoded image or an error, never a
/// crash. Run with `--nocapture` to see that nothing is printed either.
#[test]
fn damaged_codestreams_are_errors() {
    for name in [
        "gray8.j2k",
        "rgb8.j2k",
        "gray16.j2k",
        "signed12.j2k",
        "gray8.jp2",
    ] {
        let format = if name.ends_with("jp2") {
            Format::Jp2
        } else {
            Format::Codestream
        };
        let good = fixture(name);
        for cut in 0..good.len() {
            let r = decode(&good, 0, cut as u64, format, Request::default(), &accept);
            assert!(r.is_err(), "{name} cut at {cut} decoded");
        }
        let mut state = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..300 {
            let mut bad = good.clone();
            for _ in 0..3 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let i = (state % bad.len() as u64) as usize;
                bad[i] ^= (state >> 32) as u8 | 1;
            }
            let _ = decode(
                &bad,
                0,
                bad.len() as u64,
                format,
                Request::default(),
                &accept,
            );
        }
    }
}

/// A Source that fails to read is an error, with the Source's reason.
#[test]
fn a_failing_source_is_an_error() {
    struct Failing;
    impl ReadAt for Failing {
        fn size(&self) -> std::io::Result<u64> {
            Ok(1000)
        }
        fn read_exact_at(&self, _: &mut [u8], _: u64) -> std::io::Result<()> {
            Err(std::io::Error::other("the disk is gone"))
        }
    }
    let err = decode(
        &Failing,
        0,
        1000,
        Format::Codestream,
        Request::default(),
        &accept,
    )
    .err()
    .unwrap();
    assert_eq!(
        err,
        Error::Io(std::io::ErrorKind::Other, "the disk is gone".into())
    );
}

/// A Source that panics is an error too: the panic does not cross into OpenJPEG.
#[test]
fn a_panicking_source_is_an_error() {
    struct Panicking;
    impl ReadAt for Panicking {
        fn size(&self) -> std::io::Result<u64> {
            Ok(1000)
        }
        fn read_exact_at(&self, _: &mut [u8], _: u64) -> std::io::Result<()> {
            panic!("a broken Source")
        }
    }
    let err = decode(
        &Panicking,
        0,
        1000,
        Format::Codestream,
        Request::default(),
        &accept,
    );
    assert!(matches!(err, Err(Error::Io(..))));
}

#[test]
fn a_place_past_what_can_be_counted_is_refused() {
    let bytes = fixture("gray8.j2k");
    let err = decode(
        &bytes,
        u64::MAX - 4,
        100,
        Format::Codestream,
        Request::default(),
        &accept,
    );
    assert!(matches!(err, Err(Error::Refused(_))));
}

/// A JP2 file with a channel definition box added to its header box.
fn with_cdef(jp2: &[u8]) -> Vec<u8> {
    let at = jp2.windows(4).position(|w| w == b"jp2h").unwrap() - 4;
    let len = u32::from_be_bytes(jp2[at..at + 4].try_into().unwrap()) as usize;
    // One channel, 0, the color image's first, associated with the whole image.
    let cdef = [0, 0, 0, 16, b'c', b'd', b'e', b'f', 0, 1, 0, 0, 0, 0, 0, 1];
    let mut out = jp2[..at].to_vec();
    out.extend(((len + cdef.len()) as u32).to_be_bytes());
    out.extend(&jp2[at + 4..at + len]);
    out.extend(cdef);
    out.extend(&jp2[at + len..]);
    out
}

#[test]
fn jp2_boxes_that_are_not_applied_are_named() {
    let plain = fixture("gray8.jp2");
    let h = read_header(&plain, 0, plain.len() as u64, Format::Jp2).unwrap();
    assert!(h.jp2_boxes.is_empty());
    let marked = with_cdef(&plain);
    let h = read_header(&marked, 0, marked.len() as u64, Format::Jp2).unwrap();
    assert_eq!(h.jp2_boxes, ["cdef"]);
    // The codestream is unchanged, and decodes as before.
    let (_, image) = decode(
        &marked,
        0,
        marked.len() as u64,
        Format::Jp2,
        Request::default(),
        &accept,
    )
    .unwrap();
    assert_eq!(
        image.component(0).unwrap().data[40],
        value(3, 1, 0, 8, false)
    );
}

#[test]
fn the_format_is_told_by_the_first_bytes() {
    assert_eq!(Format::sniff(&fixture("gray8.jp2")), Some(Format::Jp2));
    assert_eq!(
        Format::sniff(&fixture("gray8.j2k")),
        Some(Format::Codestream)
    );
    assert_eq!(Format::sniff(b"II*\0"), None);
}
