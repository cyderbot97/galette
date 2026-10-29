//! Disc identifiers computed from the TOC, used to query online databases.

use std::fmt::Write;

use base64::Engine;
use sha1::{Digest, Sha1};

use crate::toc::{PREGAP, SECTORS_PER_SECOND, Toc};

/// MusicBrainz Disc ID, as specified at <https://musicbrainz.org/doc/Disc_ID_Calculation>.
///
/// SHA-1 of the hex-encoded TOC, in base64 with `.`, `_` and `-` replacing
/// `+`, `/` and `=` so that it fits in a URL.
pub fn musicbrainz(toc: &Toc) -> String {
    let audio: Vec<_> = toc.audio_tracks().collect();
    let first = audio.first().unwrap().number;
    let last = audio.last().unwrap().number;

    let mut offsets = [0u32; 100];
    offsets[0] = toc.audio_lead_out() + PREGAP;
    for track in &audio {
        offsets[track.number as usize] = track.start + PREGAP;
    }

    let mut text = format!("{first:02X}{last:02X}");
    for offset in offsets {
        write!(text, "{offset:08X}").unwrap();
    }

    base64::engine::general_purpose::STANDARD
        .encode(Sha1::digest(text.as_bytes()))
        .replace('+', ".")
        .replace('/', "_")
        .replace('=', "-")
}

/// FreeDB/CDDB disc ID. Still needed because AccurateRip uses it as a key.
pub fn freedb(toc: &Toc) -> u32 {
    fn digit_sum(mut n: u32) -> u32 {
        let mut sum = 0;
        while n > 0 {
            sum += n % 10;
            n /= 10;
        }
        sum
    }

    let seconds = |lba: u32| (lba + PREGAP) / SECTORS_PER_SECOND;
    let checksum: u32 = toc.tracks.iter().map(|t| digit_sum(seconds(t.start))).sum();
    let length = seconds(toc.lead_out) - seconds(toc.tracks[0].start);
    (checksum % 0xFF) << 24 | length << 8 | toc.tracks.len() as u32
}

/// Key of a disc in the AccurateRip database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccurateRipId {
    pub track_count: u8,
    pub id1: u32,
    pub id2: u32,
    pub freedb: u32,
}

impl AccurateRipId {
    // TODO: check Enhanced CDs against a real disc; we use the audio session only.
    pub fn new(toc: &Toc) -> Self {
        let mut id1 = 0u32;
        let mut id2 = 0u32;
        let mut count = 0u32;
        for (i, track) in toc.audio_tracks().enumerate() {
            let position = i as u32 + 1;
            id1 = id1.wrapping_add(track.start);
            id2 = id2.wrapping_add(track.start.max(1).wrapping_mul(position));
            count = position;
        }
        let lead_out = toc.audio_lead_out();
        id1 = id1.wrapping_add(lead_out);
        id2 = id2.wrapping_add(lead_out.max(1).wrapping_mul(count + 1));

        Self { track_count: count as u8, id1, id2, freedb: freedb(toc) }
    }

    /// URL of the binary file holding the checksums submitted for this disc.
    pub fn url(&self) -> String {
        format!(
            "http://www.accuraterip.com/accuraterip/{:x}/{:x}/{:x}/dBAR-{:03}-{:08x}-{:08x}-{:08x}.bin",
            self.id1 & 0xF,
            self.id1 >> 4 & 0xF,
            self.id1 >> 8 & 0xF,
            self.track_count,
            self.id1,
            self.id2,
            self.freedb
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Radiohead – OK Computer (CD, release 6a09041b-0f79-3278-88d9-0552dd28d37f),
    /// TOC and Disc ID as listed by MusicBrainz.
    const OK_COMPUTER: &str = "1 12 242387 150 21636 50551 70783 90795 113395 133171 \
                               142126 159578 181123 198419 218045";

    #[test]
    fn musicbrainz_disc_id() {
        let toc = Toc::parse_musicbrainz(OK_COMPUTER).unwrap();
        assert_eq!(musicbrainz(&toc), ".JJK27a.2SuhFenNvxzYSrWdrPI-");
    }

    #[test]
    fn freedb_id_layout() {
        let toc = Toc::parse_musicbrainz(OK_COMPUTER).unwrap();
        let id = freedb(&toc);
        assert_eq!(id & 0xFF, 12);
        assert_eq!(id >> 8 & 0xFFFF, 242387 / 75 - 150 / 75);
    }

    /// Another OK Computer pressing (Disc ID rdosqrxASN.ZqF.d.5pPhHtgVyA-),
    /// whose entry exists in the AccurateRip database.
    #[test]
    fn accuraterip_id() {
        let toc = Toc::parse_musicbrainz(
            "1 12 240687 150 21480 50242 70320 90180 112625 132247 141047 158345 179735 196875 216347",
        )
        .unwrap();
        assert_eq!(
            AccurateRipId::new(&toc).url(),
            "http://www.accuraterip.com/accuraterip/a/8/a/dBAR-012-00188a8a-00e12645-a60c870c.bin"
        );
    }
}
