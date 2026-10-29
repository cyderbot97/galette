//! Extraction of a track's audio from the drive.
//!
//! In secure mode, a track whose first read has no C2 error and matches
//! AccurateRip is done: it is proven correct. Otherwise it is read a second
//! time and both reads are compared; every sector that differs or had C2
//! errors is re-read, emptying the drive's cache first, until
//! [`REQUIRED_MATCHES`] error-free reads agree. Sectors that never reach
//! that agreement are reported as suspicious.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{Error, Result};
use crate::drive::{Drive, mmc};
use crate::toc::{SECTOR_BYTES, Toc};

pub const SAMPLES_PER_SECTOR: i64 = 588;
const BYTES_PER_SAMPLE: i64 = 4;

/// Sectors per READ CD command: 24 × (2352 + 294) bytes stays under the
/// 64 KiB that every USB bridge accepts, even with C2 pointers.
const SECTORS_PER_READ: u32 = 24;
/// Attempts at reading a sector the drive rejects, in a single pass.
const ATTEMPTS: usize = 3;
/// Re-read rounds for sectors that read differently.
pub const MAX_REREADS: u32 = 20;
/// Identical error-free reads needed to trust a sector that read differently.
pub const REQUIRED_MATCHES: u32 = 3;
/// Cache size assumed at least, since drives do not always report it.
const MIN_CACHE_KIB: u32 = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// One read per sector.
    Burst,
    /// Re-reads until the audio is proven or confirmed (see the module doc).
    Secure,
}

#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    pub mode: Mode,
    /// Read offset of the drive, in samples.
    pub offset: i32,
    /// Ask the drive for C2 error pointers.
    pub c2: bool,
    /// Size of the drive's read cache.
    pub cache_kib: u32,
    /// Set from another thread to stop the rip as soon as possible.
    pub cancel: Option<&'a AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Progress {
    /// First read, fraction done.
    Reading(f32),
    /// Second read, fraction done.
    Verifying(f32),
    /// Re-reading the sectors that are still uncertain.
    Rereading { attempt: u32, sectors: usize },
}

/// How the reading went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadReport {
    /// Full reads of the track: 1, or 2 when the first was not proven correct.
    pub passes: u32,
    /// Sectors with C2 errors in at least one read.
    pub c2_sectors: u32,
    /// Sectors that differed between the first two reads.
    pub mismatches: u32,
    /// Sector reads done to resolve them.
    pub rereads: u32,
    /// LBAs of the sectors whose content is not confirmed.
    pub suspicious: Vec<u32>,
    /// LBAs of the sectors that could not be read at all (left silent).
    pub unreadable: Vec<u32>,
    /// Positions that were uncertain, until the final audio turned out to
    /// match AccurateRip, which proves it correct.
    pub cleared_by_accuraterip: usize,
}

impl ReadReport {
    pub fn is_clean(&self) -> bool {
        self.suspicious.is_empty() && self.unreadable.is_empty()
    }
}

pub struct TrackRead {
    pub audio: Vec<u8>,
    pub report: ReadReport,
}

