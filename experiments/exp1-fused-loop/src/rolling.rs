// SPDX-License-Identifier: LGPL-2.1-or-later
//! Experiment 1b: the same arithmetic as `read_levels`, with rows streamed through every Level
//! on a buffer of a few rows, and every Slice written straight into buffers the caller owns.
//!
//! Each Level above Level 0 keeps a ring of rows, already halved along the row, from the Level
//! before: three for `bilinear` and five for `binomial5`. A row of Level k is made as soon as the
//! rows of Level k - 1 it needs are in the ring, and it is written out at once and passed on to
//! Level k + 1. No full-size intermediate plane is made. Where nothing can be missing, the kernels and the order of
//! operations are those of `read_levels`, so every value has the same bits.
//!
//! **Missing data.** An Image with a validity mask takes a second path, for section 5 of the
//! specification. A missing pixel is NaN in every Slice. Each halving leaves NaN out and rescales
//! the remaining weights, and a value is NaN only when every value that would contribute is
//! missing. `missing_value` is applied last, to the finished Slice. An Image with no mask makes
//! none of these checks.
//!
//! `missing_weights` chooses how a Level weights the values below it: `Equal`, the default, or
//! `PixelCount`, by the share of present full-resolution pixels behind each (version 67).
//!
//! Four choices here are proposals that the specification leaves open, awaiting the author:
//! - **One 2D average.** Along the row the weighted sum of the values present and the sum of
//!   their weights are kept apart; down the column both are combined and divided once. The result
//!   is the 3 x 3 average of the values present, rather than an average rescaled after each pass.
//! - **Stored NaN is missing.** On this path any NaN counts as missing, so a NaN stored in float
//!   data at a pixel the mask calls valid is skipped and filled like a masked one.
//! - **`valid_fraction` by filter weight.** The share of contributing pixels that were valid is
//!   weighted by the filter.
//! - **`valid` follows the averaged mask** whatever kind of Slice it is read beside, so beside
//!   labels or range it can differ from where a reduced value exists.

use super::{
    axis_plan_radius, blur5, blur5_over, blur5_row, blur5_row_weighted, blur5_weighted,
    convert_row, grid_len, halve, halve_over, halve_row, halve_row_weighted, halve_weighted, hsv0,
    hue_from, keep_present, mask_planes1, mask_planes4, mask_planes5, min3_present, min3_row,
    phase_from, polar0, present_one, reflect, weight_present, Func, Image, Reduce, Samples, Slice,
    Window,
};
use crate::simd::Isa;

/// The grid of a request: for each Level that makes anything, the rows and columns it outputs
/// and the rows and columns it must compute so that the next Level can be made.
struct Plan {
    row_out: Vec<Option<(i64, i64)>>,
    col_out: Vec<Option<(i64, i64)>>,
    row_need: Vec<(i64, i64)>,
    col_need: Vec<(i64, i64)>,
}

impl Plan {
    fn new(img: &Image, window: Window, top: u32, radius: i64) -> Plan {
        assert!(
            (top as usize) < MAX_LEVELS,
            "at most {MAX_LEVELS} Levels, Level 0 included"
        );
        let (row_out, row_need) = axis_plan_radius(
            img.rows,
            window.row_start,
            window.row_start + window.row_count - 1,
            top,
            radius,
        );
        let (col_out, col_need) = axis_plan_radius(
            img.cols,
            window.col_start,
            window.col_start + window.col_count - 1,
            top,
            radius,
        );
        // A Level is needed only while some Level at or above it has output, so the needed
        // Levels are a prefix, as in `read_levels`.
        let levels = row_need
            .iter()
            .zip(&col_need)
            .take_while(|(r, c)| r.is_some() && c.is_some())
            .count();
        Plan {
            row_out: row_out[..levels].to_vec(),
            col_out: col_out[..levels].to_vec(),
            row_need: row_need[..levels].iter().map(|r| r.unwrap()).collect(),
            col_need: col_need[..levels].iter().map(|c| c.unwrap()).collect(),
        }
    }
    fn levels(&self) -> usize {
        self.row_need.len()
    }
}

/// The working rows of one Level.
struct LevelState {
    /// Columns this Level computes.
    cols: (i64, i64),
    /// Rows this Level computes.
    rows: (i64, i64),
    /// The next row of this Level to make.
    next: i64,
    /// Grid length of the Level before, along rows, for mirroring at the Image edge.
    prev_rows: i64,
    /// For each tap of the halving, each output column's source column in the Level before,
    /// relative to the first column that Level computes. Empty for Level 0.
    gather: Vec<Vec<usize>>,
    /// The output columns `lo..hi` whose sources need no mirroring, and the source column of the
    /// first one's left neighbor. Their sources are read from the row itself.
    interior: (usize, usize, usize),
    /// Rows of the Level before, halved along the row, in one slot for each tap: one plane for
    /// each carried plane. With missing data they are weighted sums, not yet divided. A row of
    /// this Level is made the moment the last of its source rows arrives, and its oldest source
    /// is then at most taps - 1 rows back, so that many slots are enough. Each use checks that
    /// the row it reads is the one it wants.
    ring: Vec<Vec<f32>>,
    /// With missing data, the weight of each value of the current row: 0 where it is missing.
    /// Every plane of a pixel is present or missing together, so one row serves them all.
    count: Vec<f32>,
    /// With missing data, the weights of the Level before, halved along the row, in the same
    /// slots.
    ring_count: Vec<f32>,
    /// Which row of the Level before each ring slot holds.
    tags: Vec<i64>,
    /// The current row of this Level: carried planes, then Level 0's extra planes, which every
    /// Level keeps when values are kept rather than averaged.
    row: Vec<Vec<f32>>,
}

