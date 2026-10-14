// SPDX-License-Identifier: LGPL-2.1-or-later
//! Experiment 3 of the implementability assessment: containing C libraries' errors.
//!
//! libjpeg-turbo and OpenJPEG decode corrupt files behind BIL's C layer (`c/shim.c`), and so, with
//! `--gdal`, does GDAL (`c/gdal.c`), reading NITF and TIFF. The question is whether a corrupt file
//! can crash the process, end it, print, hang it, leak, or change state the whole process shares,
//! GDAL's own included (section 10.3 of the specification; H2 of the assessment).
//!
//! The program is its own supervisor. It starts copies of itself as workers, each decoding a
//! range of corrupt files made by a seeded mutation of valid ones, and watches each worker's
//! exit status, its standard error, which must stay empty, and its time. A worker prints the
//! index of each file before decoding it, so a crash or a hang names the file, which can be made
//! again from its index alone.
//!
//!     cargo run --release -- --check                         # as CI runs it
//!     cargo run --release                                    # more files
//!     cargo run --release --features gdal -- --gdal --check  # GDAL's part, as CI runs it

use std::ffi::{c_char, c_int, CStr};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- the C layer

#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct Info {
    width: u32,
    height: u32,
    components: u32,
    precision: u32,
    warnings: u32,
}

#[repr(C)]
struct Limits {
    max_image_pixels: u64,
    max_decoded_bytes: u64,
}

const OK: c_int = 0;
const ERROR: c_int = 1;
const REFUSED: c_int = 2;

