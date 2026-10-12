// SPDX-License-Identifier: LGPL-2.1-or-later
//! Experiment 1b: the same arithmetic as `read_levels`, with rows streamed through every Level
//! on a buffer of a few rows, and every Slice written straight into buffers the caller owns.
//!
//! Each Level above Level 0 keeps a ring of three rows, already halved along the row, from the
//! Level before. A row of Level k is made as soon as the three rows of Level k - 1 it needs are in
//! the ring, and it is written out at once and passed on to Level k + 1. No full-size
//! intermediate plane is made. The kernels and the order of operations are those of
//! `read_levels`, so every value has the same bits.

use super::{
    axis_plan, convert_row, grid_len, halve, hsv0, hue_from, phase_from, polar0, reflect, Func,
    Image, Slice, Window,
};
use crate::simd::Isa;

/// Rows of the Level before that each Level above Level 0 holds. A row of Level k + 1 is made
/// the moment the last of its three source rows arrives, and its oldest source is then at most
/// two rows back, so three are enough. Each use checks that the row it reads is the one it wants.
const RING: usize = 3;

/// The grid of a request: for each Level that makes anything, the rows and columns it outputs
/// and the rows and columns it must compute so that the next Level can be made.
struct Plan {
    row_out: Vec<Option<(i64, i64)>>,
    col_out: Vec<Option<(i64, i64)>>,
    row_need: Vec<(i64, i64)>,
    col_need: Vec<(i64, i64)>,
}

