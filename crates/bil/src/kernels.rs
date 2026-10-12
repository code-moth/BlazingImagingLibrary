// SPDX-License-Identifier: LGPL-2.1-or-later
//! Slice function kernels, written once over `Vf` and compiled for each instruction set.
//!
//! Each kernel maps rows of f32 component buffers to rows of f32 planes. A row is processed in
//! whole vectors, and the last few values go through the same vector code on a padded copy, so
//! a value's bits never depend on where it falls in a row. That is what makes Window
//! independence exact.

use crate::simd::Vf;

const MAX_LANES: usize = 16;

/// Applies `f` across `n` values, `NI` inputs to `NO` outputs.
#[inline(always)]
pub unsafe fn map<V: Vf, const NI: usize, const NO: usize>(
    n: usize,
    ins: [*const f32; NI],
    outs: [*mut f32; NO],
    f: impl Fn([V; NI]) -> [V; NO],
) {
    let l = V::LANES;
    let mut i = 0;
    while i + l <= n {
        let mut v = [V::splat(0.0); NI];
        for k in 0..NI {
            v[k] = V::load(ins[k].add(i));
        }
        let r = f(v);
        for k in 0..NO {
            r[k].store(outs[k].add(i));
        }
        i += l;
    }
    if i < n {
        // The tail: copy into padded lanes, run the same vector code, copy back.
        let rem = n - i;
        let mut tin = [[0f32; MAX_LANES]; NI];
        let mut tout = [[0f32; MAX_LANES]; NO];
        for k in 0..NI {
            for j in 0..rem {
                tin[k][j] = *ins[k].add(i + j);
            }
        }
        let mut v = [V::splat(0.0); NI];
        for k in 0..NI {
            v[k] = V::load(tin[k].as_ptr());
        }
        let r = f(v);
        for k in 0..NO {
            r[k].store(tout[k].as_mut_ptr());
            for j in 0..rem {
                *outs[k].add(i + j) = tout[k][j];
            }
        }
    }
}

const PI: f32 = std::f32::consts::PI;
const FRAC_PI_2: f32 = std::f32::consts::FRAC_PI_2;
const FRAC_PI_4: f32 = std::f32::consts::FRAC_PI_4;
const TAN_PI_8: f32 = 0.414_213_57;
const INV_TAU: f32 = 1.0 / std::f32::consts::TAU;

/// atan2(y, x) in (-pi, pi], from basic operations only, after Cephes `atanf`.
#[inline(always)]
pub unsafe fn atan2<V: Vf>(y: V, x: V) -> V {
    let zero = V::splat(0.0);
    let one = V::splat(1.0);
    let ax = x.abs();
    let ay = y.abs();
    let swap = ay.gt(ax);
    let num = V::select(swap, ax, ay);
    let den = V::select(swap, ay, ax);
    let den_zero = den.eq(zero);
    let t = V::select(den_zero, zero, num.div(V::select(den_zero, one, den)));
    // t is in [0, 1]. Reduce to [-tan(pi/8), tan(pi/8)].
    let big = t.gt(V::splat(TAN_PI_8));
    let tr = V::select(big, t.sub(one).div(t.add(one)), t);
    let base = V::select(big, V::splat(FRAC_PI_4), zero);
    let z = tr.mul(tr);
    let p = V::splat(8.053_744_5e-2)
        .mul(z)
        .sub(V::splat(1.387_768_6e-1))
        .mul(z)
        .add(V::splat(1.997_771_1e-1))
        .mul(z)
        .sub(V::splat(3.333_294_9e-1))
        .mul(z)
        .mul(tr)
        .add(tr);
    let mut a = base.add(p);
    a = V::select(swap, V::splat(FRAC_PI_2).sub(a), a);
    a = V::select(x.lt(zero), V::splat(PI).sub(a), a);
    V::select(y.lt(zero), zero.sub(a), a)
}

/// cos and sin of an angle given in turns, from basic operations only, after Cephes.
#[inline(always)]
pub unsafe fn cos_sin_turns<V: Vf>(h: V) -> (V, V) {
    let q = h.mul(V::splat(4.0));
    let j = q.add(V::splat(0.5)).floor();
    let r = q.sub(j).mul(V::splat(FRAC_PI_2)); // in [-pi/4, pi/4]
    let z = r.mul(r);
    let s = r.add(
        r.mul(z).mul(
            V::splat(-1.666_665_4e-1)
                .add(z.mul(V::splat(8.332_161e-3).add(z.mul(V::splat(-1.951_529_6e-4))))),
        ),
    );
    let c = V::splat(1.0).sub(V::splat(0.5).mul(z)).add(
        z.mul(z).mul(
            V::splat(4.166_664_6e-2)
                .add(z.mul(V::splat(-1.388_731_6e-3).add(z.mul(V::splat(2.443_315_7e-5))))),
        ),
    );
    // Quadrant j mod 4.
    let quad = j.sub(j.mul(V::splat(0.25)).floor().mul(V::splat(4.0)));
    let q1 = quad.eq(V::splat(1.0));
    let q2 = quad.eq(V::splat(2.0));
    let q3 = quad.eq(V::splat(3.0));
    let zero = V::splat(0.0);
    let neg_s = zero.sub(s);
    let neg_c = zero.sub(c);
    let cos = V::select(q1, neg_s, V::select(q2, neg_c, V::select(q3, s, c)));
    let sin = V::select(q1, c, V::select(q2, neg_s, V::select(q3, neg_c, s)));
    (cos, sin)
}