/// Reads the audio of the track at `index` in [`Toc::tracks`].
///
/// The read offset is applied: with an offset of +6, the drive delivers each
/// sample 6 positions too early, so we read 6 samples further. Samples
/// falling outside the disc (before LBA 0 or after the lead-out) cannot be
/// read by most drives and are left silent.
///
/// `is_accurate` tells whether audio matches AccurateRip; a track it accepts
/// after one clean read is not read again.
pub fn read_track(
    drive: &mut Drive,
    toc: &Toc,
    index: usize,
    options: Options<'_>,
    is_accurate: &mut dyn FnMut(&[u8]) -> bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<TrackRead> {
    let span = Span::new(toc, index, options.offset);
    let mut reader = Reader { drive, c2: options.c2, cancel: options.cancel };

    let first = reader.read(span.sectors.clone(), &mut |f| progress(Progress::Reading(f)))?;
    let audio = span.crop(&first.data);
    let first_clean = first.flags.iter().all(|f| *f == Flag::Good);
    if options.mode == Mode::Burst || (first_clean && is_accurate(&audio)) {
        let report = ReadReport {
            passes: 1,
            c2_sectors: first.flags.iter().filter(|f| **f == Flag::C2).count() as u32,
            unreadable: first.lbas(Flag::Failed),
            ..Default::default()
        };
        return Ok(TrackRead { audio, report });
    }

    let disc_end = toc.audio_lead_out();
    let cache_sectors = options.cache_kib.max(MIN_CACHE_KIB) * 1024 / SECTOR_BYTES as u32;
    reader.flush_cache(span.sectors.start, disc_end, cache_sectors);
    let second = reader.read(span.sectors.clone(), &mut |f| progress(Progress::Verifying(f)))?;

    let mut report = ReadReport { passes: 2, ..Default::default() };
    let mut votes: HashMap<usize, Votes> = HashMap::new();
    for i in 0..first.flags.len() {
        let same = first.sector(i) == second.sector(i);
        if same && first.flags[i] == Flag::Good && second.flags[i] == Flag::Good {
            continue;
        }
        report.mismatches += u32::from(!same);
        report.c2_sectors += u32::from(first.flags[i] == Flag::C2 || second.flags[i] == Flag::C2);
        let sector_votes = votes.entry(i).or_default();
        sector_votes.add(&first, i);
        sector_votes.add(&second, i);
    }

    let mut pending: Vec<usize> = votes.iter().filter(|(_, v)| !v.confirmed()).map(|(i, _)| *i).collect();
    pending.sort_unstable();
    for attempt in 1..=MAX_REREADS {
        if pending.is_empty() {
            break;
        }
        progress(Progress::Rereading { attempt, sectors: pending.len() });
        reader.flush_cache(span.sectors.start, disc_end, cache_sectors);
        for run in runs(&pending) {
            let start = span.sectors.start + run.start as u32;
            let pass = reader.read(start..start + run.len() as u32, &mut |_| {})?;
            for (j, i) in run.enumerate() {
                votes.get_mut(&i).unwrap().add(&pass, j);
                report.rereads += 1;
            }
        }
        pending.retain(|i| !votes[i].confirmed());
    }

    let mut data = first.data;
    for (&i, sector_votes) in &votes {
        let lba = span.sectors.start + i as u32;
        let target = &mut data[i * SECTOR_BYTES..(i + 1) * SECTOR_BYTES];
        match sector_votes.best() {
            Some(best) => {
                target.copy_from_slice(best);
                if !sector_votes.confirmed() {
                    report.suspicious.push(lba);
                }
            }
            None => {
                target.fill(0);
                report.unreadable.push(lba);
            }
        }
    }
    report.suspicious.sort_unstable();
    report.unreadable.sort_unstable();
    let audio = span.crop(&data);
    if !report.is_clean() && is_accurate(&audio) {
        report.cleared_by_accuraterip = report.suspicious.len() + report.unreadable.len();
        report.suspicious.clear();
        report.unreadable.clear();
    }
    Ok(TrackRead { audio, report })
}

/// Where a track's audio lies on the disc, once the read offset is applied.
struct Span {
    /// Position of the track's first byte on the disc (may be negative).
    start: i64,
    len: usize,
    /// Sectors holding the readable part of it.
    sectors: Range<u32>,
    /// Readable part, in bytes from the start of the disc.
    readable: Range<i64>,
}

impl Span {
    fn new(toc: &Toc, index: usize, offset: i32) -> Self {
        let sector = SECTOR_BYTES as i64;
        let len = toc.track_sectors(index) as i64 * sector;
        let start = toc.tracks[index].start as i64 * sector + offset as i64 * BYTES_PER_SAMPLE;
        let disc_end = toc.audio_lead_out() as i64 * sector;
        let readable = start.max(0)..(start + len).min(disc_end);
        let sectors = (readable.start / sector) as u32..(readable.end as u64).div_ceil(sector as u64) as u32;
        Self { start, len: len as usize, sectors, readable }
    }

    /// Copies the track's part of the raw sectors to its place in the track.
    fn crop(&self, raw: &[u8]) -> Vec<u8> {
        let mut audio = vec![0u8; self.len];
        let raw_start = self.sectors.start as i64 * SECTOR_BYTES as i64;
        let src = (self.readable.start - raw_start) as usize..(self.readable.end - raw_start) as usize;
        let dst = (self.readable.start - self.start) as usize;
        audio[dst..dst + src.len()].copy_from_slice(&raw[src]);
        audio
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flag {
    Good,
    /// Read, but the drive reported uncorrected errors.
    C2,
    /// The drive refused to read it.
    Failed,
}

/// One read of a range of sectors.
struct Pass {
    data: Vec<u8>,
    flags: Vec<Flag>,
    first_lba: u32,
}

impl Pass {
    fn sector(&self, i: usize) -> &[u8] {
        &self.data[i * SECTOR_BYTES..(i + 1) * SECTOR_BYTES]
    }

    fn lbas(&self, flag: Flag) -> Vec<u32> {
        (0..self.flags.len()).filter(|&i| self.flags[i] == flag).map(|i| self.first_lba + i as u32).collect()
    }
}

struct Reader<'a> {
    drive: &'a mut Drive,
    c2: bool,
    cancel: Option<&'a AtomicBool>,
}

impl Reader<'_> {
    fn read(&mut self, sectors: Range<u32>, progress: &mut dyn FnMut(f32)) -> Result<Pass> {
        let count = (sectors.end - sectors.start) as usize;
        let mut pass = Pass { data: vec![0; count * SECTOR_BYTES], flags: vec![Flag::Good; count], first_lba: sectors.start };
        let mut lba = sectors.start;
        while lba < sectors.end {
            if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(Error::Cancelled);
            }
            let n = SECTORS_PER_READ.min(sectors.end - lba);
            let i = (lba - sectors.start) as usize;
            match self.read_chunk(lba, n, &mut pass, i) {
                Ok(()) => {}
                Err(e) if e.is_drive_failure() => return Err(e),
                // Find out which sectors are unreadable, one by one.
                Err(_) => {
                    for k in 0..n {
                        if !self.read_sector(lba + k, &mut pass, i + k as usize)? {
                            pass.flags[i + k as usize] = Flag::Failed;
                        }
                    }
                }
            }
            lba += n;
            progress((lba - sectors.start) as f32 / count as f32);
        }
        Ok(pass)
    }

    /// Tries a few times to read one sector; `false` if the disc refuses.
    fn read_sector(&mut self, lba: u32, pass: &mut Pass, i: usize) -> Result<bool> {
        for _ in 0..ATTEMPTS {
            match self.read_chunk(lba, 1, pass, i) {
                Ok(()) => return Ok(true),
                Err(e) if e.is_drive_failure() => return Err(e),
                Err(_) => {}
            }
        }
        Ok(false)
    }

    /// Reads `n` sectors from `lba` into `pass`, at sector `i`.
    fn read_chunk(&mut self, lba: u32, n: u32, pass: &mut Pass, i: usize) -> Result<()> {
        let data = &mut pass.data[i * SECTOR_BYTES..(i + n as usize) * SECTOR_BYTES];
        if !self.c2 {
            return self.drive.read_audio(lba, n, data);
        }
        let stride = SECTOR_BYTES + mmc::C2_LEN;
        let mut buf = vec![0u8; n as usize * stride];
        self.drive.read_audio_c2(lba, n, &mut buf)?;
        for (k, raw) in buf.chunks_exact(stride).enumerate() {
            data[k * SECTOR_BYTES..(k + 1) * SECTOR_BYTES].copy_from_slice(&raw[..SECTOR_BYTES]);
            let has_errors = raw[SECTOR_BYTES..].iter().any(|&b| b != 0);
            pass.flags[i + k] = if has_errors { Flag::C2 } else { Flag::Good };
        }
        Ok(())
    }

    /// Fills the drive's cache with another part of the disc, so that the
    /// next read of `near` really reads the disc again.
    fn flush_cache(&mut self, near: u32, disc_end: u32, cache_sectors: u32) {
        let len = (cache_sectors + SECTORS_PER_READ).min(disc_end / 4);
        let start = if near < disc_end / 2 { disc_end - len } else { 0 };
        let mut buf = vec![0u8; SECTORS_PER_READ as usize * SECTOR_BYTES];
        for lba in (start..start + len).step_by(SECTORS_PER_READ as usize) {
            let n = SECTORS_PER_READ.min(start + len - lba);
            // A failure here only means this part of the disc is damaged too.
            let _ = self.drive.read_audio(lba, n, &mut buf[..n as usize * SECTOR_BYTES]);
        }
    }
}