impl Plan {
    fn new(img: &Image, window: Window, top: u32) -> Plan {
        let (row_out, row_need) = axis_plan(
            img.rows,
            window.row_start,
            window.row_start + window.row_count - 1,
            top,
        );
        let (col_out, col_need) = axis_plan(
            img.cols,
            window.col_start,
            window.col_start + window.col_count - 1,
            top,
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

/// Allocates the Slices a request makes, in the same shape `read_levels` returns, so that a
/// caller can allocate once and reuse the buffers for every read of the same shape.
pub fn plan_outputs(img: &Image, func: Func, window: Window, top: u32) -> Vec<Vec<Slice>> {
    let plan = Plan::new(img, window, top);
    (0..plan.levels())
        .map(|k| match (plan.row_out[k], plan.col_out[k]) {
            (Some(rows), Some(cols)) => {
                let size = ((rows.1 - rows.0 + 1) * (cols.1 - cols.0 + 1)) as usize;
                func.slice_names()
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
    /// For each output column, the three source columns in the Level before, relative to the
    /// first column that Level computes. Empty for Level 0.
    gather: [Vec<usize>; 3],
    /// The output columns `lo..hi` whose sources need no mirroring, and the source column of the
    /// first one's left neighbor. Their sources are read in order rather than gathered.
    interior: (usize, usize, usize),
    /// Rows of the Level before, halved along the row: `RING` slots of `carried` planes.
    ring: Vec<Vec<f32>>,
    /// Which row of the Level before each ring slot holds.
    tags: [i64; RING],
    /// The current row of this Level: carried planes, then Level 0's extra planes.
    row: Vec<Vec<f32>>,
}

impl LevelState {
    fn width(&self) -> usize {
        (self.cols.1 - self.cols.0 + 1) as usize
    }
}

/// Makes every Slice of `func` for `window` at Levels 0 to `top`, as `read_levels` does, into
/// `out`, which must have the shape `plan_outputs` gives for the same arguments.
pub fn read_levels_into(
    img: &Image,
    func: Func,
    isa: Isa,
    window: Window,
    top: u32,
    out: &mut [Vec<Slice>],
) {
    assert!(
        isa == Isa::Scalar || Isa::available().contains(&isa),
        "{isa:?} is not available"
    );
    let plan = Plan::new(img, window, top);
    assert_eq!(
        out.len(),
        plan.levels(),
        "output shape differs from the plan"
    );
    for (k, slices) in out.iter().enumerate() {
        let expected = match (plan.row_out[k], plan.col_out[k]) {
            (Some(_), Some(_)) => func.slice_names().len(),
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

    let (carried, extra) = func.level0_planes();
    let mut levels: Vec<LevelState> = (0..plan.levels())
        .map(|k| {
            let cols = plan.col_need[k];
            let w = (cols.1 - cols.0 + 1) as usize;
            let planes = if k == 0 { carried + extra } else { carried };
            let gather = if k == 0 {
                [Vec::new(), Vec::new(), Vec::new()]
            } else {
                let jc = grid_len(img.cols, k as u32 - 1);
                let first = plan.col_need[k - 1].0;
                let idx = |i: i64| (reflect(i, jc) - first) as usize;
                [
                    (cols.0..=cols.1).map(|j| idx(2 * j - 1)).collect(),
                    (cols.0..=cols.1)
                        .map(|j| (2 * j - first) as usize)
                        .collect(),
                    (cols.0..=cols.1).map(|j| idx(2 * j + 1)).collect(),
                ]
            };
            let interior = interior_of(&gather);
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
                ring: if k == 0 {
                    Vec::new()
                } else {
                    vec![vec![0f32; RING * w]; carried]
                },
                tags: [i64::MIN; RING],
                row: vec![vec![0f32; w]; planes],
            }
        })
        .collect();

    let widest = levels.iter().map(LevelState::width).max().unwrap();
    let mut scratch = Scratch {
        a: vec![0f32; widest],
        b: vec![0f32; widest],
        c: vec![0f32; widest],
        even: vec![0f32; widest + 1],
        odd: vec![0f32; widest],
    };
    let mut inputs: Vec<Vec<f32>> = vec![Vec::new(); func.inputs()];
    let (r0, r1) = levels[0].rows;
    let (c0, c1) = levels[0].cols;
    let n = levels[0].width();
    for r in r0..=r1 {
        convert_row(img, r as usize, c0 as usize, c1 as usize, &mut inputs);
        let row = &mut levels[0].row;
        match func {
            Func::Raw => row[0].copy_from_slice(&inputs[0]),
            // Every input and output row holds n values.
            Func::Polar => unsafe {
                polar0(
                    isa,
                    n,
                    inputs[0].as_ptr(),
                    inputs[1].as_ptr(),
                    [
                        row[0].as_mut_ptr(),
                        row[1].as_mut_ptr(),
                        row[2].as_mut_ptr(),
                        row[3].as_mut_ptr(),
                    ],
                )
            },
            Func::Hsv => unsafe {
                hsv0(
                    isa,
                    n,
                    inputs[0].as_ptr(),
                    inputs[1].as_ptr(),
                    inputs[2].as_ptr(),
                    [
                        row[0].as_mut_ptr(),
                        row[1].as_mut_ptr(),
                        row[2].as_mut_ptr(),
                        row[3].as_mut_ptr(),
                        row[4].as_mut_ptr(),
                    ],
                )
            },
        }
        levels[0].next = r + 1;
        emit(
            func,
            isa,
            carried,
            &plan,
            &mut levels,
            0,
            r,
            out,
            &mut scratch,
        );
    }
    for (k, level) in levels.iter().enumerate() {
        assert_eq!(level.next, level.rows.1 + 1, "Level {k} was not finished");
    }
}

/// Buffers for halving along a row: gathered sources at the edges, and the even and odd
/// columns of the interior.
struct Scratch {
    a: Vec<f32>,
    b: Vec<f32>,
    c: Vec<f32>,
    even: Vec<f32>,
    odd: Vec<f32>,
}

/// Finds the run of output columns whose three sources are consecutive columns, unmirrored.
/// Mirroring happens only at the two ends, so the run is one stretch of the row.
fn interior_of(gather: &[Vec<usize>; 3]) -> (usize, usize, usize) {
    let [a, b, c] = gather;
    let plain = |i: usize| b[i] >= 1 && a[i] == b[i] - 1 && c[i] == b[i] + 1;
    let Some(lo) = (0..b.len()).find(|&i| plain(i)) else {
        return (0, 0, 0);
    };
    let hi = (lo..b.len()).find(|&i| !plain(i)).unwrap_or(b.len());
    assert!((hi..b.len()).all(|i| !plain(i)), "mirroring inside the row");
    (lo, hi, a[lo])
}

/// Row `r` of Level `k` is in `levels[k].row`. Writes the part of it the Window shows, passes it
/// to Level k + 1, and makes every row of the Levels above that it completes.
#[allow(clippy::too_many_arguments)]
fn emit(
    func: Func,
    isa: Isa,
    carried: usize,
    plan: &Plan,
    levels: &mut [LevelState],
    k: usize,
    r: i64,
    out: &mut [Vec<Slice>],
    scratch: &mut Scratch,
) {
    write_out(func, isa, plan, &levels[k], k, r, &mut out[k]);
    if k + 1 >= levels.len() {
        return;
    }
    let (here, above) = levels.split_at_mut(k + 1);
    let (cur, next) = (&here[k], &mut above[0]);

    // Halve this row along the row into the next Level's ring.
    let w = next.width();
    let slot = r.rem_euclid(RING as i64) as usize;
    let (lo, hi, first) = next.interior;
    let m = hi - lo;
    for p in 0..carried {
        let src = &cur.row[p];
        let dst = next.ring[p][slot * w..(slot + 1) * w].as_mut_ptr();
        // The two ends, where a source may be mirrored, are gathered by index.
        for (i0, i1) in [(0, lo), (hi, w)] {
            if i0 == i1 {
                continue;
            }
            for i in i0..i1 {
                scratch.a[i] = src[next.gather[0][i]];
                scratch.b[i] = src[next.gather[1][i]];
                scratch.c[i] = src[next.gather[2][i]];
            }
            // The gather buffers hold at least w values and the ring slot w, and i0..i1 lies
            // inside both.
            unsafe {
                halve(
                    isa,
                    i1 - i0,
                    scratch.a[i0..].as_ptr(),
                    scratch.b[i0..].as_ptr(),
                    scratch.c[i0..].as_ptr(),
                    dst.add(i0),
                )
            };
        }
        // The interior: the left source of column i is the even column, the center the odd one,
        // and the right one the next even column, so two in-order copies serve all three.
        if m > 0 {
            let run = &src[first..first + 2 * m + 1];
            for (j, pair) in run[..2 * m].as_chunks::<2>().0.iter().enumerate() {
                scratch.even[j] = pair[0];
                scratch.odd[j] = pair[1];
            }
            scratch.even[m] = run[2 * m];
            // even holds at least m + 1 values, odd at least m, and lo..hi is inside the ring slot.
            unsafe {
                halve(
                    isa,
                    m,
                    scratch.even.as_ptr(),
                    scratch.odd.as_ptr(),
                    scratch.even[1..].as_ptr(),
                    dst.add(lo),
                )
            };
        }
    }
    next.tags[slot] = r;

    // Make every row of the next Level whose three source rows are now in the ring. Each row
    // made is passed on at once, so the loop takes the next Level afresh on every turn.
    loop {
        let i = {
            let next = &mut levels[k + 1];
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
            let slots = src.map(|s| {
                let slot = s.rem_euclid(RING as i64) as usize;
                assert_eq!(next.tags[slot], s, "row {s} left the ring too early");
                slot
            });
            for p in 0..carried {
                let at = |slot: usize| next.ring[p][slot * w..].as_ptr();
                let (a, b, c) = (at(slots[0]), at(slots[1]), at(slots[2]));
                // Each ring slot and the output row hold w values.
                unsafe { halve(isa, w, a, b, c, next.row[p].as_mut_ptr()) };
            }
            next.next = i + 1;
            i
        };
        emit(func, isa, carried, plan, levels, k + 1, i, out, scratch);
    }
}

/// Writes the columns the Window shows of row `r` of Level `k` into that Level's Slices.
fn write_out(
    func: Func,
    isa: Isa,
    plan: &Plan,
    level: &LevelState,
    k: usize,
    r: i64,
    slices: &mut [Slice],
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
    let mut put = |s: usize, p: usize| slices[s].data[at..at + w].copy_from_slice(plane(p));
    match func {
        Func::Raw => put(0, 0),
        Func::Polar => {
            put(0, 0);
            if k == 0 {
                put(1, 3);
            } else {
                let dst = slices[1].data[at..at + w].as_mut_ptr();
                // Both planes and the output row hold w values.
                unsafe { phase_from(isa, w, plane(1).as_ptr(), plane(2).as_ptr(), dst) };
            }
        }
        Func::Hsv => {
            put(1, 0);
            put(2, 1);
            if k == 0 {
                put(0, 4);
            } else {
                let dst = slices[0].data[at..at + w].as_mut_ptr();
                // Both planes and the output row hold w values.
                unsafe { hue_from(isa, w, plane(2).as_ptr(), plane(3).as_ptr(), dst) };
            }
        }
    }
}
