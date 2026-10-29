//! AccurateRip: a database of track checksums submitted by people who ripped
//! the same CD. If our checksum matches theirs, the rip is bit-perfect.
//!
//! The format is not officially documented; this follows the implementations
//! of EAC, CUETools and whipper.

use crate::ids::AccurateRipId;
use crate::toc::SECTOR_BYTES;
use crate::{Error, Result, http};

const OFFSETS_URL: &str = "http://www.accuraterip.com/accuraterip/DriveOffsets.bin";

/// Samples (of 4 bytes) excluded at the start of the first track and the end
/// of the last one, since many drives cannot read there.
const SKIPPED_SAMPLES: u32 = (5 * SECTOR_BYTES / 4) as u32;

/// Checksums of one track, in both versions of the algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checksums {
    pub v1: u32,
    pub v2: u32,
}

/// Computes the checksums of a track's audio (16-bit stereo, little endian).
///
/// `position` is the track's position among the audio tracks (from 1),
/// `track_count` the number of audio tracks.
pub fn checksums(audio: &[u8], position: usize, track_count: usize) -> Checksums {
    let samples = (audio.len() / 4) as u32;
    let first = if position == 1 { SKIPPED_SAMPLES } else { 0 };
    let last = if position == track_count { samples.saturating_sub(SKIPPED_SAMPLES) } else { samples };

    let mut v1 = 0u32;
    let mut high = 0u32;
    for (sample, multiplier) in audio.as_chunks::<4>().0.iter().map(|s| u32::from_le_bytes(*s)).zip(1u32..) {
        if multiplier >= first && multiplier <= last {
            let product = sample as u64 * multiplier as u64;
            v1 = v1.wrapping_add(product as u32);
            high = high.wrapping_add((product >> 32) as u32);
        }
    }
    Checksums { v1, v2: v1.wrapping_add(high) }
}

/// One set of submitted checksums. A disc usually has several: one per
/// pressing, and v1 and v2 checksums are submitted as separate sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pressing {
    pub tracks: Vec<Submission>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submission {
    /// How many people submitted this checksum.
    pub confidence: u8,
    pub checksum: u32,
}

/// Downloads the checksums of a disc. `None` if nobody submitted this disc yet.
pub fn fetch(id: &AccurateRipId) -> Result<Option<Vec<Pressing>>> {
    match http::get_bytes(&http::agent(), &id.url())? {
        Some(data) => parse(&data, id).map(Some),
        None => Ok(None),
    }
}

/// Parses a `dBAR-*.bin` file: a sequence of blocks made of a 13-byte header
/// (track count, disc IDs) and 9 bytes per track (confidence, checksum, and
/// a checksum of frame 450 used for offset detection, ignored here).
pub fn parse(data: &[u8], id: &AccurateRipId) -> Result<Vec<Pressing>> {
    let invalid = || Error::Network("malformed AccurateRip response".into());
    let u32_at = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap());

    let mut pressings = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let count = data[pos] as usize;
        let end = pos + 13 + count * 9;
        if end > data.len() {
            return Err(invalid());
        }
        if count != id.track_count as usize || u32_at(pos + 1) != id.id1 || u32_at(pos + 5) != id.id2 {
            return Err(invalid());
        }
        let tracks = (0..count)
            .map(|t| {
                let at = pos + 13 + t * 9;
                Submission { confidence: data[at], checksum: u32_at(at + 1) }
            })
            .collect();
        pressings.push(Pressing { tracks });
        pos = end;
    }
    Ok(pressings)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Matches a checksum submitted `confidence` times.
    Accurate { version: u8, confidence: u32 },
    /// The disc is in the database but this track matches no submission.
    Mismatch,
    /// The disc is not in the database.
    Unknown,
}

/// Compares a track (`index` among the audio tracks, from 0) to the database.
pub fn verify(pressings: Option<&[Pressing]>, index: usize, sums: Checksums) -> Verdict {
    let Some(pressings) = pressings else {
        return Verdict::Unknown;
    };
    let submissions = pressings.iter().filter_map(|p| p.tracks.get(index));
    let best = |checksum: u32| {
        submissions.clone().filter(|s| s.checksum == checksum).map(|s| s.confidence as u32).max()
    };
    // v2 first: it is the more reliable algorithm.
    match (best(sums.v2), best(sums.v1)) {
        (Some(confidence), _) => Verdict::Accurate { version: 2, confidence },
        (None, Some(confidence)) => Verdict::Accurate { version: 1, confidence },
        (None, None) => Verdict::Mismatch,
    }
}

/// Read offset of a drive model, in samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveOffset {
    /// As written by AccurateRip: `VENDOR - PRODUCT`.
    pub name: String,
    pub offset: i32,
    /// Number of people who confirmed this offset.
    pub submissions: u32,
}

pub fn fetch_drive_offsets() -> Result<Vec<DriveOffset>> {
    let data = http::get_bytes(&http::agent(), OFFSETS_URL)?
        .ok_or_else(|| Error::Network("AccurateRip drive offsets not found".into()))?;
    Ok(parse_drive_offsets(&data))
}