impl LevelState {
    fn width(&self) -> usize {
        (self.cols.1 - self.cols.0 + 1) as usize
    }
}

/// One planned read: the grid, and every working row it needs, allocated once. Running it again
/// on the same Image shape allocates nothing.
pub struct Pass {
    func: Func,
    /// Taps of an averaging halving: 3 for `bilinear`, 5 for `binomial5`. Kinds that are not
    /// averaged, and `decimate`, use the three of the 3 × 3 block.
    taps: usize,
    rows: usize,
    cols: usize,
    masked: bool,
    plan: Plan,
    levels: Vec<LevelState>,
    /// Gathered sources at the two ends of a row, where the Image edge mirrors them, one row for
    /// each tap.
    edge: Vec<Vec<f32>>,
    /// The native samples of one row, converted to f32 component rows.
    inputs: Vec<Vec<f32>>,
    /// Gathered weights at the two ends of a row, with missing data.
    edge_count: Vec<Vec<f32>>,
}

/// How a Level weights the values below it where some are missing (section 5 of the
/// specification).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MissingWeights {
    /// Each present value below counts with its filter weight alone.
    #[default]
    Equal,
    /// Each present value below counts with its filter weight times the share of present
    /// full-resolution pixels behind it, as if the Level were made from full resolution at once.
    PixelCount,
}

/// The level method (section 5 of the specification).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LevelMethod {
    /// Weights 1 2 1 at each halving, for every kind of Slice that is averaged.
    #[default]
    Bilinear,
    /// Weights 1 4 6 4 1 at each halving, radius 2, for every kind of Slice that is averaged.
    /// Labels and range keep their own reductions.
    Binomial5,
    /// The value at each point of the reduced grid, for every kind of Slice.
    Decimate,
}

/// The options of a read that concern this experiment, as `SliceOptions` holds them.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReadOptions {
    pub level_method: LevelMethod,
    /// A number for missing positions, in place of NaN.
    pub missing_value: Option<f32>,
    pub missing_weights: MissingWeights,
}

impl LevelMethod {
    /// Taps of each halving this method makes for `func`: five for `binomial5` where the Slices
    /// are averaged, and otherwise the three of the 3 × 3 block that `bilinear`, range and
    /// keeping use.
    pub(crate) fn taps(self, func: Func) -> usize {
        match (self, func.reduce()) {
            (LevelMethod::Binomial5, Reduce::Average) => 5,
            _ => 3,
        }
    }
}

impl Pass {
    /// Plans a read of `func` for `window` at Levels 0 to `top`, as `read_levels` makes it, for
    /// Images of `img`'s shape, with or without a validity mask as `img` has, for any level
    /// method of three taps: `bilinear`, `decimate`, and `binomial5` on kinds that are not
    /// averaged.
    pub fn new(img: &Image, func: Func, window: Window, top: u32) -> Pass {
        Pass::for_method(img, func, window, top, LevelMethod::Bilinear)
    }

    /// As `new`, for a level method. `binomial5` reaches two grid points to each side, so its
    /// margins are wider and each Level holds five rows; a read is planned for its method, as
    /// `plan` is given the whole request.
    pub fn for_method(
        img: &Image,
        func: Func,
        window: Window,
        top: u32,
        method: LevelMethod,
    ) -> Pass {
        let taps = method.taps(func);
        let radius = (taps / 2) as i64;
        let plan = Plan::new(img, window, top, radius);
        let masked = img.valid.is_some();
        let (carried, extra) = func.level0_planes();
        let levels: Vec<LevelState> = (0..plan.levels())
            .map(|k| {
                let cols = plan.col_need[k];
                let w = (cols.1 - cols.0 + 1) as usize;
                let gather = if k == 0 {
                    vec![Vec::new(); taps]
                } else {
                    let jc = grid_len(img.cols, k as u32 - 1);
                    let first = plan.col_need[k - 1].0;
                    // The center source is always on the grid; the others are mirrored at the
                    // Image edge.
                    (0..taps as i64)
                        .map(|t| {
                            (cols.0..=cols.1)
                                .map(|j| {
                                    let i = 2 * j + t - radius;
                                    let i = if t == radius { i } else { reflect(i, jc) };
                                    (i - first) as usize
                                })
                                .collect()
                        })
                        .collect()
                };
                let interior = interior_of(&gather);
                let ring_planes = if k == 0 { 0 } else { carried };
                let count_len = if masked { w } else { 0 };
                let ring_count_len = if masked && k > 0 { taps * w } else { 0 };
                LevelState {
                    cols,
                    interior,
                    rows: plan.row_need[k],
                    next: plan.row_need[k].0,
                    prev_rows: if k == 0 {
                        0
                    } else {
                        grid_len(img.rows, k as u32 - 1)
                    },
                    gather,
                    ring: vec![vec![0f32; taps * w]; ring_planes],
                    count: vec![0f32; count_len],
                    ring_count: vec![0f32; ring_count_len],
                    tags: vec![i64::MIN; taps],
                    row: vec![vec![0f32; w]; carried + extra],
                }
            })
            .collect();
        let widest = levels.iter().map(LevelState::width).max().unwrap();
        Pass {
            func,
            taps,
            rows: img.rows,
            cols: img.cols,
            masked,
            plan,
            levels,
            edge: vec![vec![0f32; widest]; taps],
            inputs: vec![vec![0f32; widest]; func.inputs().max(components(img))],
            edge_count: vec![vec![0f32; widest]; taps],
        }
    }

