// SPDX-License-Identifier: LGPL-2.1-or-later
//! A small vector abstraction with one scalar and two x86 back ends.
//!
//! Every operation is an IEEE operation that is correctly rounded (add, sub, mul, div, sqrt),
//! exact (min, max, abs, floor, compare, select), or a bit operation. There is no fused
//! multiply-add and no approximate reciprocal, so a lane computes the same bits whichever back
//! end runs it. That is the property experiment 1 tests.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// A vector of `LANES` f32 values.
///
/// # Safety
/// Implementations may use instructions that the running processor must support. Callers reach
/// the vector back ends only through functions compiled with the matching target feature, after
/// checking the processor at run time.
pub unsafe trait Vf: Copy {
    const LANES: usize;
    type M: Copy;
    unsafe fn splat(x: f32) -> Self;
    unsafe fn load(p: *const f32) -> Self;
    unsafe fn store(self, p: *mut f32);
    unsafe fn add(self, o: Self) -> Self;
    unsafe fn sub(self, o: Self) -> Self;
    unsafe fn mul(self, o: Self) -> Self;
    unsafe fn div(self, o: Self) -> Self;
    unsafe fn sqrt(self) -> Self;
    /// `self < o ? self : o`, as the x86 instruction defines it.
    unsafe fn min(self, o: Self) -> Self;
    /// `self > o ? self : o`, as the x86 instruction defines it.
    unsafe fn max(self, o: Self) -> Self;
    unsafe fn abs(self) -> Self;
    unsafe fn floor(self) -> Self;
    unsafe fn lt(self, o: Self) -> Self::M;
    unsafe fn gt(self, o: Self) -> Self::M;
    unsafe fn eq(self, o: Self) -> Self::M;
    /// `m ? a : b`, lane by lane.
    unsafe fn select(m: Self::M, a: Self, b: Self) -> Self;
    unsafe fn and(a: Self::M, b: Self::M) -> Self::M;
    unsafe fn not(a: Self::M) -> Self::M;
}

/// One lane, in plain Rust. It is also the reference the vector back ends must match.
#[derive(Clone, Copy)]
pub struct Scalar(pub f32);

unsafe impl Vf for Scalar {
    const LANES: usize = 1;
    type M = bool;
    #[inline(always)]
    unsafe fn splat(x: f32) -> Self {
        Scalar(x)
    }
    #[inline(always)]
    unsafe fn load(p: *const f32) -> Self {
        Scalar(*p)
    }
    #[inline(always)]
    unsafe fn store(self, p: *mut f32) {
        *p = self.0
    }
    #[inline(always)]
    unsafe fn add(self, o: Self) -> Self {
        Scalar(self.0 + o.0)
    }
    #[inline(always)]
    unsafe fn sub(self, o: Self) -> Self {
        Scalar(self.0 - o.0)
    }
    #[inline(always)]
    unsafe fn mul(self, o: Self) -> Self {
        Scalar(self.0 * o.0)
    }
    #[inline(always)]
    unsafe fn div(self, o: Self) -> Self {
        Scalar(self.0 / o.0)
    }
    #[inline(always)]
    unsafe fn sqrt(self) -> Self {
        Scalar(self.0.sqrt())
    }
    #[inline(always)]
    unsafe fn min(self, o: Self) -> Self {
        Scalar(if self.0 < o.0 { self.0 } else { o.0 })
    }
    #[inline(always)]
    unsafe fn max(self, o: Self) -> Self {
        Scalar(if self.0 > o.0 { self.0 } else { o.0 })
    }
    #[inline(always)]
    unsafe fn abs(self) -> Self {
        Scalar(f32::from_bits(self.0.to_bits() & 0x7fff_ffff))
    }
    #[inline(always)]
    unsafe fn floor(self) -> Self {
        Scalar(self.0.floor())
    }
    #[inline(always)]
    unsafe fn lt(self, o: Self) -> bool {
        self.0 < o.0
    }
    #[inline(always)]
    unsafe fn gt(self, o: Self) -> bool {
        self.0 > o.0
    }
    #[inline(always)]
    unsafe fn eq(self, o: Self) -> bool {
        self.0 == o.0
    }
    #[inline(always)]
    unsafe fn select(m: bool, a: Self, b: Self) -> Self {
        if m {
            a
        } else {
            b
        }
    }
    #[inline(always)]
    unsafe fn and(a: bool, b: bool) -> bool {
        a & b
    }
    #[inline(always)]
    unsafe fn not(a: bool) -> bool {
        !a
    }
}

/// Eight lanes, AVX2.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
pub struct Avx2(pub __m256);

