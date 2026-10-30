//! CD-Text: titles and names stored on the disc itself, in the lead-in
//! (MMC READ TOC format 0101b, Red Book annex J). Used when MusicBrainz does
//! not know the disc.
//!
//! The drive returns 18-byte packs: a type (title, performer…), the track
//! of the first text starting in the pack, a sequence number, the block
//! (language) and character position, 12 bytes of text and a CRC. Texts are
//! NUL-terminated and run on from pack to pack; a TAB means "same as the
//! previous track".

use crate::metadata::{Release, TrackInfo};

const PACK_LEN: usize = 18;
const TITLE: u8 = 0x80;
const PERFORMER: u8 = 0x81;
const SONGWRITER: u8 = 0x82;
const COMPOSER: u8 = 0x83;
const SIZE_INFO: u8 = 0x8F;
/// Character codes in the size information pack.
const ISO_8859_1: u8 = 0x00;
const ASCII: u8 = 0x01;

/// The texts of one disc: index 0 is the album, index n is track n.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CdText {
    pub titles: Vec<Option<String>>,
    pub performers: Vec<Option<String>>,
    pub songwriters: Vec<Option<String>>,
    pub composers: Vec<Option<String>>,
}

impl CdText {
    /// Decodes the reply of READ TOC format 0101b (4-byte header, then
    /// packs). Only the first block (language) is read, and only single-byte
    /// character sets: `None` if nothing usable is left.
    pub fn parse(data: &[u8]) -> Option<CdText> {
        let len = (u16::from_be_bytes([*data.first()?, *data.get(1)?]) as usize + 2).min(data.len());
        let packs: Vec<&[u8]> = data
            .get(4..len)?
            .as_chunks::<PACK_LEN>()
            .0
            .iter()
            .map(|p| &p[..])
            .filter(|p| crc_ok(p))
            // Block 0, single-byte characters.
            .filter(|p| p[3] & 0x70 == 0 && p[3] & 0x80 == 0)
            .collect();

        // The size information says which character set block 0 uses.
        let charset = packs.iter().find(|p| p[0] == SIZE_INFO && p[1] == 0).map_or(ISO_8859_1, |p| p[4]);
        if charset != ISO_8859_1 && charset != ASCII {
            return None;
        }

        let text = CdText {
            titles: texts(&packs, TITLE),
            performers: texts(&packs, PERFORMER),
            songwriters: texts(&packs, SONGWRITER),
            composers: texts(&packs, COMPOSER),
        };
        (text.title(0).is_some() || text.performer(0).is_some() || text.titles.iter().skip(1).any(Option::is_some))
            .then_some(text)
    }

    pub fn title(&self, index: usize) -> Option<&str> {
        self.titles.get(index)?.as_deref()
    }

    pub fn performer(&self, index: usize) -> Option<&str> {
        self.performers.get(index)?.as_deref()
    }

    /// The disc as an editable release, with no MusicBrainz identifiers.
    /// `tracks` are the audio track numbers of the disc.
    pub fn release(&self, tracks: impl Iterator<Item = u8>) -> Release {
        let album_artist = self.performer(0).unwrap_or_default().to_owned();
        Release {
            id: String::new(),
            release_group_id: None,
            title: self.title(0).unwrap_or_default().to_owned(),
            artist: album_artist.clone(),
            artist_ids: Vec::new(),
            date: None,
            original_date: None,
            country: None,
            label: None,
            catalog_number: None,
            barcode: None,
            disc_number: 1,
            disc_count: 1,
            tracks: tracks
                .map(|n| TrackInfo {
                    id: String::new(),
                    recording_id: None,
                    number: n.into(),
                    title: self.title(n.into()).unwrap_or_default().to_owned(),
                    artist: self.performer(n.into()).map_or_else(|| album_artist.clone(), str::to_owned),
                    artist_ids: Vec::new(),
                    length_ms: None,
                })
                .collect(),
        }
    }
}

