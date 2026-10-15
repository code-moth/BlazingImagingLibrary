// SPDX-License-Identifier: LGPL-2.1-or-later
// See LINKING-EXCEPTION.md in the repository for the static-linking exception.

//! Which tiles a JPEG 2000 codestream holds, read from its own markers (ISO/IEC 15444-1,
//! annex A), so that a decoder's success is not taken on trust.
//!
//! A codestream cut short, or missing a tile-part, is corrupt data, an error (specification,
//! section 3.3). OpenJPEG does not always say so: Ubuntu's 2.5.0 fails a codestream cut after
//! its first tile, while the release Homebrew installs on macOS in CI returned success for it.
//! This walk, the same whatever library decodes, finds every tile-part from its SOT marker, and
//! `check` requires the codestream to end with EOC and each tile a request touches to be present
//! and whole, as far as its parts state how many there are.
//!
//! It reads only the main header and twelve bytes at each tile-part, never the coded data.

use crate::io::ReadAt;

/// The tiling a codestream's SIZ marker states, on the reference grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tiling {
    pub x1: u32,
    pub y1: u32,
    pub x0: u32,
    pub y0: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub tile_x0: u32,
    pub tile_y0: u32,
}

impl Tiling {
    /// Tiles across and down.
    pub fn counts(&self) -> (u32, u32) {
        (
            (self.x1 - self.tile_x0).div_ceil(self.tile_width),
            (self.y1 - self.tile_y0).div_ceil(self.tile_height),
        )
    }
}

/// One tile's parts, as the codestream holds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Parts {
    /// TPsot of each part found, in the order found.
    found: Vec<u8>,
    /// TNsot, where a part states it; 0 means not stated.
    declared: u8,
}

/// The tile-parts of a codestream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tiles {
    pub tiling: Tiling,
    tiles: Vec<Parts>,
    /// Whether the codestream ends with EOC.
    ended: bool,
}