#[cfg(target_arch = "x86_64")]
unsafe impl Vf for Avx2 {
    const LANES: usize = 8;
    type M = __m256;
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn splat(x: f32) -> Self {
        Avx2(_mm256_set1_ps(x))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn load(p: *const f32) -> Self {
        Avx2(_mm256_loadu_ps(p))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn store(self, p: *mut f32) {
        _mm256_storeu_ps(p, self.0)
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn add(self, o: Self) -> Self {
        Avx2(_mm256_add_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn sub(self, o: Self) -> Self {
        Avx2(_mm256_sub_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn mul(self, o: Self) -> Self {
        Avx2(_mm256_mul_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn div(self, o: Self) -> Self {
        Avx2(_mm256_div_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn sqrt(self) -> Self {
        Avx2(_mm256_sqrt_ps(self.0))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn min(self, o: Self) -> Self {
        Avx2(_mm256_min_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn max(self, o: Self) -> Self {
        Avx2(_mm256_max_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn abs(self) -> Self {
        Avx2(_mm256_and_ps(
            self.0,
            _mm256_castsi256_ps(_mm256_set1_epi32(0x7fff_ffff)),
        ))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn floor(self) -> Self {
        Avx2(_mm256_floor_ps(self.0))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn lt(self, o: Self) -> __m256 {
        _mm256_cmp_ps::<_CMP_LT_OQ>(self.0, o.0)
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn gt(self, o: Self) -> __m256 {
        _mm256_cmp_ps::<_CMP_GT_OQ>(self.0, o.0)
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn eq(self, o: Self) -> __m256 {
        _mm256_cmp_ps::<_CMP_EQ_OQ>(self.0, o.0)
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn select(m: __m256, a: Self, b: Self) -> Self {
        Avx2(_mm256_blendv_ps(b.0, a.0, m))
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn and(a: __m256, b: __m256) -> __m256 {
        _mm256_and_ps(a, b)
    }
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn not(a: __m256) -> __m256 {
        _mm256_xor_ps(a, _mm256_castsi256_ps(_mm256_set1_epi32(-1)))
    }
}

/// Sixteen lanes, AVX-512.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
pub struct Avx512(pub __m512);

#[cfg(target_arch = "x86_64")]
unsafe impl Vf for Avx512 {
    const LANES: usize = 16;
    type M = __mmask16;
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn splat(x: f32) -> Self {
        Avx512(_mm512_set1_ps(x))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn load(p: *const f32) -> Self {
        Avx512(_mm512_loadu_ps(p))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn store(self, p: *mut f32) {
        _mm512_storeu_ps(p, self.0)
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn add(self, o: Self) -> Self {
        Avx512(_mm512_add_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn sub(self, o: Self) -> Self {
        Avx512(_mm512_sub_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn mul(self, o: Self) -> Self {
        Avx512(_mm512_mul_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn div(self, o: Self) -> Self {
        Avx512(_mm512_div_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn sqrt(self) -> Self {
        Avx512(_mm512_sqrt_ps(self.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn min(self, o: Self) -> Self {
        Avx512(_mm512_min_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn max(self, o: Self) -> Self {
        Avx512(_mm512_max_ps(self.0, o.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn abs(self) -> Self {
        Avx512(_mm512_abs_ps(self.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn floor(self) -> Self {
        Avx512(_mm512_roundscale_ps::<0x09>(self.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn lt(self, o: Self) -> __mmask16 {
        _mm512_cmp_ps_mask::<_CMP_LT_OQ>(self.0, o.0)
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn gt(self, o: Self) -> __mmask16 {
        _mm512_cmp_ps_mask::<_CMP_GT_OQ>(self.0, o.0)
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn eq(self, o: Self) -> __mmask16 {
        _mm512_cmp_ps_mask::<_CMP_EQ_OQ>(self.0, o.0)
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn select(m: __mmask16, a: Self, b: Self) -> Self {
        Avx512(_mm512_mask_blend_ps(m, b.0, a.0))
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn and(a: __mmask16, b: __mmask16) -> __mmask16 {
        a & b
    }
    #[inline]
    #[target_feature(enable = "avx512f")]
    unsafe fn not(a: __mmask16) -> __mmask16 {
        !a
    }
}

/// The instruction set a pass runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// On processors other than x86 only `Scalar` is ever made.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub enum Isa {
    Scalar,
    Avx2,
    Avx512,
}

impl Isa {
    /// Every instruction set this processor can run, scalar first.
    pub fn available() -> Vec<Isa> {
        // Only x86 adds to the list.
        #[cfg_attr(not(target_arch = "x86_64"), allow(unused_mut))]
        let mut v = vec![Isa::Scalar];
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx2") {
                v.push(Isa::Avx2);
            }
            if is_x86_feature_detected!("avx512f") {
                v.push(Isa::Avx512);
            }
        }
        v
    }

    /// The fastest available, chosen once.
    pub fn best() -> Isa {
        *Isa::available().last().unwrap()
    }
}
