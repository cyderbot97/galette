//! Finding a drive's read offset with a disc known to AccurateRip, for drives
//! missing from its offset database.
//!
//! A track is read with a few sectors of margin on each side, then its
//! AccurateRip checksum is computed at every offset within
//! ±[`MAX_OFFSET`] samples: the offset at which it matches the database is
//! the drive's. The v1 checksum slides from one offset to the next in
//! constant time; v2, which does not, is only tried at the offsets of known
//! drives. Only middle tracks are used, since AccurateRip skips samples at the
//! disc's edges, and two of them must agree.
//!
//! A disc pressed several times may match at several offsets: pressings are
//! often shifted from one another, and the database holds them all under
//! the same TOC (one disc tested matched at 17 offsets, the drive's being
//! only the 7th most confirmed). Only another disc tells them apart: its
//! pressings have no reason to be shifted the same way, so the offset that
//! both discs match is the drive's. See [`among`].

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use crate::Result;
use crate::accuraterip::{self, Pressing};
use crate::drive::Drive;
use crate::rip::{self, Mode, SAMPLES_PER_SECTOR};
use crate::toc::Toc;

/// Largest offset tried, in samples. Known drives stay well within it.
pub const MAX_OFFSET: i32 = 3000;
/// Sectors read before and after the track, to cover every offset tried.
const MARGIN_SECTORS: u32 = (MAX_OFFSET as u32).div_ceil(SAMPLES_PER_SECTOR as u32);
/// Tracks read: their offsets must agree.
const TRACKS: usize = 2;

/// An offset at which the tracks read match AccurateRip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub offset: i32,
    /// The lowest confidence of the matching checksums of the tracks.
    pub confidence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// Offsets at which every track read matches, most confirmed first. If
    /// there are several, another disc must tell which is the drive's.
    Found(Vec<Match>),
    /// No offset matches: the disc may be damaged or another pressing.
    NoMatch,
    /// The disc has no middle track known to AccurateRip.
    UnsuitableDisc,
}

/// Reads up to two middle tracks of the disc and finds the offset at which
/// they match `pressings`. `known` lists the offsets of known drives, where
/// v2 checksums are tried as well. `progress` receives the track being read
/// and the fraction read.
pub fn detect(
    drive: &mut Drive,
    toc: &Toc,
    pressings: &[Pressing],
    known: &[i32],
    cancel: Option<&AtomicBool>,
    progress: &mut dyn FnMut(u8, f32),
) -> Result<Detection> {
    let audio: Vec<usize> = (0..toc.tracks.len()).filter(|&i| toc.tracks[i].is_audio).collect();
    let confidence = |position: usize| {
        pressings.iter().filter_map(|p| p.tracks.get(position)).map(|s| u32::from(s.confidence)).max().unwrap_or(0)
    };
    // The most confirmed middle tracks.
    let mut middle: Vec<usize> = (1..audio.len().saturating_sub(1)).filter(|&p| confidence(p) > 0).collect();
    middle.sort_by_key(|&p| std::cmp::Reverse(confidence(p)));
    middle.truncate(TRACKS);
    if middle.is_empty() {
        return Ok(Detection::UnsuitableDisc);
    }

    let options = rip::Options { mode: Mode::Burst, offset: 0, c2: false, cache_kib: 0, cancel };
    let mut common: Option<HashMap<i32, u32>> = None;
    for position in middle {
        let index = audio[position];
        let track = &toc.tracks[index];
        let sectors = track.start - MARGIN_SECTORS..track.start + toc.track_sectors(index) + MARGIN_SECTORS;
        let read = rip::read_range(drive, toc, sectors, options, &mut |_| false, &mut |p| {
            if let rip::Progress::Reading(f) = p {
                progress(track.number, f);
            }
        })?;
        let mut sums: HashMap<u32, u32> = HashMap::new();
        for submission in pressings.iter().filter_map(|p| p.tracks.get(position)) {
            let best = sums.entry(submission.checksum).or_default();
            *best = (*best).max(u32::from(submission.confidence));
        }
        let len = toc.track_sectors(index) as usize * SAMPLES_PER_SECTOR as usize;
        let margin = (MARGIN_SECTORS * SAMPLES_PER_SECTOR as u32) as usize;
        let found = matching_offsets(&read.audio, margin, len, &sums, known);
        common = Some(match common {
            None => found,
            Some(previous) => previous
                .into_iter()
                .filter_map(|(offset, c)| found.get(&offset).map(|&d| (offset, c.min(d))))
                .collect(),
        });
    }

    let mut matches: Vec<Match> =
        common.unwrap_or_default().into_iter().map(|(offset, confidence)| Match { offset, confidence }).collect();
    matches.sort_by_key(|m| (std::cmp::Reverse(m.confidence), m.offset.abs()));
    Ok(if matches.is_empty() { Detection::NoMatch } else { Detection::Found(matches) })
}

/// The matches of a disc among the `candidates` that another disc left.
pub fn among(matches: &[Match], candidates: &[i32]) -> Vec<Match> {
    matches.iter().filter(|m| candidates.contains(&m.offset)).copied().collect()
}