extern "C" {
    fn bil_free(p: *mut std::ffi::c_void);
    fn bil_jpeg_decode(
        data: *const u8,
        len: usize,
        limits: *const Limits,
        out: *mut *mut u8,
        out_len: *mut usize,
        info: *mut Info,
        message: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn bil_jpeg_encode(
        pixels: *const u8,
        width: u32,
        height: u32,
        components: u32,
        quality: c_int,
        progressive: c_int,
        out: *mut *mut u8,
        out_len: *mut usize,
        message: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn bil_jpeg_decode_unprotected(data: *const u8, len: usize) -> c_int;
    fn bil_j2k_decode(
        data: *const u8,
        len: usize,
        format: c_int,
        limits: *const Limits,
        out: *mut *mut i32,
        out_len: *mut usize,
        info: *mut Info,
        message: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn bil_j2k_encode(
        pixels: *const u8,
        width: u32,
        height: u32,
        components: u32,
        format: c_int,
        tile: u32,
        out: *mut *mut u8,
        out_len: *mut usize,
        message: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn bil_j2k_decode_unprotected(data: *const u8, len: usize, format: c_int) -> c_int;
    fn bil_state_describe(buf: *mut c_char, cap: usize) -> usize;
    fn bil_heap_in_use() -> u64;
    fn bil_fp_flags() -> u32;
    fn bil_fp_clear();
    fn opj_version() -> *const c_char;
    fn bil_state_poke(what: c_int, undo: c_int) -> c_int;
    // GDAL's layer, or with the `gdal` feature off, its stand-in.
    fn bil_gdal_init() -> *const c_char;
    fn bil_gdal_decode(
        data: *const u8,
        len: usize,
        format: c_int,
        limits: *const Limits,
        out: *mut *mut u8,
        out_len: *mut usize,
        info: *mut Info,
        message: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn bil_gdal_encode(
        pixels: *const u8,
        width: u32,
        height: u32,
        components: u32,
        format: c_int,
        variant: c_int,
        out: *mut *mut u8,
        out_len: *mut usize,
        message: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn bil_gdal_decode_unprotected(data: *const u8, len: usize, format: c_int) -> c_int;
    fn bil_gdal_state_describe(buf: *mut c_char, cap: usize) -> usize;
    fn bil_gdal_state_poke(what: c_int, undo: c_int) -> c_int;
}

/// GDAL's version, after registering the drivers its layer uses, or `None` where the program was
/// built without the `gdal` feature.
fn gdal_init() -> Option<String> {
    let v = unsafe { bil_gdal_init() };
    (!v.is_null()).then(|| unsafe { CStr::from_ptr(v) }.to_string_lossy().into_owned())
}

/// The limits every decode here is given: section 10.3's limits, at sizes chosen for the
/// experiment, large beside its files.
const LIMITS: Limits = Limits {
    max_image_pixels: 1 << 22,
    max_decoded_bytes: 64 << 20,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    Jpeg,
    J2k,
    Jp2,
    /// NITF, read by GDAL.
    Nitf,
    /// TIFF, read by GDAL's GeoTIFF driver.
    Tiff,
}

impl Codec {
    fn name(self) -> &'static str {
        match self {
            Codec::Jpeg => "jpeg",
            Codec::J2k => "j2k",
            Codec::Jp2 => "jp2",
            Codec::Nitf => "nitf",
            Codec::Tiff => "tiff",
        }
    }
    fn parse(s: &str) -> Codec {
        match s {
            "jpeg" => Codec::Jpeg,
            "j2k" => Codec::J2k,
            "jp2" => Codec::Jp2,
            "nitf" => Codec::Nitf,
            "tiff" => Codec::Tiff,
            _ => panic!("unknown codec {s}"),
        }
    }
    /// The format number the C layer takes: OpenJPEG's codestream or JP2 file, or GDAL's NITF or
    /// TIFF.
    fn format(self) -> c_int {
        match self {
            Codec::Jp2 => 2,
            Codec::Tiff => 1,
            _ => 0,
        }
    }
    fn is_gdal(self) -> bool {
        matches!(self, Codec::Nitf | Codec::Tiff)
    }
}

/// How one decode turned out.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    /// Decoded, with a hash of the samples.
    Ok(Info, u64),
    /// Decoded, but the library recovered from corrupt data on the way.
    Warned(Info, u64),
    /// An error, with its message.
    Error(String),
    /// Refused before decoding, by the limits.
    Refused,
}

impl Outcome {
    /// What BIL would deliver. A warning is an error (section 3.3), so a decode with warnings
    /// delivers no samples, and only its outcome counts. That matters: on a corrupt JPEG tile,
    /// GDAL's GeoTIFF driver returns, with warnings, samples from a buffer it never wrote.
    fn delivered(&self) -> Outcome {
        match self {
            Outcome::Warned(info, _) => Outcome::Warned(*info, 0),
            other => other.clone(),
        }
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

fn decode(codec: Codec, data: &[u8], limits: &Limits) -> (Outcome, Vec<u8>) {
    let (outcome, samples, _) = decode_with(codec, data, limits, true);
    (outcome, samples)
}

/// Decodes `data`, keeping the samples if `keep`. Without `keep`, nothing on this side is
/// allocated during the call, so the change in heap in use from before the call to after the
/// decoded buffer is freed is what the C layer and the library left behind.
fn decode_with(codec: Codec, data: &[u8], limits: &Limits, keep: bool) -> (Outcome, Vec<u8>, i64) {
    let mut message = [0 as c_char; 256];
    let mut info = Info::default();
    let mut samples = Vec::new();
    let mut hash = fnv(&[]);
    let heap0 = unsafe { bil_heap_in_use() };
    let status = unsafe {
        // The decoded samples, as bytes, and how to free them.
        let (s, bytes, len): (c_int, *mut u8, usize) = match codec {
            Codec::Jpeg => {
                let (mut out, mut len) = (std::ptr::null_mut(), 0);
                let s = bil_jpeg_decode(
                    data.as_ptr(),
                    data.len(),
                    limits,
                    &mut out,
                    &mut len,
                    &mut info,
                    message.as_mut_ptr(),
                    message.len(),
                );
                (s, out, len)
            }
            Codec::J2k | Codec::Jp2 => {
                let (mut out, mut len) = (std::ptr::null_mut::<i32>(), 0);
                let s = bil_j2k_decode(
                    data.as_ptr(),
                    data.len(),
                    codec.format(),
                    limits,
                    &mut out,
                    &mut len,
                    &mut info,
                    message.as_mut_ptr(),
                    message.len(),
                );
                // Native 32-bit samples, read as their bytes.
                (s, out.cast(), len * 4)
            }
            Codec::Nitf | Codec::Tiff => {
                let (mut out, mut len) = (std::ptr::null_mut(), 0);
                let s = bil_gdal_decode(
                    data.as_ptr(),
                    data.len(),
                    codec.format(),
                    limits,
                    &mut out,
                    &mut len,
                    &mut info,
                    message.as_mut_ptr(),
                    message.len(),
                );
                (s, out, len)
            }
        };
        if !bytes.is_null() {
            let view = std::slice::from_raw_parts(bytes, len);
            hash = fnv(view);
            if keep {
                samples.extend_from_slice(view);
            }
            bil_free(bytes.cast());
        }
        s
    };
    let left = unsafe { bil_heap_in_use() } as i64 - heap0 as i64;
    let text = unsafe { CStr::from_ptr(message.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let outcome = match status {
        OK if info.warnings == 0 => Outcome::Ok(info, hash),
        OK => Outcome::Warned(info, hash),
        REFUSED => Outcome::Refused,
        ERROR => Outcome::Error(text),
        other => Outcome::Error(format!("unknown status {other}")),
    };
    (outcome, samples, if keep { 0 } else { left })
}

/// The process's state, and GDAL's where it is built.
fn state() -> String {
    // Bytes, passed as C chars, whose signedness differs between processors.
    let mut buf = vec![0u8; 16384];
    let n = unsafe { bil_state_describe(buf.as_mut_ptr().cast::<c_char>(), buf.len()) };
    let mut text = String::from_utf8_lossy(&buf[..n]).into_owned();
    let n = unsafe { bil_gdal_state_describe(buf.as_mut_ptr().cast::<c_char>(), buf.len()) };
    text.push_str(&String::from_utf8_lossy(&buf[..n]));
    text
}

// ---------------------------------------------------------------- seed files

/// A small deterministic generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// An image with smooth parts, edges and noise, so that every coding tool is used.
fn picture(width: u32, height: u32, components: u32, seed: u64) -> Vec<u8> {
    let mut rng = Rng(seed);
    let mut v = Vec::with_capacity((width * height * components) as usize);
    for r in 0..height {
        for c in 0..width {
            for k in 0..components {
                let smooth = r * 120 / height + c * 80 / width + k * 15;
                let edge = if (r / 8 + c / 8) % 2 == 0 { 0 } else { 30 };
                let noise = rng.below(12) as u32;
                v.push((smooth + edge + noise).min(255) as u8);
            }
        }
    }
    v
}

struct Seed {
    codec: Codec,
    data: Vec<u8>,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    components: u32,
    /// Whether it was coded with loss, so that it decodes only close to `pixels`.
    lossy: bool,
}

fn encode(codec: Codec, pixels: &[u8], w: u32, h: u32, comps: u32, variant: u32) -> Vec<u8> {
    let mut message = [0 as c_char; 256];
    let (mut out, mut len) = (std::ptr::null_mut(), 0usize);
    let status = unsafe {
        match codec {
            Codec::Jpeg => bil_jpeg_encode(
                pixels.as_ptr(),
                w,
                h,
                comps,
                if variant == 0 { 90 } else { 75 },
                c_int::from(variant == 1),
                &mut out,
                &mut len,
                message.as_mut_ptr(),
                message.len(),
            ),
            Codec::J2k | Codec::Jp2 => bil_j2k_encode(
                pixels.as_ptr(),
                w,
                h,
                comps,
                codec.format(),
                if variant == 1 { 32 } else { 0 },
                &mut out,
                &mut len,
                message.as_mut_ptr(),
                message.len(),
            ),
            Codec::Nitf | Codec::Tiff => bil_gdal_encode(
                pixels.as_ptr(),
                w,
                h,
                comps,
                codec.format(),
                variant as c_int,
                &mut out,
                &mut len,
                message.as_mut_ptr(),
                message.len(),
            ),
        }
    };
    let text = unsafe { CStr::from_ptr(message.as_ptr()) }.to_string_lossy();
    assert_eq!(status, OK, "encoding a seed failed: {text}");
    let v = unsafe { std::slice::from_raw_parts(out, len) }.to_vec();
    unsafe { bil_free(out.cast()) };
    v
}

/// The valid files the corrupt ones are made from: for each codec, gray and color, small and
/// odd sizes, and for JPEG a progressive file, for JPEG 2000 a tiled one. For NITF and TIFF,
/// each shape is one of the four variants `bil_gdal_encode` lists: uncompressed, blocked or
/// tiled, and compressed losslessly and with JPEG.
fn seeds(codec: Codec) -> Vec<Seed> {
    if codec.is_gdal() {
        gdal_init().expect("NITF and TIFF need the program built with the gdal feature");
    }
    let shapes = [
        (64, 48, 1, 0),
        (96, 80, 3, 1),
        (33, 17, 3, 0),
        (40, 40, 1, 1),
    ];
    shapes
        .iter()
        .enumerate()
        .map(|(i, &(w, h, comps, variant))| {
            let variant = if codec.is_gdal() { i as u32 } else { variant };
            let pixels = picture(w, h, comps, 77 + i as u64);
            let data = encode(codec, &pixels, w, h, comps, variant);
            let lossy = match codec {
                Codec::Jpeg => true,
                Codec::J2k | Codec::Jp2 => false,
                Codec::Nitf => variant % 2 == 1,
                Codec::Tiff => variant == 3,
            };
            Seed {
                codec,
                data,
                pixels,
                width: w,
                height: h,
                components: comps,
                lossy,
            }
        })
        .collect()
}

// ---------------------------------------------------------------- corrupt files

/// The corrupt file of a given index: a seed, changed by one to eight mutations chosen by a
/// generator seeded with the index, so any file can be made again from its index.
///
/// NITF's headers are fixed-length text fields, its lengths, counts and sizes written in digits,
/// so NITF files also have two mutations of their own: a digit changed in the headers, and a
/// field of digits set to all nines or all zeros. Every other format draws as before, so its
/// files are the same as in earlier runs.
fn corrupt(seeds: &[Seed], index: u64) -> Vec<u8> {
    let mut rng = Rng(index.wrapping_mul(0x2545F4914F6CDD1D) ^ 0x5EED);
    let seed = &seeds[rng.below(seeds.len())];
    let kinds = if seed.codec == Codec::Nitf { 12 } else { 10 };
    let mut d = seed.data.clone();
    for _ in 0..1 + rng.below(8) {
        if d.is_empty() {
            d.push(0);
        }
        let len = d.len();
        let at = rng.below(len);
        match rng.below(kinds) {
            // Change a digit in the headers, the first few kilobytes.
            10 => {
                let digits: Vec<usize> = (0..len.min(4096))
                    .filter(|&i| d[i].is_ascii_digit())
                    .collect();
                if let Some(&i) = digits.get(rng.below(digits.len())) {
                    d[i] = b'0' + rng.below(10) as u8;
                }
            }
            // Set a run of digits, a field, to all nines or all zeros.
            11 => {
                let digits: Vec<usize> = (0..len.min(4096))
                    .filter(|&i| d[i].is_ascii_digit())
                    .collect();
                if let Some(&i) = digits.get(rng.below(digits.len())) {
                    let fill = if rng.below(2) == 0 { b'9' } else { b'0' };
                    let mut j = i;
                    while j < len && d[j].is_ascii_digit() {
                        d[j] = fill;
                        j += 1;
                    }
                }
            }
            // Flip a bit.
            0 => d[at] ^= 1 << rng.below(8),
            // Set a byte to anything.
            1 => d[at] = rng.next() as u8,
            // Set a byte to an extreme.
            2 => d[at] = [0x00, 0xFF, 0x7F, 0x80][rng.below(4)],
            // Change the bytes after a marker: lengths, sizes, counts.
            3 | 4 => {
                let markers: Vec<usize> = (0..len - 1).filter(|&i| d[i] == 0xFF).collect();
                if let Some(&m) = markers.get(rng.below(markers.len())) {
                    for k in 0..1 + rng.below(6) {
                        if let Some(b) = d.get_mut(m + 2 + k) {
                            *b = if rng.below(2) == 0 {
                                rng.next() as u8
                            } else {
                                0xFF
                            };
                        }
                    }
                }
            }
            // Cut the file short.
            5 => d.truncate(at.max(1)),
            // Remove a run of bytes.
            6 => {
                let n = rng.below(64).min(len - at);
                d.drain(at..at + n);
            }
            // Repeat a run of bytes.
            7 => {
                let n = rng.below(64).min(len - at);
                let run: Vec<u8> = d[at..at + n].to_vec();
                let to = rng.below(d.len());
                d.splice(to..to, run);
            }
            // Insert random bytes.
            8 => {
                let n = 1 + rng.below(16);
                let run: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                d.splice(at..at, run);
            }
            // Splice in part of another seed.
            _ => {
                let other = &seeds[rng.below(seeds.len())].data;
                let from = rng.below(other.len());
                let n = rng.below(256).min(other.len() - from);
                let end = (at + n).min(d.len());
                d.splice(at..end, other[from..from + n].iter().copied());
            }
        }
    }
    d
}

// ---------------------------------------------------------------- the worker

/// Files decoded before heap is counted, for anything a library sets up once.
const WARMUP: u64 = 50;

/// Decodes the corrupt files `start..start + count` of `codec`, printing each index first, and
/// then a summary. Every tenth file is decoded twice, and both outcomes must agree.
fn worker(codec: Codec, start: u64, count: u64) {
    let seeds = seeds(codec);
    let before = state();
    unsafe { bil_fp_clear() };
    let (mut slowest, mut slowest_at) = (Duration::ZERO, start);
    // A digest of every outcome, so that two workers over the same files can be compared.
    let mut digest = fnv(&[]);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let (mut ok, mut warned, mut errors, mut refused, mut differ) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut leaked = 0i64;
    let mut shown = 0;
    let mut messages = std::collections::BTreeMap::<String, u64>::new();
    // Every outcome, for finding the file on which two runs differ.
    let trace = std::env::var_os("EXP3_TRACE").is_some();
    for i in start..start + count {
        writeln!(out, "i {i}").unwrap();
        out.flush().unwrap();
        let data = corrupt(&seeds, i);
        let begun = Instant::now();
        let (outcome, _, left) = decode_with(codec, &data, &LIMITS, false);
        let took = begun.elapsed();
        if took > slowest {
            (slowest, slowest_at) = (took, i);
        }
        digest = fnv(format!("{digest} {:?}", outcome.delivered()).as_bytes());
        if trace {
            writeln!(out, "o {i} {:?}", outcome.delivered()).unwrap();
        }
        if i >= start + WARMUP && left != 0 {
            leaked += left;
            if shown < 20 {
                writeln!(out, "left {i} {left}").unwrap();
                shown += 1;
            }
        }
        if i % 10 == 0
            && decode_with(codec, &data, &LIMITS, false).0.delivered() != outcome.delivered()
        {
            differ += 1;
            writeln!(out, "nondeterministic {i}").unwrap();
        }
        match &outcome {
            Outcome::Ok(..) => ok += 1,
            Outcome::Warned(..) => warned += 1,
            Outcome::Error(m) => {
                errors += 1;
                // Messages carry numbers; keep their words, to count the kinds.
                let kind: String = m.chars().filter(|c| !c.is_ascii_digit()).take(60).collect();
                *messages.entry(kind).or_default() += 1;
            }
            Outcome::Refused => refused += 1,
        }
    }
    let flags = unsafe { bil_fp_flags() };
    let after = state();
    writeln!(out, "fp flags={flags:x}").unwrap();
    writeln!(out, "slowest={} {slowest_at}", slowest.as_secs_f64()).unwrap();
    writeln!(out, "digest={digest:x}").unwrap();
    writeln!(out, "outcomes ok={ok} warned={warned} error={errors} refused={refused} nondeterministic={differ}").unwrap();
    writeln!(out, "heap left={leaked}").unwrap();
    if before == after {
        writeln!(out, "state unchanged").unwrap();
    } else {
        for (a, b) in before.lines().zip(after.lines()).filter(|(a, b)| a != b) {
            writeln!(out, "state changed: {a} -> {b}").unwrap();
        }
    }
    writeln!(out, "kinds of error={}", messages.len()).unwrap();
    let scans: u64 = messages
        .iter()
        .filter(|(m, _)| m.contains("progressive scans"))
        .map(|(_, n)| n)
        .sum();
    writeln!(out, "scan limit={scans}").unwrap();
    writeln!(out, "done").unwrap();
}

// ---------------------------------------------------------------- the supervisor

struct Run {
    status: Option<i32>,
    signal: Option<i32>,
    stdout: Vec<String>,
    stderr: String,
    timed_out: bool,
}

/// Runs this program with `args`, watching it for at most `limit`.
fn child(args: &[String], limit: Duration) -> Run {
    child_with(args, limit, 0)
}

/// As `child`, with glibc filling memory it hands out, and memory freed, with `perturb` where
/// that is not 0, so that a read of memory never written sees other bytes than in another run.
fn child_with(args: &[String], limit: Duration, perturb: u8) -> Run {
    let exe = std::env::current_exe().unwrap();
    let begun = Instant::now();
    let mut proc = Command::new(exe)
        .args(args)
        // glibc keeps freed blocks in per-thread caches and fast bins that its count of heap in
        // use still includes; without them, the count falls back exactly when memory is freed.
        .env(
            "GLIBC_TUNABLES",
            format!(
                "glibc.malloc.tcache_count=0:glibc.malloc.mxfast=0:glibc.malloc.perturb={perturb}"
            ),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cannot start a worker");
    let stdout = proc.stdout.take().unwrap();
    let mut stderr = proc.stderr.take().unwrap();
    let lines = std::thread::spawn(move || {
        BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .collect::<Vec<_>>()
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stderr, &mut s);
        s
    });
    let mut timed_out = false;
    let status = loop {
        if let Some(s) = proc.try_wait().unwrap() {
            break s;
        }
        if begun.elapsed() > limit {
            let _ = proc.kill();
            timed_out = true;
            break proc.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    Run {
        status: status.code(),
        signal,
        stdout: lines.join().unwrap(),
        stderr: err.join().unwrap(),
        timed_out,
    }
}

fn field(lines: &[String], key: &str) -> Option<String> {
    lines
        .iter()
        .find_map(|l| l.strip_prefix(key).map(|v| v.trim().to_string()))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--worker") => {
            worker(
                Codec::parse(&args[1]),
                args[2].parse().unwrap(),
                args[3].parse().unwrap(),
            );
            return;
        }
        Some("--control") => {
            control(Codec::parse(&args[1]));
            return;
        }
        Some("--dump") => {
            // Writes one corrupt file, made again from its index, for a closer look.
            let codec = Codec::parse(&args[1]);
            let index: u64 = args[2].parse().unwrap();
            std::fs::write(&args[3], corrupt(&seeds(codec), index)).unwrap();
            return;
        }
        Some("--limits") => {
            if args.get(1).is_some_and(|a| a == "gdal") {
                gdal_limits();
            } else {
                limits();
            }
            return;
        }
        _ => {}
    }
    let check = args.iter().any(|a| a == "--check");
    let gdal = args.iter().any(|a| a == "--gdal");
    if gdal {
        match gdal_init() {
            Some(version) => println!(
                "GDAL {version}, with its NITF, GeoTIFF, JPEG and MEM drivers registered one by \
                 one.\n"
            ),
            None => {
                println!(
                    "GDAL's part needs the program built with the gdal feature, where libgdal-dev \
                     is installed: cargo run --release --features gdal -- --gdal"
                );
                std::process::exit(2);
            }
        }
    } else {
        // The OpenJPEG header in vendor/ describes 2.5.0, and nothing else checks that the
        // library linked is the same.
        let opj = unsafe { CStr::from_ptr(opj_version()) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            opj, "2.5.0",
            "the OpenJPEG linked is not the version of the header in vendor/"
        );
    }
    let scale = if check { 1 } else { 10 };
    let per_codec: Vec<(Codec, u64)> = if gdal {
        vec![
            (Codec::Nitf, 40_000 * scale),
            (Codec::Tiff, 400_000 * scale),
        ]
    } else {
        vec![
            (Codec::Jpeg, 400_000 * scale),
            (Codec::J2k, 80_000 * scale),
            (Codec::Jp2, 40_000 * scale),
        ]
    };
    let codecs: Vec<Codec> = per_codec.iter().map(|&(c, _)| c).collect();
    let mut all_ok = true;
    let mut report = |name: &str, ok: bool, detail: String| {
        all_ok &= ok;
        println!("{name}: {detail}. Passed: **{ok}**\n");
        if !ok && std::env::var_os("GITHUB_ACTIONS").is_some() {
            println!("::error::exp3 {name}: {}", detail.replace('\n', " / "));
        }
    };

    // 1. The seeds decode: those coded with loss closely, the others sample for sample.
    let mut exact = true;
    let mut worst_psnr = f64::INFINITY;
    let mut lossy_count = 0;
    for &codec in &codecs {
        for s in seeds(codec) {
            let (outcome, samples) = decode(s.codec, &s.data, &LIMITS);
            let info = match outcome {
                Outcome::Ok(info, _) => info,
                other => panic!("a valid {} seed did not decode: {other:?}", codec.name()),
            };
            assert_eq!(
                (info.width, info.height, info.components),
                (s.width, s.height, s.components)
            );
            if s.lossy {
                lossy_count += 1;
                let mse = samples
                    .iter()
                    .zip(&s.pixels)
                    .map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2))
                    .sum::<f64>()
                    / s.pixels.len() as f64;
                let psnr = 10.0 * (255.0f64 * 255.0 / mse.max(1e-12)).log10();
                worst_psnr = worst_psnr.min(psnr);
                if std::env::var_os("EXP3_DEBUG").is_some() {
                    eprintln!(
                        "seed {}x{}x{} psnr {psnr:.1}",
                        s.width, s.height, s.components
                    );
                }
            } else if codec.is_gdal() {
                // 8-bit samples, pixel-interleaved, as they were given.
                exact &= samples == s.pixels;
            } else {
                // Planes of 32-bit samples, one component after another.
                let n = (s.width * s.height) as usize;
                let comps = s.components as usize;
                let planes: Vec<i32> = samples
                    .chunks_exact(4)
                    .map(|b| i32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                exact &= (0..n * comps)
                    .all(|p| planes[(p % comps) * n + p / comps] == i32::from(s.pixels[p]));
            }
        }
    }
    let kinds = if gdal {
        "NITF and TIFF files, uncompressed, blocked or tiled, lossless and JPEG,"
    } else {
        "JPEG, JPEG 2000 codestreams and JP2 files"
    };
    report(
        "Valid files",
        exact && worst_psnr > 30.0,
        format!("{} seeds of {kinds} decode: the {lossy_count} coded with loss at {worst_psnr:.1} dB at worst, against at least 30; the others losslessly, sample for sample: {exact}", codecs.len() * 4),
    );

    // 2. The controls: libjpeg's own error handling ends the process and prints; that is what
    // the layer prevents. OpenJPEG with no handlers set is watched for printing. GDAL's default
    // handler prints.
    if gdal {
        let nitf = child(
            &["--control".into(), "nitf".into()],
            Duration::from_secs(30),
        );
        let tiff = child(
            &["--control".into(), "tiff".into()],
            Duration::from_secs(30),
        );
        let returned = |r: &Run| r.stdout.iter().any(|l| l == "returned");
        let shown = |r: &Run| {
            let e = r.stderr.trim();
            if e.is_empty() {
                "nothing".to_string()
            } else {
                format!("{e:?}")
            }
        };
        report(
            "Without the layer",
            returned(&nitf) && returned(&tiff) && !nitf.stderr.trim().is_empty() && !tiff.stderr.trim().is_empty(),
            format!(
                "GDAL's own handler, given a NITF file and a TIFF file that are neither, printed {} and {}, and returned: {}",
                shown(&nitf),
                shown(&tiff),
                returned(&nitf) && returned(&tiff),
            ),
        );
    } else {
        let jpeg_control = child(
            &["--control".into(), "jpeg".into()],
            Duration::from_secs(30),
        );
        let ended =
            jpeg_control.status == Some(1) && !jpeg_control.stdout.iter().any(|l| l == "returned");
        let printed = jpeg_control.stderr.trim().to_string();
        let j2k_control = child(&["--control".into(), "j2k".into()], Duration::from_secs(30));
        let j2k_printed = j2k_control.stderr.trim().to_string();
        let j2k_returned = if j2k_control.stdout.iter().any(|l| l == "returned") {
            "returned"
        } else {
            "did not return"
        };
        let j2k_shown = if j2k_printed.is_empty() {
            "nothing".to_string()
        } else {
            format!("{j2k_printed:?}")
        };
        report(
            "Without the layer",
            ended && !printed.is_empty(),
            format!(
                "libjpeg's own handler ended the process with status {:?} and printed {printed:?}; \
                 OpenJPEG with no handlers set {j2k_returned} and printed {j2k_shown}",
                jpeg_control.status,
            ),
        );
    }

    // 3. Resource limits: a header that claims an enormous Image is refused before decoding,
    // with no memory taken for it; and a progressive JPEG with more scans than the layer
    // allows is stopped. In a child, where heap is counted exactly.
    let mut limit_args = vec!["--limits".to_string()];
    if gdal {
        limit_args.push("gdal".into());
    }
    let run = child(&limit_args, Duration::from_secs(60));
    let refused: usize = field(&run.stdout, "refused=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let taken: i64 = field(&run.stdout, "taken=")
        .and_then(|v| v.parse().ok())
        .unwrap_or(i64::MAX);
    let scans = field(&run.stdout, "scans=").unwrap_or_default();
    if gdal {
        report(
            "Limits",
            refused == 3 && taken == 0 && run.status == Some(0),
            format!("headers claiming 65,000 × 65,000 pixels for NITF and TIFF, and a TIFF block of 7.85 GB in an Image of 24 MB, refused before decoding: {refused} of 3; heap left: {taken} bytes{}", if run.status == Some(0) { String::new() } else { format!(" (the child ended with {:?}, signal {:?}: {:?})", run.status, run.signal, run.stderr.chars().take(300).collect::<String>()) }),
        );
    } else {
        report(
            "Limits",
            refused == 3 && taken == 0 && scans == "stopped" && run.status == Some(0),
            format!("headers claiming 65,000 × 65,000 pixels for JPEG and 65,536 × 65,536 for a codestream and a JP2 file, refused before decoding: {refused} of 3; heap left: {taken} bytes; a progressive JPEG with one scan repeated 600 times: {scans}"),
        );
    }

    // 4. The check of the check: each kind of change the comparison of process state is meant
    // to notice, made and undone, is noticed.
    let mut noticed = 0;
    let mut made = 0;
    let pokes: Vec<(bool, c_int)> = (1..=3)
        .map(|w| (false, w))
        .chain((1..=7).filter(|_| gdal).map(|w| (true, w)))
        .collect();
    for (in_gdal, what) in pokes {
        let poke = |undo| unsafe {
            if in_gdal {
                bil_gdal_state_poke(what, undo)
            } else {
                bil_state_poke(what, undo)
            }
        };
        let before = state();
        if poke(0) == 1 {
            made += 1;
            let changed = state() != before;
            poke(1);
            let restored = state() == before;
            if std::env::var_os("EXP3_DEBUG").is_some() {
                eprintln!("poke {in_gdal} {what}: changed {changed}, restored {restored}");
            }
            noticed += usize::from(changed && restored);
        }
    }
    let changes = if gdal {
        "flush-to-zero, a signal handler, the file mode mask; and in GDAL a global and a thread's configuration option, an error handler pushed, a driver registered, a dataset left open, a file left in memory, the last error"
    } else {
        "flush-to-zero, a signal handler, the file mode mask"
    };
    report(
        "The state comparison",
        made > 0 && noticed == made,
        format!("of {made} changes made and undone ({changes}), each noticed, and its undoing seen as the same state: {noticed} of {made}"),
    );

    // 5. Corrupt files, in workers.
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get());
    for (codec, total) in per_codec {
        let parts = threads as u64 * 2;
        let size = total / parts;
        let mut summary = (0u64, 0u64, 0u64, 0u64, 0u64);
        let mut failures = Vec::new();
        let mut leaked = 0i64;
        let mut kinds = 0u64;
        let mut flags = 0u32;
        let mut scan_stops = 0u64;
        let (mut slowest, mut slowest_at) = (0f64, 0u64);
        let begun = Instant::now();
        let jobs: Vec<(u64, u64)> = (0..parts).map(|p| (p * size, size)).collect();
        for batch in jobs.chunks(threads) {
            let runs: Vec<(u64, Run)> = std::thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|&(start, count)| {
                        scope.spawn(move || {
                            let args = vec![
                                "--worker".into(),
                                codec.name().into(),
                                start.to_string(),
                                count.to_string(),
                            ];
                            (start, child(&args, Duration::from_secs(600)))
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            for (start, run) in runs {
                let last = run
                    .stdout
                    .iter()
                    .rev()
                    .find_map(|l| l.strip_prefix("i ").map(str::to_string));
                let done = run.stdout.last().is_some_and(|l| l == "done");
                if !done || run.status != Some(0) || !run.stderr.is_empty() {
                    failures.push(format!(
                        "worker from {start}: status {:?}, signal {:?}, timed out {}, last file {:?}, stderr {:?}",
                        run.status, run.signal, run.timed_out, last, run.stderr.chars().take(200).collect::<String>()
                    ));
                    continue;
                }
                if let Some(o) = field(&run.stdout, "outcomes ") {
                    let get = |k: &str| -> u64 {
                        o.split_whitespace()
                            .find_map(|kv| {
                                kv.strip_prefix(&format!("{k}="))
                                    .and_then(|v| v.parse().ok())
                            })
                            .unwrap_or(0)
                    };
                    summary.0 += get("ok");
                    summary.1 += get("warned");
                    summary.2 += get("error");
                    summary.3 += get("refused");
                    summary.4 += get("nondeterministic");
                }
                leaked += field(&run.stdout, "heap left=")
                    .and_then(|g| g.parse::<i64>().ok())
                    .unwrap_or(0);
                kinds = kinds.max(
                    field(&run.stdout, "kinds of error=")
                        .and_then(|g| g.parse().ok())
                        .unwrap_or(0),
                );
                flags |= field(&run.stdout, "fp flags=")
                    .and_then(|g| u32::from_str_radix(&g, 16).ok())
                    .unwrap_or(0);
                scan_stops += field(&run.stdout, "scan limit=")
                    .and_then(|g| g.parse().ok())
                    .unwrap_or(0);
                if let Some(v) = field(&run.stdout, "slowest=") {
                    let mut it = v.split_whitespace();
                    let secs: f64 = it.next().and_then(|t| t.parse().ok()).unwrap_or(0.0);
                    if secs > slowest {
                        slowest = secs;
                        slowest_at = it.next().and_then(|t| t.parse().ok()).unwrap_or(0);
                    }
                }
                for l in run.stdout.iter().filter(|l| l.starts_with("state changed")) {
                    failures.push(format!("worker from {start}: {l}"));
                }
            }
        }
        // The first files again, in two workers whose memory glibc fills with different bytes:
        // a decode that read memory it had not written would see the difference.
        let pair = (total / 10).min(20_000);
        let args = vec![
            "--worker".into(),
            codec.name().into(),
            "0".into(),
            pair.to_string(),
        ];
        let digests: Vec<Option<String>> = std::thread::scope(|scope| {
            let a = scope.spawn(|| child_with(&args, Duration::from_secs(600), 0x55));
            let b = scope.spawn(|| child_with(&args, Duration::from_secs(600), 0xAA));
            [a.join().unwrap(), b.join().unwrap()]
                .iter()
                .map(|r| field(&r.stdout, "digest="))
                .collect()
        });
        let same = digests[0].is_some() && digests[0] == digests[1];
        let (ok, warned, errors, refused, differ) = summary;
        let decoded = ok + warned + errors + refused;
        let passed = failures.is_empty()
            && differ == 0
            && same
            && decoded == parts * size
            && leaked == 0
            && slowest < 5.0;
        report(
            &format!("Corrupt {} files", codec.name()),
            passed,
            format!(
                "{decoded} files in {parts} workers, {:.1} s: {ok} decoded, {warned} decoded with corrupt data recovered from, {errors} errors ({kinds} kinds at most in one worker), {refused} refused by the limits, {scan_stops} stopped at the scan limit; decoded twice in one worker with a different outcome: {differ}; the first {pair} decoded alike in two workers with memory filled differently: {same}; heap left behind by the library and the layer, after each worker's first {WARMUP} files: {leaked} bytes; slowest file {slowest_at}, {:.3} s, against a limit of 5; floating-point flags raised: {}; no crash, exit, hang or output, and process state unchanged: {}{}",
                begun.elapsed().as_secs_f64(),
                slowest,
                flag_names(flags),
                failures.is_empty(),
                if failures.is_empty() { String::new() } else { format!(" ({})", failures.join("; ")) },
            ),
        );
    }

    println!("All checks of containing C errors passed: **{all_ok}**");
    std::process::exit(if all_ok { 0 } else { 1 });
}

/// The limits, checked in a child: three headers claiming enormous Images, and a progressive
/// JPEG with one scan repeated past the scan limit.
fn limits() {
    let heap0 = unsafe { bil_heap_in_use() };
    let mut refused = 0;
    for codec in [Codec::Jpeg, Codec::J2k, Codec::Jp2] {
        let mut d = seeds(codec)[0].data.clone();
        if codec == Codec::Jpeg {
            // SOF's height and width, after the marker, length and precision: 65,000 × 65,000.
            let sof = (0..d.len() - 1)
                .find(|&i| d[i] == 0xFF && (d[i + 1] == 0xC0 || d[i + 1] == 0xC2))
                .unwrap();
            d[sof + 5..sof + 9].copy_from_slice(&[0xFD, 0xE8, 0xFD, 0xE8]);
        } else {
            // SIZ, after its marker, length and capabilities: Xsiz and Ysiz, then XTsiz and
            // YTsiz, so that one tile still covers the Image: 65,536 × 65,536.
            let siz = (0..d.len() - 1)
                .find(|&i| d[i] == 0xFF && d[i + 1] == 0x51)
                .unwrap();
            d[siz + 6..siz + 14].copy_from_slice(&[0, 1, 0, 0, 0, 1, 0, 0]);
            d[siz + 22..siz + 30].copy_from_slice(&[0, 1, 0, 0, 0, 1, 0, 0]);
            if codec == Codec::Jp2 {
                // The JP2 header box states the size too, as height and then width, and must
                // agree with the codestream's.
                let ihdr = d.windows(4).position(|w| w == b"ihdr").unwrap();
                d[ihdr + 4..ihdr + 12].copy_from_slice(&[0, 1, 0, 0, 0, 1, 0, 0]);
            }
        }
        refused += usize::from(decode_with(codec, &d, &LIMITS, false).0 == Outcome::Refused);
    }
    let taken = unsafe { bil_heap_in_use() } as i64 - heap0 as i64;
    // The last scan of a progressive seed, from its SOS marker to the end of the image, repeated.
    let seed = &seeds(Codec::Jpeg)[1].data;
    let sos = (0..seed.len() - 1)
        .rev()
        .find(|&i| seed[i] == 0xFF && seed[i + 1] == 0xDA)
        .unwrap();
    let eoi = seed.len() - 2;
    let mut many = seed[..eoi].to_vec();
    for _ in 0..600 {
        many.extend_from_slice(&seed[sos..eoi]);
    }
    many.extend_from_slice(&[0xFF, 0xD9]);
    let scans = match decode_with(Codec::Jpeg, &many, &LIMITS, false).0 {
        Outcome::Error(m) if m.contains("progressive scans") => "stopped".to_string(),
        other => format!("not stopped ({other:?})"),
    };
    println!("refused={refused}");
    println!("taken={taken}");
    println!("scans={scans}");
}

/// GDAL's limits, checked in a child: a NITF file and a TIFF file whose headers claim
/// 65,000 × 65,000 pixels, and a TIFF file whose one block would take 7.85 GB.
fn gdal_limits() {
    let mut files = Vec::new();
    // NITF: the first seed, 64 × 48 pixels in one block. The image subheader gives NROWS and
    // NCOLS, then, after the bands, one block per row and column (NBPR, NBPC) and the block's
    // size (NPPBH, NPPBV). A block size of 0 with one block means the whole Image, as
    // MIL-STD-2500C allows, so the claimed size stays consistent.
    let mut d = seeds(Codec::Nitf)[0].data.clone();
    let replace = |d: &mut Vec<u8>, from: &[u8], to: &[u8]| {
        let at = d
            .windows(from.len())
            .position(|w| w == from)
            .unwrap_or_else(|| panic!("{:?} not found", String::from_utf8_lossy(from)));
        d[at..at + to.len()].copy_from_slice(to);
    };
    replace(&mut d, b"0000004800000064", b"0006500000065000");
    replace(&mut d, b"0001000100640048", b"0001000100000000");
    files.push((Codec::Nitf, d));
    // TIFF: the first seed, in strips, little-endian. Its width, length and rows per strip, so
    // that one strip still covers the Image.
    let mut d = seeds(Codec::Tiff)[0].data.clone();
    assert_eq!(
        &d[..4],
        b"II*\0",
        "the TIFF seed is classic little-endian TIFF"
    );
    let ifd = u32::from_le_bytes(d[4..8].try_into().unwrap()) as usize;
    let count = usize::from(u16::from_le_bytes([d[ifd], d[ifd + 1]]));
    for e in 0..count {
        let at = ifd + 2 + 12 * e;
        let tag = u16::from_le_bytes([d[at], d[at + 1]]);
        let kind = u16::from_le_bytes([d[at + 2], d[at + 3]]);
        if matches!(tag, 256 | 257 | 278) {
            // SHORT (3) or LONG (4), held in the entry itself.
            let v = 65_000u32.to_le_bytes();
            let n = if kind == 3 { 2 } else { 4 };
            d[at + 8..at + 8 + n].copy_from_slice(&v[..n]);
        }
    }
    files.push((Codec::Tiff, d));
    // TIFF file 1,197,389 of the corrupt files: 40 × 40 pixels of 14,974 bands, within the
    // limits, in one tile of 16 × 32,784 pixels, for which GDAL asks 7.85 GB at once. It is
    // refused by the cap on a library's working memory, twice max_decoded_bytes.
    files.push((Codec::Tiff, corrupt(&seeds(Codec::Tiff), 1_197_389)));
    // GDAL sets up some things once, on its first read with a driver; that is not memory taken
    // for a refused file.
    for codec in [Codec::Nitf, Codec::Tiff] {
        for s in seeds(codec) {
            decode_with(codec, &s.data, &LIMITS, false);
        }
    }
    let heap0 = unsafe { bil_heap_in_use() };
    let mut refused = 0;
    for (codec, d) in &files {
        let outcome = decode_with(*codec, d, &LIMITS, false).0;
        if std::env::var_os("EXP3_DEBUG").is_some() {
            eprintln!("{}: {outcome:?}", codec.name());
        }
        refused += usize::from(outcome == Outcome::Refused);
    }
    let taken = unsafe { bil_heap_in_use() } as i64 - heap0 as i64;
    println!("refused={refused}");
    println!("taken={taken}");
}

/// The floating-point exception flags `bil_fp_flags` reports, by name.
fn flag_names(flags: u32) -> String {
    let names: &[(u32, &str)] = if cfg!(target_arch = "x86_64") {
        &[
            (0x01, "invalid"),
            (0x02, "denormal"),
            (0x04, "divide by zero"),
            (0x08, "overflow"),
            (0x10, "underflow"),
            (0x20, "inexact"),
            (0x0100, "x87 invalid"),
            (0x0200, "x87 denormal"),
            (0x0400, "x87 divide by zero"),
            (0x0800, "x87 overflow"),
            (0x1000, "x87 underflow"),
            (0x2000, "x87 inexact"),
        ]
    } else if cfg!(target_arch = "aarch64") {
        &[
            (0x01, "invalid"),
            (0x02, "divide by zero"),
            (0x04, "overflow"),
            (0x08, "underflow"),
            (0x10, "inexact"),
            (0x80, "input denormal"),
        ]
    } else {
        &[]
    };
    let found: Vec<&str> = names
        .iter()
        .filter(|(b, _)| flags & b != 0)
        .map(|(_, n)| *n)
        .collect();
    if found.is_empty() {
        "none".into()
    } else {
        found.join(", ")
    }
}

/// Decodes one corrupt file with the library's own error handling, in a child process.
fn control(codec: Codec) {
    let s = &seeds(codec)[0];
    // Not a JPEG or JPEG 2000 file at all: the first bytes are wrong.
    let mut d = s.data.clone();
    d[0] = 0;
    d[1] = 0;
    unsafe {
        match codec {
            Codec::Jpeg => bil_jpeg_decode_unprotected(d.as_ptr(), d.len()),
            Codec::Nitf | Codec::Tiff => {
                bil_gdal_decode_unprotected(d.as_ptr(), d.len(), codec.format())
            }
            _ => bil_j2k_decode_unprotected(d.as_ptr(), d.len(), codec.format()),
        };
    }
    println!("returned");
}