    /// Allocates the Slices this read makes, in the same shape `read_levels` returns, for the
    /// caller to keep and reuse.
    pub fn outputs(&self) -> Vec<Vec<Slice>> {
        let plan = &self.plan;
        (0..plan.levels())
            .map(|k| match (plan.row_out[k], plan.col_out[k]) {
                (Some(rows), Some(cols)) => {
                    let size = ((rows.1 - rows.0 + 1) * (cols.1 - cols.0 + 1)) as usize;
                    self.func
                        .slice_names()
                        .iter()
                        .map(|&name| Slice {
                            name,
                            level: k as u32,
                            rows,
                            cols,
                            data: vec![0f32; size],
                        })
                        .collect()
                }
                _ => Vec::new(),
            })
            .collect()
    }

    /// Runs the read into `out`, which must have the shape `outputs` gives. Missing positions
    /// are NaN, or the `missing_value` of `options` where the caller gives one.
    pub fn run(&mut self, img: &Image, isa: Isa, options: ReadOptions, out: &mut [Vec<Slice>]) {
        self.check_shape(out);
        let mut views = empty_views();
        for (v, level) in views.iter_mut().zip(out.iter_mut()) {
            for (d, s) in v.iter_mut().zip(level.iter_mut()) {
                *d = &mut s.data[..];
            }
        }
        self.run_views(img, isa, options, &mut views);
    }

