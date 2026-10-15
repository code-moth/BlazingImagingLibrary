// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! A TIFF chunk's stored bytes to native samples, by BIL's own code (M39, option (b)): the
//! compression Tier 1 reads (Deflate through `miniz_oxide`, LZW through `weezl`, PackBits),
//! the predictors, the file's byte order and samples of any width. Nothing here interprets a
//! sample: WhiteIsZero, palettes and extra samples come back as stored (section 4).

use crate::types::SampleType;

/// How a chunk's bytes are coded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Coding {
    None,
    Deflate,
    Lzw,
    PackBits,
}

impl Coding {
    /// The coding of a Compression tag's value, where this build decodes it.
    pub(super) fn of(compression: u16) -> Option<Coding> {
        Some(match compression {
            1 => Coding::None,
            8 | 32946 => Coding::Deflate,
            5 => Coding::Lzw,
            32773 => Coding::PackBits,
            _ => return None,
        })
    }

    /// The most bytes one stored byte can decode to: more than this, and the chunk's declared
    /// size is one its data cannot hold.
    fn most_per_byte(self) -> u64 {
        match self {
            Coding::None => 1,
            // A Deflate match of 258 bytes takes 2 bits at the least.
            Coding::Deflate => 1_032,
            // A 9-bit code can stand for a string of up to 4,096 bytes.
            Coding::Lzw => 3_641,
            // Two bytes repeat one byte 128 times.
            Coding::PackBits => 64,
        }
    }
}

/// How a chunk's samples are stored, from its IFD.
#[derive(Clone, Copy, Debug)]
pub(super) struct Samples {
    /// The type delivered.
    pub sample_type: SampleType,
    /// Bits of one sample as stored: the type's own, or another width that widens to it (M43).
    pub bits: u32,
    /// Whether an integer of another width is signed, to be extended.
    pub signed: bool,
    pub big_endian: bool,
    /// 1 none, 2 horizontal differencing, 3 floating point (TIFF Technical Note 3).
    pub predictor: u16,
    /// FillOrder 2: the bits of each stored byte are in reverse.
    pub reversed_bits: bool,
}

impl Samples {
    /// Whether samples are stored at their type's own width.
    fn whole(&self) -> bool {
        self.bits as usize == self.sample_type.size() * 8
    }

    /// Bytes of one stored row of `values` samples: rows start on a byte (TIFF 6.0, section 7).
    pub(super) fn row_bytes(&self, values: u64) -> Option<u64> {
        values
            .checked_mul(u64::from(self.bits))
            .map(|b| b.div_ceil(8))
    }

    /// Why these samples cannot be read, if they cannot: a predictor TIFF does not define for
    /// them. libtiff refuses the same.
    pub(super) fn check(&self) -> Result<(), String> {
        let float = matches!(
            self.sample_type,
            SampleType::F16 | SampleType::F32 | SampleType::F64
        );
        let complex = self.sample_type.number_size() != self.sample_type.size();
        match self.predictor {
            1 => Ok(()),
            2 if self.whole() && !complex => Ok(()),
            3 if float => Ok(()),
            p @ (2 | 3) => Err(format!(
                "predictor {p} with {} samples of {} bits",
                if float { "float" } else { "these" },
                self.bits
            )),
            p => Err(format!("predictor {p}, which TIFF does not define")),
        }
    }
}