/// CRC-16/CCITT (polynomial 0x1021) of the first 16 bytes, stored inverted.
/// Some drives return zeros instead of the CRC: those packs are kept.
fn crc_ok(pack: &[u8]) -> bool {
    let stored = u16::from_be_bytes([pack[16], pack[17]]);
    if stored == 0 {
        return true;
    }
    let mut crc: u16 = 0;
    for &byte in &pack[..16] {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    !crc == stored
}

/// The texts of packs of type `kind`, by track (0 = album).
fn texts(packs: &[&[u8]], kind: u8) -> Vec<Option<String>> {
    let mut packs: Vec<&&[u8]> = packs.iter().filter(|p| p[0] == kind).collect();
    packs.sort_by_key(|p| p[2]); // sequence number
    let Some(first) = packs.first() else { return Vec::new() };
    let first_track = usize::from(first[1] & 0x7F);

    let bytes: Vec<u8> = packs.iter().flat_map(|p| p[4..16].iter().copied()).collect();
    let mut texts: Vec<Option<String>> = vec![None; first_track];
    let mut previous: Option<String> = None;
    // The last piece has no terminating NUL when it is padding.
    let pieces: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    for piece in &pieces[..pieces.len() - 1] {
        let text = if *piece == b"\t" {
            previous.clone()
        } else {
            // ISO 8859-1 is the first 256 code points of Unicode.
            let text: String = piece.iter().map(|&b| char::from(b)).collect();
            let text = text.trim().to_owned();
            (!text.is_empty()).then_some(text)
        };
        previous = text.clone();
        texts.push(text);
    }
    texts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pack with its CRC.
    fn pack(kind: u8, track: u8, seq: u8, text: &[u8]) -> Vec<u8> {
        let mut p = vec![kind, track, seq, 0];
        let mut data = text.to_vec();
        data.resize(12, 0);
        p.extend(data);
        let mut crc: u16 = 0;
        for &byte in &p {
            crc ^= u16::from(byte) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
            }
        }
        p.extend((!crc).to_be_bytes());
        p
    }

    fn reply(packs: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = packs.concat();
        let mut data = ((body.len() + 2) as u16).to_be_bytes().to_vec();
        data.extend([0, 0]);
        data.extend(body);
        data
    }

    #[test]
    fn titles_run_across_packs() {
        // "Album\0Été\0Second song\0" split over 12-byte packs, then performers
        // with a TAB meaning "same as before".
        let titles = b"Album\0\xC9t\xE9\0Second song\0";
        let performers = b"Band\0\t\0Duo\0";
        let data = reply(&[
            pack(TITLE, 0, 0, &titles[..12]),
            pack(TITLE, 1, 1, &titles[12..]),
            pack(PERFORMER, 0, 2, performers),
        ]);
        let text = CdText::parse(&data).unwrap();
        assert_eq!(text.title(0), Some("Album"));
        assert_eq!(text.title(1), Some("Été"));
        assert_eq!(text.title(2), Some("Second song"));
        assert_eq!(text.performer(1), Some("Band"));
        assert_eq!(text.performer(2), Some("Duo"));

        let release = text.release([1, 2].into_iter());
        assert_eq!((release.artist.as_str(), release.title.as_str()), ("Band", "Album"));
        assert_eq!(release.tracks[1].title, "Second song");
        assert_eq!(release.tracks[1].artist, "Duo");
    }

    #[test]
    fn rejects_corrupt_packs_and_empty_replies() {
        let mut bad = pack(TITLE, 0, 0, b"Album\0Song\0");
        bad[5] ^= 0xFF; // damaged after the CRC was computed
        assert_eq!(CdText::parse(&reply(&[bad])), None);
        assert_eq!(CdText::parse(&[0, 2, 0, 0]), None);
        assert_eq!(CdText::parse(&[]), None);
    }
}
