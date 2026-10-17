// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Warnings held on the Source and drained by the caller (section 3.3): recorded as a code and
//! small structured context, kept once for each code, place and detail, bounded, and turned into
//! text only when asked for.

// With no reader compiled in, nothing makes a Source's store or cache.
#![cfg_attr(not(any(feature = "nitf", feature = "geotiff")), allow(dead_code))]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard};

use crate::cache::BlockKey;
use crate::types::Scope;

/// What a warning is about (section 3.3's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WarningCode {
    /// A block larger than `set_cache_bytes` was held alone; the detail is its bytes.
    CacheCeilingExceeded,
    /// A subdirectory that could not be read was skipped; the detail is its offset.
    SubdirectorySkipped,
    /// Under `corrupt_as_missing`, a corrupt block was delivered as missing.
    CorruptBlockMissing,
    /// A block larger than the cache's ceiling is decoded again by each read of part of it,
    /// as for an Image held in one strip; the detail is its bytes.
    BadlyPacked,
}

impl WarningCode {
    /// Its snake_case name.
    pub fn name(self) -> &'static str {
        match self {
            WarningCode::CacheCeilingExceeded => "cache_ceiling_exceeded",
            WarningCode::SubdirectorySkipped => "subdirectory_skipped",
            WarningCode::CorruptBlockMissing => "corrupt_block_missing",
            WarningCode::BadlyPacked => "badly_packed",
        }
    }
}

/// The blocks kept with each warning, the first it occurred on (section 3.4).
pub const BLOCKS_KEPT: usize = 4;

/// The default of `max_held_warnings` (section 3.4).
pub const DEFAULT_MAX_HELD: usize = 256;

/// One warning, however often it occurred.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub code: WarningCode,
    /// Where: the Source, or one Image.
    pub scope: Scope,
    /// The number that tells its cases apart, where the code has one.
    pub detail: Option<u64>,
    /// How many times it occurred.
    pub count: u64,
    /// The first blocks it occurred on, at most `BLOCKS_KEPT`.
    pub blocks: Vec<BlockKey>,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let place = match self.scope {
            Scope::Source => "the Source".to_string(),
            Scope::Image(i) => format!("image {i}"),
        };
        let detail = self.detail.unwrap_or(0);
        match self.code {
            WarningCode::CacheCeilingExceeded => write!(
                f,
                "{place}: a block of {detail} bytes, larger than the cache's ceiling, was held alone"
            )?,
            WarningCode::SubdirectorySkipped => write!(
                f,
                "{place}: the subdirectory at offset {detail} could not be read and was skipped"
            )?,
            WarningCode::BadlyPacked => write!(
                f,
                "WARNING: {place} is badly packed for reading in pieces: its blocks of {detail} \
                 bytes are larger than the cache's ceiling, so each read of part of one decodes \
                 it again. Raise set_cache_bytes above {detail}, or rewrite the file in tiles"
            )?,
            WarningCode::CorruptBlockMissing => {
                write!(f, "{place}: a corrupt block was delivered as missing")?
            }
        }
        if self.count > 1 {
            write!(f, " ({} times)", self.count)?;
        }
        Ok(())
    }
}

/// What `take_warnings` returns: the warnings held, in the order first met, and how many
/// distinct ones were dropped past `max_held_warnings`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Warnings {
    pub warnings: Vec<Warning>,
    pub dropped: u64,
}

type Key = (WarningCode, Scope, Option<u64>);

#[derive(Default)]
struct Held {
    warnings: Vec<Warning>,
    index: HashMap<Key, usize>,
    dropped: u64,
}

/// One Source's store. Its lock is taken only when a warning occurs or is drained.
pub(crate) struct WarningStore {
    max_held: usize,
    held: Mutex<Held>,
}

impl WarningStore {
    pub(crate) fn new(max_held: usize) -> WarningStore {
        WarningStore {
            max_held,
            held: Mutex::new(Held::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Records one occurrence.
    pub(crate) fn record(
        &self,
        code: WarningCode,
        scope: Scope,
        detail: Option<u64>,
        block: Option<BlockKey>,
    ) {
        let mut held = self.lock();
        let held = &mut *held;
        let key = (code, scope, detail);
        let at = match held.index.get(&key) {
            Some(&at) => at,
            None if held.warnings.len() < self.max_held => {
                held.index.insert(key, held.warnings.len());
                held.warnings.push(Warning {
                    code,
                    scope,
                    detail,
                    count: 0,
                    blocks: Vec::new(),
                });
                held.warnings.len() - 1
            }
            None => {
                held.dropped += 1;
                return;
            }
        };
        let w = &mut held.warnings[at];
        w.count += 1;
        if let Some(b) = block {
            if w.blocks.len() < BLOCKS_KEPT && !w.blocks.contains(&b) {
                w.blocks.push(b);
            }
        }
    }

    /// Returns what is held and empties the store.
    pub(crate) fn take(&self) -> Warnings {
        let held = std::mem::take(&mut *self.lock());
        Warnings {
            warnings: held.warnings,
            dropped: held.dropped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(c: u32) -> BlockKey {
        BlockKey {
            image: 0,
            level: 0,
            frame: 0,
            block_row: 0,
            block_col: c,
            components: (0, 1),
        }
    }

    #[test]
    fn repeats_are_one_entry_with_a_count_and_the_first_blocks() {
        let s = WarningStore::new(8);
        for c in 0..10 {
            s.record(
                WarningCode::CacheCeilingExceeded,
                Scope::Image(0),
                Some(4096),
                Some(block(c)),
            );
        }
        s.record(
            WarningCode::SubdirectorySkipped,
            Scope::Image(0),
            Some(800),
            None,
        );
        let w = s.take();
        assert_eq!((w.warnings.len(), w.dropped), (2, 0));
        assert_eq!(w.warnings[0].count, 10);
        assert_eq!(w.warnings[0].blocks, (0..4).map(block).collect::<Vec<_>>());
        assert_eq!(
            w.warnings[1].to_string(),
            "image 0: the subdirectory at offset 800 could not be read and was skipped"
        );
        assert!(w.warnings[0].to_string().ends_with("(10 times)"));
        // Drained: empty until the next.
        assert_eq!(s.take(), Warnings::default());
    }

    #[test]
    fn past_the_bound_distinct_warnings_are_counted_as_dropped() {
        let s = WarningStore::new(2);
        for offset in 0..5 {
            s.record(
                WarningCode::SubdirectorySkipped,
                Scope::Source,
                Some(offset),
                None,
            );
        }
        // A repeat of one held still counts.
        s.record(
            WarningCode::SubdirectorySkipped,
            Scope::Source,
            Some(0),
            None,
        );
        let w = s.take();
        assert_eq!((w.warnings.len(), w.dropped), (2, 3));
        assert_eq!(w.warnings[0].count, 2);
    }
}