/// The versions read for one sector, with how often each was read with and
/// without C2 errors.
#[derive(Default)]
struct Votes {
    versions: Vec<(Vec<u8>, u32, u32)>,
}

impl Votes {
    fn add(&mut self, pass: &Pass, i: usize) {
        let clean = match pass.flags[i] {
            Flag::Failed => return,
            flag => flag == Flag::Good,
        };
        let data = pass.sector(i);
        let entry = match self.versions.iter_mut().position(|(d, _, _)| d == data) {
            Some(k) => &mut self.versions[k],
            None => {
                self.versions.push((data.to_vec(), 0, 0));
                self.versions.last_mut().unwrap()
            }
        };
        if clean { entry.1 += 1 } else { entry.2 += 1 }
    }

    fn confirmed(&self) -> bool {
        self.versions.iter().any(|(_, clean, _)| *clean >= REQUIRED_MATCHES)
    }

    /// The most often read version, error-free reads first.
    fn best(&self) -> Option<&[u8]> {
        self.versions.iter().max_by_key(|(_, clean, c2)| (*clean, *c2)).map(|(d, _, _)| d.as_slice())
    }
}

/// Splits sorted sector indexes into runs of consecutive ones, short enough
/// for one read command each.
fn runs(indexes: &[usize]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for &i in indexes {
        match runs.last_mut() {
            Some(run) if run.end == i && run.len() < SECTORS_PER_READ as usize => run.end += 1,
            _ => runs.push(i..i + 1),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use super::*;
    use crate::drive::Transport;
    use crate::error::{Error, ScsiError};

    /// How a damaged sector behaves.
    #[derive(Clone, Copy)]
    enum Damage {
        /// Wrong data on the first `n` reads, flagged by C2 or not.
        WrongFirst { n: u32, c2: bool },
        /// The drive refuses to read it.
        Unreadable,
        /// Different garbage on every read.
        Random,
        /// Right data, always with C2 errors.
        FlaggedButRight,
    }

    /// A disc holding samples numbered from 1 at LBA 0, with damaged sectors.
    #[derive(Default)]
    struct FakeDisc {
        damage: HashMap<u32, Damage>,
        reads: HashMap<u32, u32>,
    }

    impl Transport for FakeDisc {
        fn read(&mut self, cdb: &[u8], buf: &mut [u8], _: Duration) -> Result<usize> {
            let lba = u32::from_be_bytes(cdb[2..6].try_into().unwrap());
            let count = u32::from_be_bytes([0, cdb[6], cdb[7], cdb[8]]);
            let stride = buf.len() / count as usize;
            for k in 0..count {
                let sector = lba + k;
                let reads = self.reads.entry(sector).or_default();
                *reads += 1;
                let raw = &mut buf[k as usize * stride..(k as usize + 1) * stride];
                for (i, sample) in raw[..SECTOR_BYTES].as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    *sample = (sector * 588 + i as u32 + 1).to_le_bytes();
                }
                raw[SECTOR_BYTES..].fill(0);
                match self.damage.get(&sector) {
                    Some(Damage::Unreadable) => {
                        return Err(Error::Scsi(ScsiError { opcode: cdb[0], sense_key: 3, asc: 0x11, ascq: 5 }));
                    }
                    Some(Damage::WrongFirst { n, c2 }) if *reads <= *n => {
                        raw[100] ^= 0xFF;
                        if *c2 && stride > SECTOR_BYTES {
                            raw[SECTOR_BYTES + 12] = 0x80;
                        }
                    }
                    Some(Damage::Random) => raw[100] = *reads as u8,
                    Some(Damage::FlaggedButRight) if stride > SECTOR_BYTES => raw[SECTOR_BYTES] = 1,
                    _ => {}
                }
            }
            Ok(buf.len())
        }
    }

    const TOC: &str = "1 2 250 150 200";

    fn rip(disc: FakeDisc, index: usize, offset: i32, mode: Mode, accurate: bool) -> TrackRead {
        let toc = Toc::parse_musicbrainz(TOC).unwrap();
        let mut drive = Drive::new(Box::new(disc));
        let options = Options { mode, offset, c2: true, cache_kib: 0, cancel: None };
        read_track(&mut drive, &toc, index, options, &mut |_| accurate, &mut |_| {}).unwrap()
    }

    fn samples(audio: &[u8]) -> Vec<u32> {
        audio.as_chunks::<4>().0.iter().map(|s| u32::from_le_bytes(*s)).collect()
    }

    /// The samples a perfect rip of track 2 (LBA 50 to 100) gives.
    fn expected_track_2() -> Vec<u32> {
        (50 * 588 + 1..=100 * 588).collect()
    }

    #[test]
    fn applies_read_offset() {
        let track = samples(&rip(FakeDisc::default(), 1, 6, Mode::Burst, false).audio);
        assert_eq!(track.len(), 50 * 588);
        assert_eq!(track[0], 50 * 588 + 7);
        // Past the lead-out: silence.
        assert_eq!(&track[track.len() - 6..], &[0; 6]);
        assert_eq!(track[track.len() - 7], 100 * 588);
    }

    #[test]
    fn negative_offset_at_disc_start() {
        let track = samples(&rip(FakeDisc::default(), 0, -30, Mode::Burst, false).audio);
        assert_eq!(&track[..30], &[0; 30]);
        assert_eq!(track[30], 1);
        assert_eq!(track[track.len() - 1], 50 * 588 - 30);
    }

    #[test]
    fn accurate_track_is_read_once() {
        let read = rip(FakeDisc::default(), 1, 0, Mode::Secure, true);
        assert_eq!(read.report, ReadReport { passes: 1, ..Default::default() });
    }

    #[test]
    fn unverified_clean_track_is_read_twice() {
        let read = rip(FakeDisc::default(), 1, 0, Mode::Secure, false);
        assert_eq!(read.report, ReadReport { passes: 2, ..Default::default() });
        assert_eq!(samples(&read.audio), expected_track_2());
    }

    #[test]
    fn corrects_sectors_read_wrong_once() {
        let mut disc = FakeDisc::default();
        disc.damage.insert(60, Damage::WrongFirst { n: 1, c2: false });
        disc.damage.insert(70, Damage::WrongFirst { n: 2, c2: true });
        // Even AccurateRip accepting the bad first read must not stop a C2 error.
        let read = rip(disc, 1, 0, Mode::Secure, true);
        assert_eq!(samples(&read.audio), expected_track_2());
        assert_eq!(read.report.passes, 2);
        assert_eq!(read.report.mismatches, 1);
        assert_eq!(read.report.c2_sectors, 1);
        assert!(read.report.is_clean());
    }

    #[test]
    fn reports_unstable_and_unreadable_sectors() {
        let mut disc = FakeDisc::default();
        disc.damage.insert(60, Damage::Random);
        disc.damage.insert(80, Damage::Unreadable);
        let read = rip(disc, 1, 0, Mode::Secure, false);
        assert_eq!(read.report.suspicious, [60]);
        assert_eq!(read.report.unreadable, [80]);
        // Both sectors are re-read in every round.
        assert_eq!(read.report.rereads, 2 * MAX_REREADS);
        let audio = samples(&read.audio);
        assert_eq!(&audio[30 * 588..31 * 588], &[0; 588]);
        assert_eq!(audio[31 * 588], 81 * 588 + 1);
    }

    #[test]
    fn accuraterip_clears_doubtful_sectors() {
        // Always flagged by C2, yet always read right: never 3 error-free reads.
        let mut disc = FakeDisc::default();
        disc.damage.insert(60, Damage::FlaggedButRight);
        let toc = Toc::parse_musicbrainz(TOC).unwrap();
        let mut drive = Drive::new(Box::new(disc));
        let options = Options { mode: Mode::Secure, offset: 0, c2: true, cache_kib: 0, cancel: None };
        let expected = expected_track_2();
        let mut is_accurate = |audio: &[u8]| samples(audio) == expected;
        let read = read_track(&mut drive, &toc, 1, options, &mut is_accurate, &mut |_| {}).unwrap();
        assert!(read.report.is_clean());
        assert_eq!(read.report.cleared_by_accuraterip, 1);
        assert_eq!(read.report.rereads, MAX_REREADS);
    }

    #[test]
    fn burst_mode_reports_unreadable_sectors() {
        let mut disc = FakeDisc::default();
        disc.damage.insert(80, Damage::Unreadable);
        let read = rip(disc, 1, 0, Mode::Burst, false);
        assert_eq!(read.report.unreadable, [80]);
        assert_eq!(read.report.passes, 1);
    }

    #[test]
    fn can_be_cancelled() {
        let toc = Toc::parse_musicbrainz(TOC).unwrap();
        let mut drive = Drive::new(Box::new(FakeDisc::default()));
        let cancel = AtomicBool::new(true);
        let options = Options { mode: Mode::Secure, offset: 0, c2: true, cache_kib: 0, cancel: Some(&cancel) };
        let result = read_track(&mut drive, &toc, 1, options, &mut |_| false, &mut |_| {});
        assert!(matches!(result, Err(Error::Cancelled)));
    }

    /// A drive that stops answering, like a USB bridge that lost track.
    struct DeadDrive;

    impl Transport for DeadDrive {
        fn read(&mut self, _: &[u8], _: &mut [u8], _: Duration) -> Result<usize> {
            Err(Error::DriveNotResponding("host status 0x07".into()))
        }
    }

    #[test]
    fn stops_when_the_drive_stops_responding() {
        let toc = Toc::parse_musicbrainz(TOC).unwrap();
        let mut drive = Drive::new(Box::new(DeadDrive));
        let options = Options { mode: Mode::Secure, offset: 0, c2: true, cache_kib: 0, cancel: None };
        let result = read_track(&mut drive, &toc, 1, options, &mut |_| false, &mut |_| {});
        assert!(matches!(result, Err(Error::DriveNotResponding(_))));
    }

    #[test]
    fn groups_runs() {
        assert_eq!(runs(&[1, 2, 3, 7, 8, 40]), [1..4, 7..9, 40..41]);
    }
}
