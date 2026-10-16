// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! The Fragment cache (section 4): decoded blocks kept for each Source, shared by every Reader,
//! least recently used first out, bounded by a count and by bytes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};

use crate::error::Result;

/// What identifies one block (section 4, "One block, one entry"): its Image, level, frame,
/// block row and column, and the components it holds: the first, and one past the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockKey {
    pub image: u32,
    pub level: u32,
    pub frame: u64,
    pub block_row: u32,
    pub block_col: u32,
    pub components: (u32, u32),
}

/// What `describe_cache` reports (section 4). Counts are in blocks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheReport {
    /// Blocks held now, and their bytes.
    pub fragment_count: usize,
    pub bytes: u64,
    /// The limits set by `set_cache_size` and `set_cache_bytes`.
    pub max_fragments: usize,
    pub max_bytes: u64,
    /// Blocks reads found in the cache, and blocks they decoded or copied.
    pub hits: u64,
    pub misses: u64,
    /// Blocks served from read-ahead: always 0 until read-ahead is built.
    pub served_from_read_ahead: u64,
    /// Times a block larger than `max_bytes` was held alone, past the ceiling: the warning
    /// that the ceiling is below the blocks the file holds (section 4, "At least one block").
    pub held_past_ceiling: u64,
}

/// The default count of blocks kept (section 4).
pub const DEFAULT_FRAGMENTS: usize = 16;

/// The default ceiling: a quarter of physical memory, or 1 GiB where it cannot be found
/// (section 4), read once.
pub fn default_bytes() -> u64 {
    static BYTES: OnceLock<u64> = OnceLock::new();
    *BYTES.get_or_init(|| physical_memory().map_or(1 << 30, |m| m / 4))
}

struct Entry {
    data: Arc<Vec<u8>>,
    used: u64,
}

struct State {
    max_fragments: usize,
    max_bytes: u64,
    entries: HashMap<BlockKey, Entry>,
    /// Entries by last use, oldest first.
    by_use: BTreeMap<u64, BlockKey>,
    /// Blocks some read is decoding now.
    pending: HashSet<BlockKey>,
    clock: u64,
    report: CacheReport,
}

/// One Source's cache.
pub struct Cache {
    state: Mutex<State>,
    decoded: Condvar,
}

impl Default for Cache {
    fn default() -> Cache {
        Cache::new(DEFAULT_FRAGMENTS, default_bytes())
    }
}

/// Takes a block out of `pending` however its decode ends, a panic included, so no waiter
/// waits forever.
struct Pending<'a> {
    cache: &'a Cache,
    key: BlockKey,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.cache.lock().pending.remove(&self.key);
        self.cache.decoded.notify_all();
    }
}