/// Reads `n` bytes at `at`, which must lie before `end`.
fn bytes(source: &dyn ReadAt, at: u64, n: usize, end: u64) -> Result<Vec<u8>, String> {
    if at.checked_add(n as u64).is_none_or(|e| e > end) {
        return Err("the codestream is cut short".into());
    }
    let mut buf = vec![0; n];
    source
        .read_exact_at(&mut buf, at)
        .map_err(|e| format!("reading the codestream: {e}"))?;
    Ok(buf)
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Where the codestream of a JP2 file lies: the contents of its top-level `jp2c` box.
pub fn jp2_codestream(source: &dyn ReadAt, offset: u64, length: u64) -> Result<(u64, u64), String> {
    let end = offset
        .checked_add(length)
        .ok_or("the JP2 file's length is too large")?;
    let mut at = offset;
    while at < end {
        let head = bytes(source, at, 8, end)?;
        let (mut size, kind) = (u64::from(be32(&head)), &head[4..8]);
        let mut header = 8;
        if size == 1 {
            size = u64::from_be_bytes(bytes(source, at + 8, 8, end)?.try_into().unwrap());
            header = 16;
        } else if size == 0 {
            size = end - at;
        }
        if size < header || at.checked_add(size).is_none_or(|e| e > end) {
            return Err(format!("a JP2 box of {size} bytes does not fit the file"));
        }
        if kind == b"jp2c" {
            return Ok((at + header, size - header));
        }
        at += size;
    }
    Err("the JP2 file holds no codestream".into())
}

/// Finds every tile-part of the codestream at `offset`, `length` bytes long.
pub fn read(source: &dyn ReadAt, offset: u64, length: u64) -> Result<Tiles, String> {
    let end = offset
        .checked_add(length)
        .ok_or("the codestream's length is too large")?;
    let soc = bytes(source, offset, 4, end)?;
    if soc[..4] != [0xFF, 0x4F, 0xFF, 0x51] {
        return Err("the codestream does not begin with SOC and SIZ".into());
    }
    // SIZ: Lsiz, Rsiz, then eight 32-bit sizes and offsets.
    let siz = bytes(source, offset + 4, 36, end)?;
    let lsiz = u64::from(be16(&siz));
    // 38 bytes, then 3 for each component, of which there is at least one.
    if lsiz < 41 {
        return Err(format!("an SIZ marker of length {lsiz}, too short"));
    }
    let v = |i: usize| be32(&siz[4 + 4 * i..]);
    let tiling = Tiling {
        x1: v(0),
        y1: v(1),
        x0: v(2),
        y0: v(3),
        tile_width: v(4),
        tile_height: v(5),
        tile_x0: v(6),
        tile_y0: v(7),
    };
    let t = tiling;
    if t.tile_width == 0
        || t.tile_height == 0
        || t.x0 >= t.x1
        || t.y0 >= t.y1
        || t.tile_x0 > t.x0
        || t.tile_y0 > t.y0
        || u64::from(t.tile_x0) + u64::from(t.tile_width) <= u64::from(t.x0)
        || u64::from(t.tile_y0) + u64::from(t.tile_height) <= u64::from(t.y0)
    {
        return Err("SIZ states a tiling that does not cover the Image".into());
    }
    let (across, down) = t.counts();
    let count = u64::from(across) * u64::from(down);
    if count > 65_535 {
        return Err(format!(
            "SIZ states {count} tiles, more than a codestream can number"
        ));
    }
    // The other marker segments of the main header, to the first SOT.
    let mut at = (offset + 4)
        .checked_add(lsiz)
        .ok_or("the main header runs past what can be counted")?;
    loop {
        let m = bytes(source, at, 4, end)?;
        if m[0] != 0xFF {
            return Err("the main header holds a byte that is not a marker".into());
        }
        if m[1] == 0x90 {
            break;
        }
        let len = u64::from(be16(&m[2..]));
        if len < 2 {
            return Err(
                "a marker segment of the main header is shorter than its length field".into(),
            );
        }
        at = at
            .checked_add(2 + len)
            .ok_or("the main header runs past what can be counted")?;
    }
    let mut tiles = vec![Parts::default(); count as usize];
    let mut ended = false;
    while at < end {
        let marker = bytes(source, at, 2, end)?;
        match (marker[0], marker[1]) {
            (0xFF, 0xD9) => {
                ended = true;
                break;
            }
            (0xFF, 0x90) => {}
            _ => return Err("a tile-part does not end where the next begins".into()),
        }
        let sot = bytes(source, at, 12, end)?;
        let (lsot, isot, psot) = (be16(&sot[2..]), be16(&sot[4..]), u64::from(be32(&sot[6..])));
        let (tpsot, tnsot) = (sot[10], sot[11]);
        if lsot != 10 {
            return Err(format!("an SOT marker of length {lsot}, not 10"));
        }
        let tile = tiles
            .get_mut(usize::from(isot))
            .ok_or_else(|| format!("a tile-part of tile {isot}, of {count}"))?;
        // Psot counts from the SOT marker to the end of the tile-part; 0 means to the end of the
        // codestream, before EOC.
        let next = match psot {
            0 => end,
            p if p < 14 => return Err(format!("a tile-part of {p} bytes, too short to hold data")),
            p => at
                .checked_add(p)
                .filter(|&n| n <= end)
                .ok_or("a tile-part runs past the end of the codestream")?,
        };
        if tnsot != 0 {
            if tile.declared != 0 && tile.declared != tnsot {
                return Err(format!(
                    "tile {isot}'s parts state {} and {tnsot} parts",
                    tile.declared
                ));
            }
            tile.declared = tnsot;
        }
        tile.found.push(tpsot);
        if psot == 0 {
            // To the end, so the bytes must end with EOC.
            ended = end - at >= 14 && bytes(source, end - 2, 2, end)? == [0xFF, 0xD9];
            break;
        }
        at = next;
    }
    Ok(Tiles {
        tiling,
        tiles,
        ended,
    })
}

impl Tiles {
    /// Whether the tile at `index` is whole: its parts numbered from 0 with none missing, as many
    /// as any part declares.
    fn whole(&self, index: usize) -> Result<(), String> {
        let parts = &self.tiles[index];
        if parts.found.is_empty() {
            return Err(format!("tile {index} is missing"));
        }
        let mut numbers = parts.found.clone();
        numbers.sort_unstable();
        let expected = if parts.declared != 0 {
            usize::from(parts.declared)
        } else {
            numbers.len()
        };
        if numbers.len() != expected
            || numbers
                .iter()
                .enumerate()
                .any(|(i, &n)| usize::from(n) != i)
        {
            return Err(format!(
                "tile {index} has parts {:?}, of {expected}",
                parts.found
            ));
        }
        Ok(())
    }

    /// Checks that the codestream ends with EOC, and that every tile overlapping `area`, or every
    /// tile where `area` is `None`, is whole. `area` is `(x0, y0, x1, y1)`, end exclusive, in
    /// pixels from the Image's top-left corner.
    pub fn check(&self, area: Option<(u64, u64, u64, u64)>) -> Result<(), String> {
        if !self.ended {
            return Err("the codestream does not end with EOC: it was cut short".into());
        }
        let t = self.tiling;
        let (across, down) = t.counts();
        let (ax0, ay0, ax1, ay1) =
            area.unwrap_or((0, 0, u64::from(t.x1 - t.x0), u64::from(t.y1 - t.y0)));
        // From the Image's corner to the reference grid, and to tile numbers.
        // Saturating, so an area past the Image, which `decode` refuses first, cannot overflow.
        let column = |x: u64| {
            let tile = (x.saturating_add(u64::from(t.x0)) - u64::from(t.tile_x0))
                / u64::from(t.tile_width);
            tile.min(u64::from(across) - 1)
        };
        let row = |y: u64| {
            let tile = (y.saturating_add(u64::from(t.y0)) - u64::from(t.tile_y0))
                / u64::from(t.tile_height);
            tile.min(u64::from(down) - 1)
        };
        if ax0 >= ax1 || ay0 >= ay1 {
            return Ok(());
        }
        for q in row(ay0)..=row(ay1 - 1) {
            for p in column(ax0)..=column(ax1 - 1) {
                self.whole((q * u64::from(across) + p) as usize)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A codestream of `across` × `down` tiles of 4 × 4 pixels, each tile in `parts` tile-parts
    /// of a little filler data, as markers only: no decoder would accept the data, but the
    /// walk does not read it.
    fn codestream(across: u32, down: u32, parts: u8, skip: &[(u16, u8)]) -> Vec<u8> {
        let mut c = vec![0xFF, 0x4F, 0xFF, 0x51];
        c.extend(41u16.to_be_bytes());
        c.extend(0u16.to_be_bytes());
        for v in [across * 4, down * 4, 0, 0, 4, 4, 0, 0] {
            c.extend(v.to_be_bytes());
        }
        c.extend(1u16.to_be_bytes());
        c.extend([8, 1, 1]);
        // A COD segment of 12 bytes, which the walk steps over.
        c.extend([0xFF, 0x52, 0, 12, 0, 0, 0, 1, 0, 0, 4, 4, 0, 0]);
        for tile in 0..(across * down) as u16 {
            for part in 0..parts {
                if skip.contains(&(tile, part)) {
                    continue;
                }
                c.extend([0xFF, 0x90, 0, 10]);
                c.extend(tile.to_be_bytes());
                c.extend(18u32.to_be_bytes());
                c.extend([part, parts, 0xFF, 0x93, 1, 2, 3, 4]);
            }
        }
        c.extend([0xFF, 0xD9]);
        c
    }

    #[test]
    fn whole_codestreams_pass() {
        let c = codestream(3, 2, 2, &[]);
        let tiles = read(&c.as_slice(), 0, c.len() as u64).unwrap();
        assert_eq!(tiles.tiling.counts(), (3, 2));
        assert_eq!(tiles.check(None), Ok(()));
    }

    #[test]
    fn every_cut_is_found() {
        let c = codestream(2, 2, 1, &[]);
        for cut in 0..c.len() {
            let r = read(&c.as_slice(), 0, cut as u64).and_then(|t| t.check(None));
            assert!(r.is_err(), "cut at {cut} passed");
        }
    }

    #[test]
    fn missing_parts_fail_only_where_asked_for() {
        // Tile 3, the bottom right, lacks its second part.
        let c = codestream(2, 2, 2, &[(3, 1)]);
        let tiles = read(&c.as_slice(), 0, c.len() as u64).unwrap();
        assert!(tiles.check(None).unwrap_err().contains("tile 3"));
        // The top-left tile alone is whole.
        assert_eq!(tiles.check(Some((0, 0, 4, 4))), Ok(()));
        // An area reaching into tile 3 is not.
        assert!(tiles.check(Some((3, 3, 5, 5))).is_err());
        // An area far past the Image counts as its last column, tile 1 here, with no overflow.
        assert!(tiles.check(Some((u64::MAX - 1, 0, u64::MAX, 1))).is_ok());
        // A tile with no parts at all.
        let c = codestream(2, 1, 1, &[(1, 0)]);
        let tiles = read(&c.as_slice(), 0, c.len() as u64).unwrap();
        assert_eq!(tiles.check(None), Err("tile 1 is missing".into()));
    }

    #[test]
    fn damage_is_an_error_never_a_panic() {
        let good = codestream(3, 3, 2, &[]);
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..20_000 {
            let mut bad = good.clone();
            for _ in 0..1 + state % 4 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let i = (state % bad.len() as u64) as usize;
                bad[i] = (state >> 40) as u8;
            }
            let _ = read(&bad.as_slice(), 0, bad.len() as u64).map(|t| t.check(None));
        }
    }

    /// The codestreams the JPEG 2000 tests decode, written by OpenJPEG itself: whole, each passes;
    /// cut anywhere, each fails, whatever a decoder would do with it.
    #[test]
    fn the_codec_fixtures_pass_whole_and_fail_cut() {
        let fixtures: [(&str, &[u8]); 5] = [
            (
                "gray8.j2k",
                include_bytes!("../../tests/data/jpeg2000/gray8.j2k"),
            ),
            (
                "rgb8.j2k",
                include_bytes!("../../tests/data/jpeg2000/rgb8.j2k"),
            ),
            (
                "gray16.j2k",
                include_bytes!("../../tests/data/jpeg2000/gray16.j2k"),
            ),
            (
                "signed12.j2k",
                include_bytes!("../../tests/data/jpeg2000/signed12.j2k"),
            ),
            (
                "gray8.jp2",
                include_bytes!("../../tests/data/jpeg2000/gray8.jp2"),
            ),
        ];
        for (name, bytes) in fixtures {
            let walk = |len: u64| {
                let (at, len) = if name.ends_with("jp2") {
                    jp2_codestream(&bytes, 0, len)?
                } else {
                    (0, len)
                };
                read(&bytes, at, len)?.check(None)
            };
            assert_eq!(walk(bytes.len() as u64), Ok(()), "{name}");
            for cut in 0..bytes.len() as u64 {
                assert!(walk(cut).is_err(), "{name} cut at {cut} passed");
            }
        }
    }

    #[test]
    fn a_jp2_file_holds_its_codestream_in_jp2c() {
        let c = codestream(1, 1, 1, &[]);
        let mut f = vec![0, 0, 0, 12];
        f.extend(b"jP  \r\n\x87\n");
        f.extend(((c.len() + 8) as u32).to_be_bytes());
        f.extend(b"jp2c");
        f.extend(&c);
        let (at, len) = jp2_codestream(&f.as_slice(), 0, f.len() as u64).unwrap();
        assert_eq!((at, len), (20, c.len() as u64));
        assert!(jp2_codestream(&f[..30].to_vec().as_slice(), 0, 30).is_err());
    }
}
