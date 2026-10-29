//! Detection of pregaps: the part of a track before its INDEX 01, usually
//! silence, which players show as a countdown before the track starts.
//!
//! The TOC only gives INDEX 01 positions. The pregap of track N begins where
//! the Q sub-channel starts saying "track N" instead of "track N-1"; we find
//! that sector by binary search.

use crate::drive::mmc::QPosition;
use crate::toc::Toc;
use crate::Result;

/// Sectors tried after a probe whose Q frame carries no position.
const NEIGHBOURS: u32 = 3;

/// Length in sectors of each track's pregap, in the order of [`Toc::tracks`].
///
/// Track 1's pregap is whatever lies between LBA 0 and its start (usually
/// nothing; audio there is a hidden track). Data tracks are given 0.
/// `read_q` reads the Q sub-channel position of one sector.
pub fn detect_pregaps(
    toc: &Toc,
    read_q: &mut dyn FnMut(u32) -> Result<Option<QPosition>>,
) -> Result<Vec<u32>> {
    let mut pregaps = vec![toc.tracks[0].start];
    for pair in toc.tracks.windows(2) {
        let (previous, track) = (&pair[0], &pair[1]);
        if !previous.is_audio || !track.is_audio {
            pregaps.push(0);
            continue;
        }
        // Whether `lba` is past the end of `previous`, judging from the closest
        // sector with a position: the following ones, else the preceding ones.
        let mut in_track = |lba: u32| -> Result<Option<bool>> {
            let after = lba..(lba + NEIGHBOURS).min(track.start);
            let before = (lba.saturating_sub(NEIGHBOURS).max(previous.start + 1)..lba).rev();
            for candidate in after.chain(before) {
                if let Some(q) = read_q(candidate)? {
                    return Ok(Some(q.track >= track.number));
                }
            }
            Ok(None)
        };

        // Most tracks have no pregap: one read is enough to know.
        if in_track(track.start - 1)? != Some(true) {
            pregaps.push(0);
            continue;
        }
        // Invariant: `low` is before the pregap, `high` inside it.
        let (mut low, mut high) = (previous.start, track.start - 1);
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            match in_track(middle)? {
                Some(true) => high = middle,
                Some(false) => low = middle,
                None => break,
            }
        }
        pregaps.push(track.start - high);
    }
    Ok(pregaps)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Track 2 starts at 10 000 with a 150-sector pregap, track 3 at 20 000
    /// without. Every 100th frame carries an MCN instead of a position,
    /// including the one just before track 2.
    fn fake_q(lba: u32) -> Result<Option<QPosition>> {
        if lba % 100 == 42 || lba == 9_999 {
            return Ok(None);
        }
        let (track, index) = match lba {
            0..9_850 => (1, 1),
            9_850..10_000 => (2, 0),
            10_000..20_000 => (2, 1),
            _ => (3, 1),
        };
        Ok(Some(QPosition { track, index }))
    }

    #[test]
    fn finds_pregaps() {
        let toc = Toc::parse_musicbrainz("1 3 30150 150 10150 20150").unwrap();
        let pregaps = detect_pregaps(&toc, &mut fake_q).unwrap();
        assert_eq!(pregaps, [0, 150, 0]);
    }

    #[test]
    fn few_reads_without_pregaps() {
        let toc = Toc::parse_musicbrainz("1 3 30150 150 10150 20150").unwrap();
        let mut reads = 0;
        let mut counting = |lba| {
            reads += 1;
            Ok(Some(QPosition { track: if lba < 10_000 { 1 } else { 2 }, index: 1 }))
        };
        detect_pregaps(&toc, &mut counting).unwrap();
        assert_eq!(reads, 2);
    }
}