    /// `run`, into the rows of each Slice that this read's Window owns, given as views: for each
    /// Level, one view for each Slice, holding the rows of `row_out` and the columns of `col_out`.
    fn run_views(&mut self, img: &Image, isa: Isa, options: ReadOptions, views: &mut Views) {
        assert!(
            self.plan.levels() <= MAX_LEVELS,
            "more Levels than a read can hold"
        );
        assert!(
            isa == Isa::Scalar || Isa::available().contains(&isa),
            "{isa:?} is not available"
        );
        assert!(
            img.rows == self.rows && img.cols == self.cols && img.valid.is_some() == self.masked,
            "the Image differs from the one planned for"
        );
        assert_eq!(
            options.level_method.taps(self.func),
            self.taps,
            "the read was planned for another level method"
        );
        for level in &mut self.levels {
            level.next = level.rows.0;
            level.tags.fill(i64::MIN);
        }
        let func = self.func;
        let reduce = match options.level_method {
            LevelMethod::Decimate => Reduce::Keep,
            LevelMethod::Bilinear | LevelMethod::Binomial5 => func.reduce(),
        };
        let (r0, r1) = self.levels[0].rows;
        let (c0, c1) = self.levels[0].cols;
        let n = self.levels[0].width();
        for r in r0..=r1 {
            let row = &mut self.levels[0].row;
            match func {
                // Converted straight into the Level 0 row.
                Func::Raw | Func::Labels | Func::Range => {
                    convert_row(img, r as usize, c0 as usize, c1 as usize, &mut row[..1])
                }
                // The mask, 1.0 where present and 0.0 where missing, averaged as it is. A sample
                // that is not a number counts as missing, as on the weighted path.
                Func::Mask => {
                    match &img.valid {
                        Some(mask) => {
                            let at = r as usize * img.cols;
                            let bytes = &mask[at + c0 as usize..=at + c1 as usize];
                            for (v, &b) in row[0].iter_mut().zip(bytes) {
                                *v = f32::from(u8::from(b != 0));
                            }
                        }
                        None => row[0].fill(1.0),
                    }
                    // As on the weighted path, NaN is looked for only where the Image can have
                    // missing pixels, and only float samples can hold one.
                    if img.valid.is_some()
                        && matches!(img.samples, Samples::F32(_) | Samples::ComplexF32(_))
                    {
                        let k = components(img);
                        convert_row(
                            img,
                            r as usize,
                            c0 as usize,
                            c1 as usize,
                            &mut self.inputs[..k],
                        );
                        let w = row[0].as_mut_ptr();
                        for x in &self.inputs[..k] {
                            // The sample rows and the mask row hold n values; the mask row is
                            // read before it is written.
                            unsafe { weight_present(isa, n, x.as_ptr(), w, w) };
                        }
                    }
                }
                Func::Polar => {
                    convert_row(img, r as usize, c0 as usize, c1 as usize, &mut self.inputs);
                    let i = &self.inputs;
                    // Every input and output row holds n values.
                    unsafe {
                        polar0(
                            isa,
                            n,
                            i[0].as_ptr(),
                            i[1].as_ptr(),
                            [
                                row[0].as_mut_ptr(),
                                row[1].as_mut_ptr(),
                                row[2].as_mut_ptr(),
                                row[3].as_mut_ptr(),
                            ],
                        )
                    }
                }
                Func::Hsv => {
                    convert_row(img, r as usize, c0 as usize, c1 as usize, &mut self.inputs);
                    let i = &self.inputs;
                    // Every input and output row holds n values.
                    unsafe {
                        hsv0(
                            isa,
                            n,
                            i[0].as_ptr(),
                            i[1].as_ptr(),
                            i[2].as_ptr(),
                            [
                                row[0].as_mut_ptr(),
                                row[1].as_mut_ptr(),
                                row[2].as_mut_ptr(),
                                row[3].as_mut_ptr(),
                                row[4].as_mut_ptr(),
                            ],
                        )
                    }
                }
            }
            if let (Some(mask), true) = (&img.valid, self.weighted()) {
                // Each pixel's weight is 1 where present and 0 where missing, and every plane of
                // a missing pixel is NaN, whatever its kernel made of the samples.
                let level0 = &mut self.levels[0];
                let at = r as usize * img.cols;
                let bytes = &mask[at + c0 as usize..=at + c1 as usize];
                for (v, &b) in level0.count.iter_mut().zip(bytes) {
                    *v = if b == 0 { 0.0 } else { 1.0 };
                }
                // A value that is not a number is missing too, in every plane of its pixel. One
                // pass reads every plane once, sets the weight and makes the pixel NaN.
                let c = level0.count.as_mut_ptr();
                let mut p = [std::ptr::null_mut(); 5];
                for (d, plane) in p.iter_mut().zip(level0.row.iter_mut()) {
                    *d = plane.as_mut_ptr();
                }
                // The weight row and every plane hold n values; each is read before it is
                // written, so every row is both input and output.
                unsafe {
                    match level0.row.len() {
                        1 => mask_planes1(isa, n, [c, p[0]]),
                        4 => mask_planes4(isa, n, [c, p[0], p[1], p[2], p[3]]),
                        5 => mask_planes5(isa, n, [c, p[0], p[1], p[2], p[3], p[4]]),
                        _ => unreachable!("every function has one, four or five planes"),
                    }
                }
            }
            self.levels[0].next = r + 1;
            self.emit(isa, options, reduce, 0, r, views);
        }
        for (k, level) in self.levels.iter().enumerate() {
            assert_eq!(level.next, level.rows.1 + 1, "Level {k} was not finished");
        }
    }

    /// Whether missing values are NaN and skipped. The mask Slices are made from the mask itself,
    /// so they have no missing values.
    fn weighted(&self) -> bool {
        self.masked && self.func != Func::Mask
    }

    fn check_shape(&self, out: &[Vec<Slice>]) {
        let plan = &self.plan;
        assert_eq!(
            out.len(),
            plan.levels(),
            "output shape differs from the plan"
        );
        for (k, slices) in out.iter().enumerate() {
            let expected = match (plan.row_out[k], plan.col_out[k]) {
                (Some(_), Some(_)) => self.func.slice_names().len(),
                _ => 0,
            };
            assert_eq!(slices.len(), expected, "output shape differs at Level {k}");
            for s in slices {
                assert!(
                    Some(s.rows) == plan.row_out[k] && Some(s.cols) == plan.col_out[k],
                    "output grid differs at Level {k}"
                );
            }
        }
    }