/// One halving of a linear plane: (a + c + 2b) / 4, in a fixed order.
#[inline(always)]
pub unsafe fn halve<V: Vf>(n: usize, a: *const f32, b: *const f32, c: *const f32, out: *mut f32) {
    map::<V, 3, 1>(
        n,
        [a, b, c],
        [out],
        #[inline(always)]
        |[a, b, c]| [a.add(c).add(b.add(b)).mul(V::splat(0.25))],
    );
}

/// One halving along a row, reading the row itself: `out[i]` is made from `src[2i]`, `src[2i + 1]`
/// and `src[2i + 2]` with the arithmetic of `halve`, so the bits are the same. The even and odd
/// columns are separated by vector shuffles, which only move values.
///
/// `src` holds `2n + 1` values.
#[inline(always)]
pub unsafe fn halve_row<V: Vf>(n: usize, src: *const f32, out: *mut f32) {
    row_pairs::<V, 1>(
        n,
        src,
        [out],
        #[inline(always)]
        |a, b, c| [a.add(c).add(b.add(b)).mul(V::splat(0.25))],
    );
}

/// The first half of a halving where values may be missing, as NaN: the weighted sum of the
/// values present, and the sum of their weights, each with weights 1 2 1 and scaled by a quarter
/// as `halve` scales, but not yet divided by the weight.
#[inline(always)]
pub unsafe fn halve_sum<V: Vf>(
    n: usize,
    a: *const f32,
    b: *const f32,
    c: *const f32,
    out: [*mut f32; 2],
) {
    map::<V, 3, 2>(
        n,
        [a, b, c],
        out,
        #[inline(always)]
        |[a, b, c]| sum_present(a, b, c),
    );
}

/// `halve_sum` along a row, reading the row itself as `halve_row` does. `src` holds `2n + 1`
/// values.
#[inline(always)]
pub unsafe fn halve_row_sum<V: Vf>(n: usize, src: *const f32, out: [*mut f32; 2]) {
    row_pairs::<V, 2>(
        n,
        src,
        out,
        #[inline(always)]
        |a, b, c| sum_present(a, b, c),
    );
}

/// The second half of a halving where values may be missing: the weighted sums and weights of
/// three rows, combined with weights 1 2 1 and divided. NaN where no value was present.
#[inline(always)]
pub unsafe fn halve_skip<V: Vf>(
    n: usize,
    sums: [*const f32; 3],
    weights: [*const f32; 3],
    out: *mut f32,
) {
    map::<V, 6, 1>(
        n,
        [
            sums[0], sums[1], sums[2], weights[0], weights[1], weights[2],
        ],
        [out],
        #[inline(always)]
        |[sa, sb, sc, wa, wb, wc]| {
            let quarter = V::splat(0.25);
            let s = sa.add(sc).add(sb.add(sb)).mul(quarter);
            let w = wa.add(wc).add(wb.add(wb)).mul(quarter);
            let none = w.eq(V::splat(0.0));
            [V::select(
                none,
                V::splat(f32::NAN),
                s.div(V::select(none, V::splat(1.0), w)),
            )]
        },
    );
}

/// `x` where `probe` is a number, and NaN where `probe` is NaN. A mask of 1.0 and NaN applies a
/// validity mask; a Slice as the probe carries its missing values to another made beside it.
#[inline(always)]
pub unsafe fn keep_present<V: Vf>(n: usize, probe: *const f32, x: *const f32, out: *mut f32) {
    map::<V, 2, 1>(
        n,
        [probe, x],
        [out],
        #[inline(always)]
        |[p, x]| [V::select(p.eq(p), x, V::splat(f32::NAN))],
    );
}

/// Present values with weights 1 2 1, and the sum of their weights, both scaled by a quarter. A
/// value is present when it is not NaN. With all three present the weight is exactly 1 and the
/// sum has the bits of `halve`, so a mask with nothing missing changes nothing, over the whole
/// float32 range. Every operation is exact or correctly rounded, so the result is the same on
/// every instruction set.
#[inline(always)]
unsafe fn sum_present<V: Vf>(a: V, b: V, c: V) -> [V; 2] {
    let zero = V::splat(0.0);
    let one = V::splat(1.0);
    let (pa, pb, pc) = (a.eq(a), b.eq(b), c.eq(c));
    let (a, b, c) = (
        V::select(pa, a, zero),
        V::select(pb, b, zero),
        V::select(pc, c, zero),
    );
    let (wa, wb, wc) = (
        V::select(pa, one, zero),
        V::select(pb, one, zero),
        V::select(pc, one, zero),
    );
    let quarter = V::splat(0.25);
    [
        a.add(c).add(b.add(b)).mul(quarter),
        wa.add(wc).add(wb.add(wb)).mul(quarter),
    ]
}