/// Decodes a chunk's stored bytes into `out`, which holds exactly the bytes its rows take as
/// stored. Too little data is corrupt; data left over once `out` is full is not used, as
/// libtiff does not use it.
pub(super) fn decompress(
    coding: Coding,
    stored: &mut [u8],
    reversed_bits: bool,
    out: &mut [u8],
) -> Result<(), String> {
    if reversed_bits {
        // FillOrder applies to the stored bytes, before decompression, as in libtiff.
        for b in stored.iter_mut() {
            *b = b.reverse_bits();
        }
    }
    if (out.len() as u64) > (stored.len() as u64).saturating_mul(coding.most_per_byte()) {
        return Err(format!(
            "a chunk of {} bytes cannot hold the {} bytes its rows need",
            stored.len(),
            out.len()
        ));
    }
    let want = out.len();
    let short = |n: usize| format!("a chunk holds {n} of the {want} bytes its rows need");
    match coding {
        Coding::None => {
            let n = out.len();
            out.copy_from_slice(stored.get(..n).ok_or_else(|| short(stored.len()))?);
        }
        Coding::Deflate => {
            use miniz_oxide::inflate::core::{decompress, inflate_flags, DecompressorOxide};
            use miniz_oxide::inflate::TINFLStatus;
            let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
                | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
            let mut d = Box::<DecompressorOxide>::default();
            let (status, _, n) = decompress(&mut d, stored, out, 0, flags);
            match status {
                // The rows are all there, whatever follows them.
                TINFLStatus::Done
                | TINFLStatus::HasMoreOutput
                | TINFLStatus::NeedsMoreInput
                | TINFLStatus::FailedCannotMakeProgress
                    if n == out.len() => {}
                TINFLStatus::Done
                | TINFLStatus::NeedsMoreInput
                | TINFLStatus::FailedCannotMakeProgress => return Err(short(n)),
                e => return Err(format!("Deflate data that cannot be decoded ({e:?})")),
            }
        }
        Coding::Lzw => {
            use weezl::{decode::Decoder, BitOrder, LzwStatus};
            let mut d = Decoder::with_tiff_size_switch(BitOrder::Msb, 8);
            let (mut read, mut written) = (0, 0);
            while written < out.len() {
                let r = d.decode_bytes(&stored[read..], &mut out[written..]);
                read += r.consumed_in;
                written += r.consumed_out;
                match r.status {
                    Ok(LzwStatus::Ok) => {}
                    Ok(LzwStatus::Done | LzwStatus::NoProgress) => break,
                    Err(e) => return Err(format!("LZW data that cannot be decoded ({e})")),
                }
            }
            if written < out.len() {
                return Err(short(written));
            }
        }
        Coding::PackBits => {
            let (mut i, mut o) = (0, 0);
            while o < out.len() && i < stored.len() {
                let n = stored[i] as i8;
                i += 1;
                if n >= 0 {
                    let run = (n as usize + 1).min(out.len() - o);
                    let from = stored.get(i..i + run).ok_or_else(|| short(o))?;
                    out[o..o + run].copy_from_slice(from);
                    (i, o) = (i + n as usize + 1, o + run);
                } else if n != -128 {
                    let b = *stored.get(i).ok_or_else(|| short(o))?;
                    let run = ((1 - i32::from(n)) as usize).min(out.len() - o);
                    out[o..o + run].fill(b);
                    (i, o) = (i + 1, o + run);
                }
            }
            if o < out.len() {
                return Err(short(o));
            }
        }
    }
    Ok(())
}