/// Parses `DriveOffsets.bin`: 69-byte records made of the offset (i16),
/// the drive name (33 bytes, NUL-padded) and the submission count (u32).
pub fn parse_drive_offsets(data: &[u8]) -> Vec<DriveOffset> {
    data.as_chunks::<69>()
        .0
        .iter()
        .map(|record| {
            let name = &record[2..35];
            let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
            DriveOffset {
                name: String::from_utf8_lossy(name).into_owned(),
                offset: i16::from_le_bytes([record[0], record[1]]) as i32,
                submissions: u32::from_le_bytes(record[35..39].try_into().unwrap()),
            }
        })
        .collect()
}

/// The best-confirmed offset for a drive, matching names regardless of
/// spacing and case (the database has both `ASUS     - X` and `ASUS - X`).
pub fn find_drive_offset<'a>(offsets: &'a [DriveOffset], vendor: &str, product: &str) -> Option<&'a DriveOffset> {
    let normalize = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_uppercase();
    let wanted = normalize(&format!("{vendor} - {product}"));
    offsets.iter().filter(|o| normalize(&o.name) == wanted).max_by_key(|o| o.submissions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive_record(offset: i16, name: &str, submissions: u32) -> Vec<u8> {
        let mut r = offset.to_le_bytes().to_vec();
        let mut field = [0u8; 33];
        field[..name.len()].copy_from_slice(name.as_bytes());
        r.extend(field);
        r.extend(submissions.to_le_bytes());
        r.resize(69, 0);
        r
    }

    #[test]
    fn drive_offset_lookup() {
        let mut data = drive_record(6, "ASUS     - SDRW-08U9M-U", 255);
        data.extend(drive_record(6, "ASUS - SDRW-08U9M-U", 1));
        data.extend(drive_record(-472, "PLEXTOR  - CD-R   PX-W4012A", 40));
        let offsets = parse_drive_offsets(&data);
        assert_eq!(offsets.len(), 3);
        assert_eq!(offsets[2].offset, -472);

        let found = find_drive_offset(&offsets, "ASUS", "SDRW-08U9M-U").unwrap();
        assert_eq!((found.offset, found.submissions), (6, 255));
        assert!(find_drive_offset(&offsets, "ASUS", "OTHER").is_none());
    }

    #[test]
    fn checksums_weight_samples_by_position() {
        // Two samples, 1 and 2, in a middle track: 1*1 + 2*2.
        let audio = [1, 0, 0, 0, 2, 0, 0, 0];
        assert_eq!(checksums(&audio, 2, 3), Checksums { v1: 5, v2: 5 });
        // A sample whose product overflows 32 bits: v2 adds the high part.
        let audio = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let sums = checksums(&audio, 2, 3);
        let expected = 0xFFFF_FFFFu64 * 3;
        assert_eq!(sums.v1, expected as u32);
        assert_eq!(sums.v2, (expected as u32).wrapping_add(((0xFFFF_FFFFu64 * 2) >> 32) as u32));
    }

    #[test]
    fn checksums_skip_disc_edges() {
        let audio = vec![1u8; 4 * 10_000];
        let middle = checksums(&audio, 2, 3);
        assert_ne!(checksums(&audio, 1, 3), middle);
        assert_ne!(checksums(&audio, 3, 3), middle);
        // A single-track disc skips both ends.
        let sample = u32::from_le_bytes([1; 4]) as u64;
        let sum: u64 = (SKIPPED_SAMPLES as u64..=10_000 - SKIPPED_SAMPLES as u64).map(|m| m * sample).sum();
        assert_eq!(checksums(&audio, 1, 1).v1, sum as u32);
    }

    #[test]
    fn parse_and_verify() {
        let id = AccurateRipId { track_count: 2, id1: 0x10, id2: 0x20, freedb: 0x30 };
        let block = |a: u32, b: u32| {
            let mut d = vec![2];
            for v in [0x10u32, 0x20, 0x30] {
                d.extend(v.to_le_bytes());
            }
            for (conf, sum) in [(12u8, a), (12, b)] {
                d.push(conf);
                d.extend(sum.to_le_bytes());
                d.extend(0u32.to_le_bytes());
            }
            d
        };
        let mut data = block(111, 222);
        data.extend(block(333, 444));
        let pressings = parse(&data, &id).unwrap();
        assert_eq!(pressings.len(), 2);

        let verdict = |v1, v2| verify(Some(&pressings), 1, Checksums { v1, v2 });
        assert_eq!(verdict(0, 444), Verdict::Accurate { version: 2, confidence: 12 });
        assert_eq!(verdict(222, 0), Verdict::Accurate { version: 1, confidence: 12 });
        assert_eq!(verdict(1, 2), Verdict::Mismatch);
        assert_eq!(verify(None, 0, Checksums { v1: 0, v2: 0 }), Verdict::Unknown);
    }
}