    /// Row `r` of Level `k` is in `levels[k].row`. Writes the part of it the Window shows,
    /// passes it to Level k + 1, and makes every row of the Levels above that it completes.
    fn emit(
        &mut self,
        isa: Isa,
        options: ReadOptions,
        reduce: Reduce,
        k: usize,
        r: i64,
        views: &mut Views,
    ) {
        let fill = options.missing_value;
        write_out(
            self.func,
            isa,
            self.weighted(),
            reduce == Reduce::Keep,
            fill,
            &self.plan,
            &self.levels[k],
            k,
            r,
            &mut views[k],
        );
        if k + 1 >= self.levels.len() {
            return;
        }
        let (carried, extra) = self.func.level0_planes();
        let masked = self.weighted();
        if reduce == Reduce::Keep {
            // Every second value of every second row, with nothing averaged: the next Level's
            // row i is this Level's row 2i at the even columns.
            let i = {
                let (here, above) = self.levels.split_at_mut(k + 1);
                let (cur, next) = (&here[k], &mut above[0]);
                if next.next > next.rows.1 || r != 2 * next.next {
                    return;
                }
                let even = &next.gather[self.taps / 2];
                for p in 0..carried + extra {
                    for (d, &j) in next.row[p].iter_mut().zip(even) {
                        *d = cur.row[p][j];
                    }
                }
                next.next += 1;
                next.next - 1
            };
            return self.emit(isa, options, reduce, k + 1, i, views);
        }
        if reduce == Reduce::Min3 {
            return self.emit_min3(isa, options, k, r, views);
        }
        let taps = self.taps;
        let radius = (taps / 2) as i64;
        let (here, above) = self.levels.split_at_mut(k + 1);
        let (cur, next) = (&here[k], &mut above[0]);

        // Halve this row along the row into the next Level's ring. With missing data the
        // weights are halved first, once for every plane, and each plane is weighted by them.
        let w = next.width();
        let slot = r.rem_euclid(taps as i64) as usize;
        let (lo, hi, first) = next.interior;
        let span = 2 * (hi - lo) + taps - 2;
        if masked {
            let src = &cur.count;
            let dst = next.ring_count[slot * w..].as_mut_ptr();
            for (i0, i1) in [(0, lo), (hi, w)] {
                for i in i0..i1 {
                    for t in 0..taps {
                        self.edge_count[t][i] = src[next.gather[t][i]];
                    }
                }
                if i1 > i0 {
                    let x = taps_at(&self.edge_count, i0);
                    // The edge buffers hold at least w values and the ring slot w, and i0..i1
                    // lies inside both.
                    unsafe { along(isa, taps, i1 - i0, x, dst.add(i0)) };
                }
            }
            if hi > lo {
                let from = src[first..first + span].as_ptr();
                // `from` holds 2(hi - lo) + taps - 2 values, and lo..hi lies inside the ring slot.
                unsafe { along_row(isa, taps, hi - lo, from, dst.add(lo)) };
            }
        }
        for p in 0..carried {
            let src = &cur.row[p];
            let dst = next.ring[p][slot * w..].as_mut_ptr();
            // The two ends, where a source may be mirrored, are gathered by index.
            for (i0, i1) in [(0, lo), (hi, w)] {
                if i0 == i1 {
                    continue;
                }
                for i in i0..i1 {
                    for t in 0..taps {
                        self.edge[t][i] = src[next.gather[t][i]];
                    }
                }
                let x = taps_at(&self.edge, i0);
                // The edge buffers hold at least w values and each ring slot w, and i0..i1 lies
                // inside both.
                unsafe {
                    if masked {
                        let c = taps_at(&self.edge_count, i0);
                        along_weighted(isa, taps, i1 - i0, x, c, dst.add(i0));
                    } else {
                        along(isa, taps, i1 - i0, x, dst.add(i0));
                    }
                }
            }
            // The interior is read from the row itself: 2(hi - lo) + taps - 2 values from `first`.
            if hi > lo {
                let from = src[first..first + span].as_ptr();
                // `from` and the weights hold that many values from `first`, and lo..hi lies
                // inside each ring slot.
                unsafe {
                    if masked {
                        let weights = cur.count[first..first + span].as_ptr();
                        along_row_weighted(isa, taps, hi - lo, from, weights, dst.add(lo));
                    } else {
                        along_row(isa, taps, hi - lo, from, dst.add(lo));
                    }
                }
            }
        }
        next.tags[slot] = r;

        // Make every row of the next Level whose source rows are now in the ring. Each row made
        // is passed on at once, so the loop takes the next Level afresh on every turn.
        loop {
            let i = {
                let next = &mut self.levels[k + 1];
                if next.next > next.rows.1 {
                    return;
                }
                let i = next.next;
                let mut src = [0i64; 5];
                for (t, s) in src.iter_mut().enumerate().take(taps) {
                    let row = 2 * i + t as i64 - radius;
                    *s = if t as i64 == radius {
                        row
                    } else {
                        reflect(row, next.prev_rows)
                    };
                }
                if src[..taps].iter().any(|&s| s > r) {
                    return;
                }
                let mut slots = [0usize; 5];
                for t in 0..taps {
                    let slot = src[t].rem_euclid(taps as i64) as usize;
                    assert_eq!(
                        next.tags[slot], src[t],
                        "row {} left the ring too early",
                        src[t]
                    );
                    slots[t] = slot;
                }
                let rows_of = |ring: &[f32]| {
                    let mut x = [std::ptr::null(); 5];
                    for t in 0..taps {
                        x[t] = ring[slots[t] * w..].as_ptr();
                    }
                    x
                };
                if masked {
                    // The weights of this row first: they divide every plane.
                    let x = rows_of(&next.ring_count);
                    let wt = next.count.as_mut_ptr();
                    // Each ring slot and the weight row hold w values.
                    unsafe { along(isa, taps, w, x, wt) };
                }
                for p in 0..carried {
                    let out = next.row[p].as_mut_ptr();
                    let x = rows_of(&next.ring[p]);
                    // Each ring slot, the weight row and the output row hold w values.
                    unsafe {
                        if masked {
                            over(isa, taps, w, x, next.count.as_ptr(), out);
                        } else {
                            along(isa, taps, w, x, out);
                        }
                    }
                }
                if masked && options.missing_weights == MissingWeights::Equal {
                    // Each present value counts the same at the next halving.
                    let wt = next.count.as_mut_ptr();
                    // The weight row holds w values, read before written.
                    unsafe { present_one(isa, w, wt, wt) };
                }
                next.next = i + 1;
                i
            };
            self.emit(isa, options, reduce, k + 1, i, views);
        }
    }