impl Cache {
    pub fn new(max_fragments: usize, max_bytes: u64) -> Cache {
        Cache {
            state: Mutex::new(State {
                max_fragments,
                max_bytes,
                entries: HashMap::new(),
                by_use: BTreeMap::new(),
                pending: HashSet::new(),
                clock: 0,
                report: CacheReport::default(),
            }),
            decoded: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while the lock was held leaves the counts as they were; nothing is half done.
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The block, from the cache or decoded by `decode` and then kept. Where another read is
    /// decoding it, this waits for that decode; where that decode failed, this decodes it
    /// itself (section 4, "Decoded once among threads").
    pub fn get_or_decode(
        &self,
        key: BlockKey,
        decode: impl FnOnce() -> Result<Vec<u8>>,
    ) -> Result<Arc<Vec<u8>>> {
        self.read(key, 0, false, |_| decode())
    }

    /// As `get_or_decode`, for a block of `whole_bytes` that the read may need only part of
    /// (`partial`). On a miss, where the cache is off, or where the block would pass the ceiling
    /// and the read needs only part, `decode(false)` decodes only what the read needs and nothing
    /// is kept (M67); otherwise `decode(true)` decodes it whole, and it is kept.
    pub fn read(
        &self,
        key: BlockKey,
        whole_bytes: u64,
        partial: bool,
        decode: impl FnOnce(bool) -> Result<Vec<u8>>,
    ) -> Result<Arc<Vec<u8>>> {
        let mut state = self.lock();
        loop {
            state.clock += 1;
            let now = state.clock;
            if let Some(entry) = state.entries.get_mut(&key) {
                let (data, before) = (entry.data.clone(), entry.used);
                entry.used = now;
                state.by_use.remove(&before);
                state.by_use.insert(now, key);
                state.report.hits += 1;
                return Ok(data);
            }
            if !state.pending.contains(&key) {
                break;
            }
            state = self.decoded.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        state.report.misses += 1;
        if state.max_fragments == 0 || (partial && whole_bytes > state.max_bytes) {
            drop(state);
            return decode(false).map(Arc::new);
        }
        state.pending.insert(key);
        drop(state);
        let guard = Pending { cache: self, key };
        let data = Arc::new(decode(true)?);
        self.keep(key, data.clone());
        drop(guard);
        Ok(data)
    }

    fn keep(&self, key: BlockKey, data: Arc<Vec<u8>>) {
        let mut state = self.lock();
        if state.max_fragments == 0 {
            return;
        }
        let bytes = data.len() as u64;
        if bytes > state.max_bytes {
            // At least one block: it is held alone, and the ceiling's giving way is reported.
            Self::evict_until(&mut state, 0, 0);
            state.report.held_past_ceiling += 1;
        } else {
            let (count, room) = (state.max_fragments - 1, state.max_bytes - bytes);
            Self::evict_until(&mut state, count, room);
        }
        state.clock += 1;
        let now = state.clock;
        state.entries.insert(key, Entry { data, used: now });
        state.by_use.insert(now, key);
        state.report.bytes += bytes;
        state.report.fragment_count = state.entries.len();
    }

    /// Drops the least recently used blocks until at most `count` are held, in at most `bytes`.
    fn evict_until(state: &mut State, count: usize, bytes: u64) {
        while state.entries.len() > count || state.report.bytes > bytes {
            let Some((_, key)) = state.by_use.pop_first() else {
                break;
            };
            if let Some(e) = state.entries.remove(&key) {
                state.report.bytes -= e.data.len() as u64;
            }
        }
        state.report.fragment_count = state.entries.len();
    }

    /// Sets how many blocks are kept; 0 turns the cache off and empties it.
    pub fn set_size(&self, max_fragments: usize) {
        let mut state = self.lock();
        state.max_fragments = max_fragments;
        let bytes = state.max_bytes;
        Self::evict_until(&mut state, max_fragments, bytes);
    }

    /// Sets the most bytes kept. The most recent block stays even past it, until another
    /// displaces it, and that counts as the ceiling giving way (section 4, "At least one
    /// block").
    pub fn set_bytes(&self, max_bytes: u64) {
        let mut state = self.lock();
        state.max_bytes = max_bytes;
        let count = state.max_fragments;
        while state.entries.len() > count
            || (state.entries.len() > 1 && state.report.bytes > max_bytes)
        {
            let Some((_, key)) = state.by_use.pop_first() else {
                break;
            };
            if let Some(e) = state.entries.remove(&key) {
                state.report.bytes -= e.data.len() as u64;
            }
        }
        state.report.fragment_count = state.entries.len();
        if state.report.bytes > max_bytes {
            state.report.held_past_ceiling += 1;
        }
    }

    pub fn describe(&self) -> CacheReport {
        let state = self.lock();
        CacheReport {
            max_fragments: state.max_fragments,
            max_bytes: state.max_bytes,
            ..state.report
        }
    }

    /// Frees every block; the counts are kept.
    pub fn clear(&self) {
        let mut state = self.lock();
        state.entries.clear();
        state.by_use.clear();
        state.report.bytes = 0;
        state.report.fragment_count = 0;
    }
}

/// The machine's physical memory, in bytes, found through each system's own call (section 4).
fn physical_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        extern "C" {
            fn sysconf(name: std::ffi::c_int) -> std::ffi::c_long;
        }
        // _SC_PAGESIZE and _SC_PHYS_PAGES, as Linux numbers them.
        let (page, pages) = unsafe { (sysconf(30), sysconf(85)) };
        (page > 0 && pages > 0).then(|| (page as u64).saturating_mul(pages as u64))
    }
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn sysctlbyname(
                name: *const std::ffi::c_char,
                old: *mut std::ffi::c_void,
                old_len: *mut usize,
                new: *mut std::ffi::c_void,
                new_len: usize,
            ) -> i32;
        }
        let mut bytes = 0u64;
        let mut len = std::mem::size_of::<u64>();
        let ok = unsafe {
            sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut bytes as *mut u64).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        (ok && bytes > 0).then_some(bytes)
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        struct MemoryStatusEx {
            length: u32,
            memory_load: u32,
            total_phys: u64,
            rest: [u64; 6],
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
        }
        let mut status = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            rest: [0; 6],
        };
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) } != 0;
        (ok && status.total_phys > 0).then_some(status.total_phys)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    fn key(n: u32) -> BlockKey {
        BlockKey {
            image: 0,
            level: 0,
            frame: 0,
            block_row: 0,
            block_col: n,
            components: (0, 1),
        }
    }

    fn get(c: &Cache, n: u32, len: usize) -> Arc<Vec<u8>> {
        c.get_or_decode(key(n), || Ok(vec![n as u8; len])).unwrap()
    }

    #[test]
    fn least_recently_used_goes_first() {
        let c = Cache::new(2, 1000);
        get(&c, 1, 10);
        get(&c, 2, 10);
        get(&c, 1, 10); // 1 is now the newer
        get(&c, 3, 10); // 2 goes
        let r = c.describe();
        assert_eq!((r.fragment_count, r.bytes, r.hits, r.misses), (2, 20, 1, 3));
        get(&c, 1, 10);
        assert_eq!(c.describe().hits, 2);
        get(&c, 2, 10);
        assert_eq!(c.describe().misses, 4);
    }

    #[test]
    fn the_byte_ceiling_bounds_what_is_held() {
        let c = Cache::new(16, 25);
        get(&c, 1, 10);
        get(&c, 2, 10);
        get(&c, 3, 10); // 30 bytes would pass 25: 1 goes
        let r = c.describe();
        assert_eq!((r.fragment_count, r.bytes, r.held_past_ceiling), (2, 20, 0));
    }

    #[test]
    fn a_block_past_the_ceiling_is_held_alone_and_reported() {
        let c = Cache::new(16, 25);
        get(&c, 1, 10);
        get(&c, 2, 40);
        let r = c.describe();
        assert_eq!((r.fragment_count, r.bytes, r.held_past_ceiling), (1, 40, 1));
        // Read again, it is not decoded again.
        get(&c, 2, 40);
        assert_eq!(c.describe().hits, 1);
        // The next block that fits displaces it.
        get(&c, 3, 10);
        assert_eq!((c.describe().fragment_count, c.describe().bytes), (1, 10));
    }

    #[test]
    fn size_zero_turns_it_off() {
        let c = Cache::new(4, 1000);
        get(&c, 1, 10);
        c.set_size(0);
        assert_eq!(c.describe().fragment_count, 0);
        get(&c, 1, 10);
        get(&c, 1, 10);
        let r = c.describe();
        assert_eq!((r.fragment_count, r.hits, r.misses), (0, 0, 3));
    }

    #[test]
    fn a_failed_decode_is_not_kept() {
        let c = Cache::new(4, 1000);
        let e = c.get_or_decode(key(1), || Err(Error::Corrupt("bad".into())));
        assert!(e.is_err());
        assert_eq!(c.describe().fragment_count, 0);
        get(&c, 1, 10);
        assert_eq!(c.describe().fragment_count, 1);
    }

    #[test]
    fn a_block_needed_at_once_is_decoded_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let c = Cache::new(4, 1000);
        let decodes = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    c.get_or_decode(key(7), || {
                        decodes.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        Ok(vec![7; 10])
                    })
                    .unwrap()
                });
            }
        });
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
        assert_eq!(c.describe().hits, 7);
    }

    #[test]
    fn a_waiter_on_a_failed_decode_decodes_itself() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let c = Cache::new(4, 1000);
        let tries = AtomicUsize::new(0);
        let results: Vec<bool> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..4)
                .map(|_| {
                    s.spawn(|| {
                        c.get_or_decode(key(3), || {
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            // The first decode fails; any later one succeeds.
                            if tries.fetch_add(1, Ordering::SeqCst) == 0 {
                                Err(Error::Corrupt("first".into()))
                            } else {
                                Ok(vec![3; 10])
                            }
                        })
                        .is_ok()
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        // One read failed, by its own decode; every waiter then got the block.
        assert_eq!(results.iter().filter(|&&ok| !ok).count(), 1);
        assert_eq!(tries.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn physical_memory_is_found_here() {
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        assert!(physical_memory().is_some_and(|m| m >= 1 << 26));
        assert!(default_bytes() > 0);
    }
}