/// Applies `f` to the left, center and right sources of each output of a halving along a row,
/// read from the row itself: `src[2i]`, `src[2i + 1]` and `src[2i + 2]`. `src` holds `2n + 1`
/// values. The last outputs go through the same vector code on a padded copy.
#[inline(always)]
unsafe fn row_pairs<V: Vf, const NO: usize>(
    n: usize,
    src: *const f32,
    outs: [*mut f32; NO],
    f: impl Fn(V, V, V) -> [V; NO],
) {
    let l = V::LANES;
    let mut i = 0;
    // The right source of the last output in a vector is read by the second load, which ends
    // one value past it, so a full vector needs i + l < n.
    while i + l < n {
        let (a, b) = V::load_even_odd(src.add(2 * i));
        let (c, _) = V::load_even_odd(src.add(2 * i + 2));
        let r = f(a, b, c);
        for k in 0..NO {
            r[k].store(outs[k].add(i));
        }
        i += l;
    }
    if i < n {
        let rem = n - i;
        let mut t = [0f32; 2 * MAX_LANES + 2];
        for j in 0..2 * rem + 1 {
            t[j] = *src.add(2 * i + j);
        }
        let (a, b) = V::load_even_odd(t.as_ptr());
        let (c, _) = V::load_even_odd(t.as_ptr().add(2));
        let r = f(a, b, c);
        let mut tout = [0f32; MAX_LANES];
        for k in 0..NO {
            r[k].store(tout.as_mut_ptr());
            for j in 0..rem {
                *outs[k].add(i + j) = tout[j];
            }
        }
    }
}

/// `complex.polar` at Level 0: magnitude, the unit vector carried to coarser Levels, and phase.
#[inline(always)]
pub unsafe fn polar0<V: Vf>(n: usize, re: *const f32, im: *const f32, out: [*mut f32; 4]) {
    map::<V, 2, 4>(
        n,
        [re, im],
        out,
        #[inline(always)]
        |[re, im]| {
            let mag = re.mul(re).add(im.mul(im)).sqrt();
            let zero = V::splat(0.0);
            let is_zero = mag.eq(zero);
            let safe = V::select(is_zero, V::splat(1.0), mag);
            let ure = V::select(is_zero, zero, re.div(safe));
            let uim = V::select(is_zero, zero, im.div(safe));
            [mag, ure, uim, atan2(im, re)]
        },
    );
}

/// Phase at a coarser Level, from the averaged unit vector.
#[inline(always)]
pub unsafe fn phase_from<V: Vf>(n: usize, ure: *const f32, uim: *const f32, out: *mut f32) {
    map::<V, 2, 1>(
        n,
        [ure, uim],
        [out],
        #[inline(always)]
        |[c, s]| [atan2(s, c)],
    );
}

/// `color.hsv` at Level 0, from encoded values: saturation, value, the hue unit vector
/// (zero where hue is undefined), and hue in turns.
#[inline(always)]
pub unsafe fn hsv0<V: Vf>(
    n: usize,
    r: *const f32,
    g: *const f32,
    b: *const f32,
    out: [*mut f32; 5],
) {
    map::<V, 3, 5>(
        n,
        [r, g, b],
        out,
        #[inline(always)]
        |[r, g, b]| {
            let zero = V::splat(0.0);
            let one = V::splat(1.0);
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let delta = max.sub(min);
            let max_zero = max.eq(zero);
            let s = V::select(max_zero, zero, delta.div(V::select(max_zero, one, max)));
            let flat = delta.eq(zero);
            let d = V::select(flat, one, delta);
            let hr = g.sub(b).div(d);
            let hr = V::select(hr.lt(zero), hr.add(V::splat(6.0)), hr);
            let hg = b.sub(r).div(d).add(V::splat(2.0));
            let hb = r.sub(g).div(d).add(V::splat(4.0));
            let is_r = max.eq(r);
            let is_g = V::and(V::not(is_r), max.eq(g));
            let sixths = V::select(is_r, hr, V::select(is_g, hg, hb));
            let h = V::select(flat, zero, sixths.mul(V::splat(1.0 / 6.0)));
            let h = V::select(h.lt(one), h, zero); // 6/6 wraps to 0
            let (c, sn) = cos_sin_turns(h);
            let hc = V::select(flat, zero, c);
            let hs = V::select(flat, zero, sn);
            [s, max, hc, hs, h]
        },
    );
}

/// Hue at a coarser Level, in turns, from the averaged unit vector.
#[inline(always)]
pub unsafe fn hue_from<V: Vf>(n: usize, hc: *const f32, hs: *const f32, out: *mut f32) {
    map::<V, 2, 1>(
        n,
        [hc, hs],
        [out],
        #[inline(always)]
        |[c, s]| {
            let h = atan2(s, c).mul(V::splat(INV_TAU));
            [V::select(h.lt(V::splat(0.0)), h.add(V::splat(1.0)), h)]
        },
    );
}