    /// As `emit`'s averaging, for range: the least present value of each 3 × 3 block, along the
    /// row into the ring and then down the column.
    fn emit_min3(&mut self, isa: Isa, options: ReadOptions, k: usize, r: i64, views: &mut Views) {
        {
            let (here, above) = self.levels.split_at_mut(k + 1);
            let (cur, next) = (&here[k], &mut above[0]);
            let w = next.width();
            let slot = r.rem_euclid(3) as usize;
            let (lo, hi, first) = next.interior;
            let src = &cur.row[0];
            let dst = next.ring[0][slot * w..].as_mut_ptr();
            for (i0, i1) in [(0, lo), (hi, w)] {
                for i in i0..i1 {
                    for t in 0..3 {
                        self.edge[t][i] = src[next.gather[t][i]];
                    }
                }
                if i1 > i0 {
                    let [a, b, c] = [0, 1, 2].map(|t| self.edge[t][i0..].as_ptr());
                    // The edge buffers hold at least w values and the ring slot w, and i0..i1
                    // lies inside both.
                    unsafe { min3_present(isa, i1 - i0, a, b, c, dst.add(i0)) };
                }
            }
            if hi > lo {
                let from = src[first..first + 2 * (hi - lo) + 1].as_ptr();
                // `from` holds 2(hi - lo) + 1 values, and lo..hi lies inside the ring slot.
                unsafe { min3_row(isa, hi - lo, from, dst.add(lo)) };
            }
            next.tags[slot] = r;
        }
        loop {
            let i = {
                let next = &mut self.levels[k + 1];
                let w = next.width();
                if next.next > next.rows.1 {
                    return;
                }
                let i = next.next;
                let src = [
                    reflect(2 * i - 1, next.prev_rows),
                    2 * i,
                    reflect(2 * i + 1, next.prev_rows),
                ];
                if src.iter().any(|&s| s > r) {
                    return;
                }
                let [a, b, c] = src.map(|s| {
                    let slot = s.rem_euclid(3) as usize;
                    assert_eq!(next.tags[slot], s, "row {s} left the ring too early");
                    next.ring[0][slot * w..].as_ptr()
                });
                // Each ring slot and the output row hold w values.
                unsafe { min3_present(isa, w, a, b, c, next.row[0].as_mut_ptr()) };
                next.next = i + 1;
                i
            };
            self.emit(isa, options, Reduce::Min3, k + 1, i, views);
        }
    }
}

/// The most bands a read is split into.
const MAX_BANDS: usize = 16;

/// A read split across threads by bands of rows (section 12, rule 7).
///
/// Each band is a Window of whole rows of the request's Window, and every band but the first
/// starts on a row that is a multiple of 2^top, so every grid point of every Level falls in
/// exactly one band. Each band runs its own `Pass`, with its own rings, and writes straight into
/// its own rows of the caller's Slices. The rows a band needs from beyond its edges, for the
/// halvings, are made by both neighbors. Window independence makes every value the one a single
/// `Pass` makes, and the checks confirm it bit for bit.
///
/// The working rows of every band are planned once. Starting the threads allocates for each read;
/// a pool kept between reads would remove that. The views of every band are held on the calling
/// thread's stack, about 48 KB.
pub struct BandedPass {
    whole: Pass,
    bands: Vec<Pass>,
}

impl BandedPass {
    /// Plans a read as `Pass::for_method` does, split into at most `threads` bands, never more
    /// than 16, and fewer where the Window has fewer rows of Level `top`.
    pub fn new(
        img: &Image,
        func: Func,
        window: Window,
        top: u32,
        method: LevelMethod,
        threads: usize,
    ) -> BandedPass {
        let whole = Pass::for_method(img, func, window, top, method);
        let first = window.row_start;
        let end = window.row_start + window.row_count;
        // `Pass::for_method` has checked that `top` is below `MAX_LEVELS`, so the shift is sound.
        let step = 1usize << top;
        let mut starts = vec![first];
        for j in 1..threads.clamp(1, MAX_BANDS) {
            let target = first + window.row_count * j / threads.clamp(1, MAX_BANDS);
            let start = target.div_ceil(step).saturating_mul(step);
            if start > *starts.last().unwrap() && start < end {
                starts.push(start);
            }
        }
        starts.push(end);
        let bands = starts
            .windows(2)
            .map(|b| {
                let band = Window {
                    row_start: b[0],
                    row_count: b[1] - b[0],
                    ..window
                };
                Pass::for_method(img, func, band, top, method)
            })
            .collect();
        BandedPass { whole, bands }
    }

    /// How many bands, and so threads, a read uses.
    pub fn band_count(&self) -> usize {
        self.bands.len()
    }