/// One stored row of `values` samples, decoded and predicted, to native samples of the type
/// delivered, appended to `dst`. `per` is the samples a pixel within the chunk, the predictor's
/// stride.
pub(super) fn row(s: &Samples, src: &mut [u8], values: usize, per: usize, dst: &mut Vec<u8>) {
    let size = s.sample_type.size();
    let start = dst.len();
    if s.predictor == 3 {
        // Each byte less the one `per` before it, then the bytes of each value regrouped by
        // place, most significant first (TIFF Technical Note 3).
        for i in per..src.len() {
            src[i] = src[i].wrapping_add(src[i - per]);
        }
        dst.resize(start + values * size, 0);
        for i in 0..values {
            for k in 0..size {
                let b = src[k * values + i];
                let at = if cfg!(target_endian = "big") {
                    k
                } else {
                    size - 1 - k
                };
                dst[start + i * size + at] = b;
            }
        }
        return;
    }
    if s.whole() {
        dst.extend_from_slice(&src[..values * size]);
        let number = s.sample_type.number_size();
        if s.big_endian != cfg!(target_endian = "big") && number > 1 {
            for n in dst[start..].chunks_exact_mut(number) {
                n.reverse();
            }
        }
        if s.predictor == 2 {
            let d = &mut dst[start..];
            macro_rules! add {
                ($t:ty) => {{
                    const W: usize = std::mem::size_of::<$t>();
                    for i in per..values {
                        let prev = <$t>::from_ne_bytes(d[(i - per) * W..][..W].try_into().unwrap());
                        let at = &mut d[i * W..][..W];
                        let v = <$t>::from_ne_bytes((&*at).try_into().unwrap());
                        at.copy_from_slice(&v.wrapping_add(prev).to_ne_bytes());
                    }
                }};
            }
            match size {
                1 => add!(u8),
                2 => add!(u16),
                4 => add!(u32),
                _ => add!(u64),
            }
        }
        return;
    }
    // Another width: whole bytes in the file's order, or a stream of bits, most significant
    // first, widened to the type delivered and sign-extended where signed (M43).
    let bits = s.bits as usize;
    dst.reserve(values * size);
    for i in 0..values {
        let v: u64 = if bits.is_multiple_of(8) {
            let b = &src[i * bits / 8..][..bits / 8];
            if s.big_endian {
                b.iter().fold(0, |v, &x| v << 8 | u64::from(x))
            } else {
                b.iter().rev().fold(0, |v, &x| v << 8 | u64::from(x))
            }
        } else {
            let mut v = 0u64;
            for k in i * bits..(i + 1) * bits {
                v = v << 1 | u64::from(src[k / 8] >> (7 - k % 8) & 1);
            }
            v
        };
        let v = if s.signed && bits < 64 && v >> (bits - 1) & 1 == 1 {
            v | !0u64 << bits
        } else {
            v
        };
        dst.extend_from_slice(
            &v.to_ne_bytes()[if cfg!(target_endian = "big") {
                8 - size
            } else {
                0
            }..][..size],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(t: SampleType, bits: u32, signed: bool) -> Samples {
        Samples {
            sample_type: t,
            bits,
            signed,
            big_endian: false,
            predictor: 1,
            reversed_bits: false,
        }
    }

    #[test]
    fn packbits_runs_and_literals() {
        // Apple's example in TIFF 6.0, section 9.
        let mut stored = [
            0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7,
            0xAA,
        ];
        let mut out = [0u8; 24];
        decompress(Coding::PackBits, &mut stored, false, &mut out).unwrap();
        assert_eq!(
            out,
            [
                0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22,
                0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA
            ]
        );
        let mut more = [0u8; 25];
        assert!(decompress(Coding::PackBits, &mut stored, false, &mut more).is_err());
    }

    #[test]
    fn lzw_round_trips_through_weezl() {
        let data: Vec<u8> = (0..5_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut stored = weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
            .encode(&data)
            .unwrap();
        let mut out = vec![0u8; data.len()];
        decompress(Coding::Lzw, &mut stored, false, &mut out).unwrap();
        assert_eq!(out, data);
        let mut more = vec![0u8; data.len() + 1];
        assert!(decompress(Coding::Lzw, &mut stored, false, &mut more).is_err());
    }

    #[test]
    fn odd_widths_widen_and_extend_their_sign() {
        // Three 12-bit samples: 0xABC, 0x123, 0xFFF, as a stream of bits.
        let mut src = [0xAB, 0xC1, 0x23, 0xFF, 0xF0];
        let mut dst = Vec::new();
        row(
            &samples(SampleType::U16, 12, false),
            &mut src,
            3,
            1,
            &mut dst,
        );
        let u: Vec<u16> = dst
            .chunks(2)
            .map(|b| u16::from_ne_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(u, [0xABC, 0x123, 0xFFF]);
        dst.clear();
        row(
            &samples(SampleType::I16, 12, true),
            &mut src,
            3,
            1,
            &mut dst,
        );
        let i: Vec<i16> = dst
            .chunks(2)
            .map(|b| i16::from_ne_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(i, [0xABC_u16 as i16 - 4096, 0x123, -1]);
        // 24 bits, in the file's byte order.
        let mut src = [0x01, 0x02, 0x03];
        dst.clear();
        let mut s = samples(SampleType::U32, 24, false);
        row(&s, &mut src, 1, 1, &mut dst);
        assert_eq!(u32::from_ne_bytes(dst[..4].try_into().unwrap()), 0x030201);
        dst.clear();
        s.big_endian = true;
        row(&s, &mut src, 1, 1, &mut dst);
        assert_eq!(u32::from_ne_bytes(dst[..4].try_into().unwrap()), 0x010203);
    }

    #[test]
    fn predictors_need_the_widths_tiff_defines() {
        let mut s = samples(SampleType::U16, 12, false);
        s.predictor = 2;
        assert!(s.check().is_err());
        s.predictor = 3;
        assert!(s.check().is_err());
        let mut f = samples(SampleType::F32, 32, false);
        f.predictor = 3;
        assert!(f.check().is_ok());
        f.predictor = 4;
        assert!(f.check().is_err());
        let mut c = samples(SampleType::ComplexI16, 32, true);
        c.predictor = 2;
        assert!(c.check().is_err());
    }
}
