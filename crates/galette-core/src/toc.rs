//! Table of contents of an audio CD.
//!
//! All positions are LBAs (logical block addresses) in sectors of 1/75 s.
//! LBA 0 is the start of track 1's audio; the 150-sector (2 s) pregap before
//! it is *not* included. Some formats (MusicBrainz, FreeDB) count it, they add
//! [`PREGAP`] themselves.

use serde::Serialize;

use crate::{Error, Result};

/// Sectors per second of audio.
pub const SECTORS_PER_SECOND: u32 = 75;
/// Mandatory 2-second pregap before track 1 (MSF 00:02:00).
pub const PREGAP: u32 = 150;
/// Bytes of audio in one sector: 588 stereo samples of 16 bits.
pub const SECTOR_BYTES: usize = 2352;
/// Gap between the audio session and the data session on an Enhanced CD.
pub const SESSION_GAP: u32 = 11_400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Track {
    pub number: u8,
    /// First sector of the track (LBA).
    pub start: u32,
    pub is_audio: bool,
    /// The audio was recorded with pre-emphasis (a treble boost that players
    /// are expected to undo), as some early CDs were.
    pub pre_emphasis: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Toc {
    pub tracks: Vec<Track>,
    /// First sector after the last track (LBA).
    pub lead_out: u32,
}

impl Toc {
    pub fn new(tracks: Vec<Track>, lead_out: u32) -> Result<Self> {
        if tracks.is_empty() {
            return Err(Error::InvalidToc("no tracks".into()));
        }
        if !tracks.iter().any(|t| t.is_audio) {
            return Err(Error::InvalidToc("no audio track on this disc".into()));
        }
        for pair in tracks.windows(2) {
            if pair[1].number != pair[0].number + 1 || pair[1].start <= pair[0].start {
                return Err(Error::InvalidToc(format!(
                    "track {} does not follow track {}",
                    pair[1].number, pair[0].number
                )));
            }
        }
        let last = tracks.last().unwrap();
        if lead_out <= last.start {
            return Err(Error::InvalidToc("lead-out before last track".into()));
        }
        Ok(Self { tracks, lead_out })
    }

    /// Parses a TOC in MusicBrainz notation: `first last lead-out offset1 offset2 ...`,
    /// offsets including the 150-sector pregap. All tracks are assumed to be audio.
    ///
    /// Handy for testing without a drive, e.g. `1 2 30000 150 15000`.
    pub fn parse_musicbrainz(s: &str) -> Result<Self> {
        let nums: Vec<u32> = s
            .split_whitespace()
            .map(|n| n.parse().map_err(|_| Error::InvalidToc(format!("not a number: {n}"))))
            .collect::<Result<_>>()?;
        let [first, last, lead_out, offsets @ ..] = nums.as_slice() else {
            return Err(Error::InvalidToc("expected: first last lead-out offsets...".into()));
        };
        if *last < *first || offsets.len() != (last - first + 1) as usize {
            return Err(Error::InvalidToc("track count does not match offsets".into()));
        }
        let unpregap = |o: u32| {
            o.checked_sub(PREGAP)
                .ok_or_else(|| Error::InvalidToc(format!("offset {o} is inside the pregap")))
        };
        let tracks = offsets
            .iter()
            .zip(*first..)
            .map(|(&o, n)| {
                Ok(Track { number: n as u8, start: unpregap(o)?, is_audio: true, pre_emphasis: false })
            })
            .collect::<Result<_>>()?;
        Self::new(tracks, unpregap(*lead_out)?)
    }

    pub fn audio_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.is_audio)
    }

    /// End of the audio part of the disc. On an Enhanced CD (audio session
    /// followed by a data session) this is the data track minus the session gap.
    pub fn audio_lead_out(&self) -> u32 {
        let last_audio = self.audio_tracks().last().unwrap().number;
        match self.tracks.iter().find(|t| t.number > last_audio && !t.is_audio) {
            Some(data) => data.start.saturating_sub(SESSION_GAP),
            None => self.lead_out,
        }
    }

    /// Length in sectors of the track at `index` in [`Toc::tracks`].
    pub fn track_sectors(&self, index: usize) -> u32 {
        let track = &self.tracks[index];
        let end = match self.tracks.get(index + 1) {
            Some(next) if track.is_audio && !next.is_audio => self.audio_lead_out(),
            Some(next) => next.start,
            None => self.lead_out,
        };
        end - track.start
    }
}

/// Formats a sector count as `mm:ss.ff` (minutes, seconds, frames).
pub fn format_msf(sectors: u32) -> String {
    let frames = sectors % SECTORS_PER_SECOND;
    let seconds = sectors / SECTORS_PER_SECOND;
    format!("{:02}:{:02}.{:02}", seconds / 60, seconds % 60, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_musicbrainz_notation() {
        let toc = Toc::parse_musicbrainz("1 3 30150 150 10150 20150").unwrap();
        assert_eq!(toc.tracks.len(), 3);
        assert_eq!(toc.tracks[1], Track { number: 2, start: 10_000, is_audio: true, pre_emphasis: false });
        assert_eq!(toc.lead_out, 30_000);
        assert_eq!(toc.track_sectors(2), 10_000);
    }

    #[test]
    fn rejects_inconsistent_toc() {
        assert!(Toc::parse_musicbrainz("1 3 30150 150 10150").is_err());
        assert!(Toc::parse_musicbrainz("1 2 30150 150 100").is_err());
    }

    #[test]
    fn enhanced_cd_audio_lead_out() {
        let tracks = vec![
            Track { number: 1, start: 0, is_audio: true, pre_emphasis: false },
            Track { number: 2, start: 20_000, is_audio: true, pre_emphasis: false },
            Track { number: 3, start: 51_400, is_audio: false, pre_emphasis: false },
        ];
        let toc = Toc::new(tracks, 60_000).unwrap();
        assert_eq!(toc.audio_lead_out(), 40_000);
        assert_eq!(toc.track_sectors(1), 20_000);
    }

    #[test]
    fn msf() {
        assert_eq!(format_msf(75 * 61 + 3), "01:01.03");
    }
}