    /// Allocates the Slices this read makes, as `Pass::outputs` does for the whole Window.
    pub fn outputs(&self) -> Vec<Vec<Slice>> {
        self.whole.outputs()
    }

    /// Runs every band on its own thread, the first on the calling thread, into `out`, which must
    /// have the shape `outputs` gives.
    pub fn run(&mut self, img: &Image, isa: Isa, options: ReadOptions, out: &mut [Vec<Slice>]) {
        self.whole.check_shape(out);
        let mut views: [Views; MAX_BANDS] = std::array::from_fn(|_| empty_views());
        for (k, level) in out.iter_mut().enumerate() {
            for (s, slice) in level.iter_mut().enumerate() {
                let w = slice.width();
                let mut rest: &mut [f32] = &mut slice.data[..];
                for (band, view) in self.bands.iter().zip(views.iter_mut()) {
                    let plan = &band.plan;
                    let rows = match (plan.row_out.get(k), plan.col_out.get(k)) {
                        (Some(Some(r)), Some(Some(_))) => (r.1 - r.0 + 1) as usize,
                        _ => 0,
                    };
                    let (mine, after) = std::mem::take(&mut rest).split_at_mut(rows * w);
                    view[k][s] = mine;
                    rest = after;
                }
                assert!(rest.is_empty(), "the bands do not cover Level {k}");
            }
        }
        std::thread::scope(|scope| {
            let mut work = self.bands.iter_mut().zip(views.iter_mut());
            let (first, first_views) = work.next().expect("a read has at least one band");
            for (pass, view) in work {
                scope.spawn(move || pass.run_views(img, isa, options, view));
            }
            first.run_views(img, isa, options, first_views);
        });
    }
}

/// Allocates the Slices a request makes, in the same shape `read_levels` returns, so that a
/// caller can allocate once and reuse the buffers for every read of the same shape.
pub fn plan_outputs(img: &Image, func: Func, window: Window, top: u32) -> Vec<Vec<Slice>> {
    Pass::new(img, func, window, top).outputs()
}

/// Makes every Slice of `func` for `window` at Levels 0 to `top`, as `read_levels` does, into
/// `out`, which must have the shape `plan_outputs` gives. A caller reading many times keeps a
/// `Pass` instead, so that nothing is allocated per read.
pub fn read_levels_into(
    img: &Image,
    func: Func,
    isa: Isa,
    window: Window,
    top: u32,
    options: ReadOptions,
    out: &mut [Vec<Slice>],
) {
    Pass::for_method(img, func, window, top, options.level_method).run(img, isa, options, out);
}

/// The components each pixel's samples hold: two for complex samples, else the Image's count.
fn components(img: &Image) -> usize {
    match img.samples {
        Samples::ComplexI16(_) | Samples::ComplexF32(_) => 2,
        _ => img.comps,
    }
}

/// Finds the run of output columns whose sources are consecutive columns, unmirrored, and the
/// source column of the first one's first tap. Mirroring happens only at the two ends, so the run
/// is one stretch of the row.
fn interior_of(gather: &[Vec<usize>]) -> (usize, usize, usize) {
    let taps = gather.len();
    let radius = taps / 2;
    let center = &gather[radius];
    let plain =
        |i: usize| center[i] >= radius && (0..taps).all(|t| gather[t][i] + radius == center[i] + t);
    let w = center.len();
    let lo = (0..w).find(|&i| plain(i)).unwrap_or(w);
    let hi = (lo..w).find(|&i| !plain(i)).unwrap_or(w);
    let first = if lo < hi { gather[0][lo] } else { 0 };
    (lo, hi, first)
}

/// The rows of `bufs`, one for each tap, from index `at`.
fn taps_at(bufs: &[Vec<f32>], at: usize) -> [*const f32; 5] {
    let mut x = [std::ptr::null(); 5];
    for (p, b) in x.iter_mut().zip(bufs) {
        *p = b[at..].as_ptr();
    }
    x
}

/// One halving of `n` values across `taps` sources: `halve` or `blur5`.
///
/// # Safety
/// The first `taps` sources and `out` hold `n` values.
unsafe fn along(isa: Isa, taps: usize, n: usize, x: [*const f32; 5], out: *mut f32) {
    debug_assert!(taps == 3 || taps == 5, "a halving has three or five taps");
    unsafe {
        if taps == 3 {
            halve(isa, n, x[0], x[1], x[2], out)
        } else {
            blur5(isa, n, x, out)
        }
    }
}

/// One halving along a row read from the row itself: `halve_row` or `blur5_row`.
///
/// # Safety
/// `src` holds `2n + taps - 2` values and `out` holds `n`.
unsafe fn along_row(isa: Isa, taps: usize, n: usize, src: *const f32, out: *mut f32) {
    debug_assert!(taps == 3 || taps == 5, "a halving has three or five taps");
    unsafe {
        if taps == 3 {
            halve_row(isa, n, src, out)
        } else {
            blur5_row(isa, n, src, out)
        }
    }
}