/// The distinct offsets of the drives in AccurateRip's database, where v2
/// checksums are worth trying; empty if it cannot be downloaded.
pub fn known_offsets() -> Vec<i32> {
    let drives = accuraterip::fetch_drive_offsets().unwrap_or_default();
    let mut offsets: Vec<i32> = drives.iter().map(|d| d.offset).collect();
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

/// The offsets at which the `len` samples of a middle track, read with
/// `margin` samples before and after it, have a checksum among `sums`
/// (checksum → confidence), with that confidence.
fn matching_offsets(audio: &[u8], margin: usize, len: usize, sums: &HashMap<u32, u32>, known: &[i32]) -> HashMap<i32, u32> {
    let samples: Vec<u32> = audio.as_chunks::<4>().0.iter().map(|s| u32::from_le_bytes(*s)).collect();
    let max = MAX_OFFSET.min(margin as i32);
    let first = margin - max as usize;
    let mut found = HashMap::new();

    // v1 = Σ (i+1)·x[o+i] and the plain sum T = Σ x[o+i] over the window at
    // offset o. Moving the window by one sample:
    // v1(o+1) = v1(o) − T(o) + len·x[o+len], T(o+1) = T(o) − x[o] + x[o+len].
    let window = &samples[first..first + len];
    let mut v1 = window.iter().zip(1u32..).fold(0u32, |sum, (&x, m)| sum.wrapping_add(x.wrapping_mul(m)));
    let mut total = window.iter().fold(0u32, |sum, &x| sum.wrapping_add(x));
    for o in -max..=max {
        if let Some(&confidence) = sums.get(&v1) {
            found.insert(o, confidence);
        }
        let start = (margin as i32 + o) as usize;
        let Some(&next) = samples.get(start + len) else { break };
        v1 = v1.wrapping_sub(total).wrapping_add((len as u32).wrapping_mul(next));
        total = total.wrapping_sub(samples[start]).wrapping_add(next);
    }

    for &o in known.iter().filter(|o| o.abs() <= max) {
        let start = (margin as i32 + o) as usize * 4;
        // Position 2 of 3: a middle track, nothing skipped.
        let v2 = accuraterip::checksums(&audio[start..start + len * 4], 2, 3).v2;
        if let Some(&confidence) = sums.get(&v2) {
            found.insert(o, confidence);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::accuraterip::Submission;
    use crate::drive::Transport;
    use crate::toc::SECTOR_BYTES;

    /// The true value of sample `k` of the disc.
    fn sample(k: i64) -> u32 {
        (k as u32).wrapping_mul(2_654_435_761).rotate_left(13) ^ 0x5bd1_e995
    }

    /// A drive whose read offset is `offset`: the sample it delivers at
    /// position p is the disc's p − offset.
    struct OffsetDrive {
        offset: i64,
    }

    impl Transport for OffsetDrive {
        fn read(&mut self, cdb: &[u8], buf: &mut [u8], _: Duration) -> Result<usize> {
            let lba = u32::from_be_bytes(cdb[2..6].try_into().unwrap()) as i64;
            for (i, s) in buf.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *s = sample(lba * SECTOR_BYTES as i64 / 4 + i as i64 - self.offset).to_le_bytes();
            }
            Ok(buf.len())
        }
    }

    const TOC: &str = "1 4 4150 150 1150 2150 3150";

    /// The database's checksums for the disc: tracks 2 and 3 with v1 or v2.
    fn pressings(v2: bool) -> Vec<Pressing> {
        let toc = Toc::parse_musicbrainz(TOC).unwrap();
        let tracks = (0..4)
            .map(|i| {
                let start = toc.tracks[i].start as i64 * 588;
                let audio: Vec<u8> = (0..1000 * 588).flat_map(|k| sample(start + k).to_le_bytes()).collect();
                let sums = accuraterip::checksums(&audio, i + 1, 4);
                Submission { confidence: 20 + i as u8, checksum: if v2 { sums.v2 } else { sums.v1 } }
            })
            .collect();
        vec![Pressing { tracks }]
    }

    fn detect_with(drive_offset: i64, pressings: &[Pressing], known: &[i32]) -> Detection {
        let toc = Toc::parse_musicbrainz(TOC).unwrap();
        let mut drive = Drive::new(Box::new(OffsetDrive { offset: drive_offset }));
        detect(&mut drive, &toc, pressings, known, None, &mut |_, _| {}).unwrap()
    }

    #[test]
    fn finds_the_offset_from_v1_checksums() {
        for offset in [6, -472, 667, MAX_OFFSET as i64] {
            let found = detect_with(offset, &pressings(false), &[]);
            assert_eq!(found, Detection::Found(vec![Match { offset: offset as i32, confidence: 21 }]));
        }
    }

    #[test]
    fn tries_v2_checksums_at_known_offsets() {
        let found = detect_with(48, &pressings(true), &[6, 48, 102]);
        assert_eq!(found, Detection::Found(vec![Match { offset: 48, confidence: 21 }]));
        assert_eq!(detect_with(48, &pressings(true), &[6]), Detection::NoMatch);
    }

    #[test]
    fn needs_a_middle_track_in_the_database() {
        let toc = Toc::parse_musicbrainz("1 2 2150 150 1150").unwrap();
        let mut drive = Drive::new(Box::new(OffsetDrive { offset: 6 }));
        let result = detect(&mut drive, &toc, &pressings(false), &[], None, &mut |_, _| {}).unwrap();
        assert_eq!(result, Detection::UnsuitableDisc);
    }
}