/// The weighted sum of a halving with missing data: `halve_weighted` or `blur5_weighted`.
///
/// # Safety
/// The first `taps` values and weights and `out` hold `n` values.
unsafe fn along_weighted(
    isa: Isa,
    taps: usize,
    n: usize,
    x: [*const f32; 5],
    c: [*const f32; 5],
    out: *mut f32,
) {
    debug_assert!(taps == 3 || taps == 5, "a halving has three or five taps");
    unsafe {
        if taps == 3 {
            halve_weighted(isa, n, [x[0], x[1], x[2]], [c[0], c[1], c[2]], out)
        } else {
            blur5_weighted(isa, n, x, c, out)
        }
    }
}

/// `along_weighted` read from the rows themselves: `halve_row_weighted` or `blur5_row_weighted`.
///
/// # Safety
/// `x` and `c` hold `2n + taps - 2` values and `out` holds `n`.
unsafe fn along_row_weighted(
    isa: Isa,
    taps: usize,
    n: usize,
    x: *const f32,
    c: *const f32,
    out: *mut f32,
) {
    debug_assert!(taps == 3 || taps == 5, "a halving has three or five taps");
    unsafe {
        if taps == 3 {
            halve_row_weighted(isa, n, x, c, out)
        } else {
            blur5_row_weighted(isa, n, x, c, out)
        }
    }
}

/// The weighted sums combined down the column and divided: `halve_over` or `blur5_over`.
///
/// # Safety
/// The first `taps` sums, `w` and `out` hold `n` values.
unsafe fn over(isa: Isa, taps: usize, n: usize, s: [*const f32; 5], w: *const f32, out: *mut f32) {
    debug_assert!(taps == 3 || taps == 5, "a halving has three or five taps");
    unsafe {
        if taps == 3 {
            halve_over(isa, n, [s[0], s[1], s[2]], w, out)
        } else {
            blur5_over(isa, n, s, w, out)
        }
    }
}

/// Writes the columns the Window shows of row `r` of Level `k` into that Level's Slices.
#[allow(clippy::too_many_arguments)]
fn write_out(
    func: Func,
    isa: Isa,
    masked: bool,
    kept: bool,
    fill: Option<f32>,
    plan: &Plan,
    level: &LevelState,
    k: usize,
    r: i64,
    slices: &mut [&mut [f32]; 3],
) {
    let (Some(rows), Some(cols)) = (plan.row_out[k], plan.col_out[k]) else {
        return;
    };
    if r < rows.0 || r > rows.1 {
        return;
    }
    let w = (cols.1 - cols.0 + 1) as usize;
    let o = (cols.0 - level.cols.0) as usize;
    let at = ((r - rows.0) as usize) * w;
    let plane = |p: usize| &level.row[p][o..o + w];
    let mut put = |s: usize, p: usize| slices[s][at..at + w].copy_from_slice(plane(p));
    // An angle made from an averaged unit vector, NaN where the vector is missing.
    let angle = |slices: &mut [&mut [f32]; 3], s: usize, x: usize, y: usize, f: DeriveFn| {
        let dst = slices[s][at..at + w].as_mut_ptr();
        // Both planes and the output row hold w values.
        unsafe {
            f(isa, w, plane(x).as_ptr(), plane(y).as_ptr(), dst);
            if masked {
                keep_present(isa, w, plane(x).as_ptr(), dst, dst);
            }
        }
    };
    // Kept values are those of Level 0, so their angles are Level 0's too.
    let direct = k == 0 || kept;
    match func {
        Func::Raw | Func::Labels | Func::Range => put(0, 0),
        Func::Mask => {
            put(1, 0);
            let dst = slices[0][at..at + w].as_mut_ptr();
            // The plane and the output row hold w values.
            unsafe { present_one(isa, w, plane(0).as_ptr(), dst) };
        }
        Func::Polar => {
            put(0, 0);
            if direct {
                put(1, 3);
            } else {
                angle(slices, 1, 1, 2, phase_from);
            }
        }
        Func::Hsv => {
            put(1, 0);
            put(2, 1);
            if direct {
                put(0, 4);
            } else {
                angle(slices, 0, 2, 3, hue_from);
            }
        }
    }
    // The caller's number in place of NaN, applied last.
    if let (Some(v), true) = (fill, masked) {
        for s in slices.iter_mut().take(func.slice_names().len()) {
            for x in &mut s[at..at + w] {
                if x.is_nan() {
                    *x = v;
                }
            }
        }
    }
}

/// The most Levels one read makes: a halving of every Image dimension BIL can address.
const MAX_LEVELS: usize = 64;

/// Where a read writes: for each Level, one view for each of up to three Slices, each holding the
/// rows and columns the read's Window owns at that Level. Kept on the stack, so that a read
/// allocates nothing.
type Views<'a> = [[&'a mut [f32]; 3]; MAX_LEVELS];

/// Views that hold nothing, to be pointed at the Slices of a read.
fn empty_views<'a>() -> Views<'a> {
    std::array::from_fn(|_| -> [&'a mut [f32]; 3] { [&mut [], &mut [], &mut []] })
}

/// A kernel that makes one plane from two.
type DeriveFn = unsafe fn(Isa, usize, *const f32, *const f32, *mut f32);
