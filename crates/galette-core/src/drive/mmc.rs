//! Building and decoding MMC commands (SCSI Multimedia Commands, as in the
//! T10 MMC-6 specification). Platform independent, hence testable without a drive.

use super::DriveInfo;
use crate::toc::{Toc, Track};
use crate::{Error, Result};

const INQUIRY: u8 = 0x12;
const START_STOP_UNIT: u8 = 0x1B;
const PREVENT_ALLOW_MEDIUM_REMOVAL: u8 = 0x1E;
const MODE_SENSE_10: u8 = 0x5A;
const READ_SUB_CHANNEL: u8 = 0x42;
const READ_TOC: u8 = 0x43;
const READ_CD: u8 = 0xBE;

/// Track number of the lead-out in a TOC response.
const LEAD_OUT: u8 = 0xAA;
/// Bit of the CONTROL field marking a data track.
const CONTROL_DATA: u8 = 0x04;

/// Header + 99 tracks + lead-out, 8 bytes each.
pub const TOC_BUFFER_LEN: usize = 4 + 100 * 8;

/// PREVENT ALLOW MEDIUM REMOVAL, allowing it.
pub fn allow_removal_cdb() -> [u8; 6] {
    [PREVENT_ALLOW_MEDIUM_REMOVAL, 0, 0, 0, 0, 0]
}

/// START STOP UNIT with LoEj set and Start cleared: opens the tray.
pub fn eject_cdb() -> [u8; 6] {
    [START_STOP_UNIT, 0, 0, 0, 0x02, 0]
}

/// Standard INQUIRY data up to the product revision.
pub const INQUIRY_LEN: usize = 36;

pub fn inquiry_cdb() -> [u8; 6] {
    [INQUIRY, 0, 0, 0, INQUIRY_LEN as u8, 0]
}

/// Vendor, product and revision, as space-padded ASCII in the INQUIRY data.
pub fn parse_inquiry(data: &[u8]) -> Result<DriveInfo> {
    if data.len() < INQUIRY_LEN {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "INQUIRY response too short",
        )));
    }
    let text = |range: std::ops::Range<usize>| String::from_utf8_lossy(&data[range]).trim().to_string();
    Ok(DriveInfo { vendor: text(8..16), product: text(16..32), revision: text(32..36) })
}

/// READ TOC/PMA/ATIP, format 0000b (formatted TOC), addresses as LBA.
pub fn read_toc_cdb(alloc_len: usize) -> [u8; 10] {
    let len = (alloc_len as u16).to_be_bytes();
    [READ_TOC, 0, 0, 0, 0, 0, 0, len[0], len[1], 0]
}

/// READ CD of `count` CD-DA sectors from `lba`, user data only (2352 bytes each).
pub fn read_cd_cdb(lba: u32, count: u32) -> [u8; 12] {
    read_cd(lba, count, 0)
}

/// Bytes of C2 error information per sector: one bit per byte of audio
/// (294 bytes), the OR of them all and a pad byte.
///
/// The format without the last two bytes makes sectors of 2646 bytes, which
/// some USB bridges (Initio) cannot transfer several at a time: they stop
/// mid-transfer and stop responding. 2648 bytes is a multiple of 8.
pub const C2_LEN: usize = 296;

/// READ CD of `count` CD-DA sectors, each followed by its C2 error information.
pub fn read_cd_c2_cdb(lba: u32, count: u32) -> [u8; 12] {
    let mut cdb = read_cd(lba, count, 0);
    cdb[9] |= 0b10 << 1; // C2 and block error bits
    cdb
}

/// What a drive can do, from the "CD/DVD capabilities" mode page (2Ah).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// The drive can report the bytes it could not correct (C2 errors).
    pub c2_pointers: bool,
    /// The drive can restart reading audio exactly at the requested sector.
    pub accurate_stream: bool,
    /// Size of the read cache in KiB.
    pub buffer_kib: u16,
}

pub const CAPABILITIES_LEN: usize = 64;
const CAPABILITIES_PAGE: u8 = 0x2A;

/// MODE SENSE (10) of the capabilities page, without block descriptors.
pub fn mode_sense_capabilities_cdb() -> [u8; 10] {
    let len = (CAPABILITIES_LEN as u16).to_be_bytes();
    [MODE_SENSE_10, 0x08, CAPABILITIES_PAGE, 0, 0, 0, 0, len[0], len[1], 0]
}

pub fn parse_capabilities(data: &[u8]) -> Option<Capabilities> {
    let descriptors = u16::from_be_bytes([*data.get(6)?, *data.get(7)?]) as usize;
    let page = data.get(8 + descriptors..)?;
    if page.len() < 14 || page[0] & 0x3F != CAPABILITIES_PAGE {
        return None;
    }
    Some(Capabilities {
        c2_pointers: page[5] & 0x10 != 0,
        accurate_stream: page[5] & 0x02 != 0,
        buffer_kib: u16::from_be_bytes([page[12], page[13]]),
    })
}

/// Bytes of formatted Q sub-channel appended to each sector by [`read_cd_q_cdb`].
pub const Q_LEN: usize = 16;

/// READ CD of one sector followed by its formatted Q sub-channel.
pub fn read_cd_q_cdb(lba: u32) -> [u8; 12] {
    read_cd(lba, 1, 0b010)
}

fn read_cd(lba: u32, count: u32, subchannel: u8) -> [u8; 12] {
    let lba = lba.to_be_bytes();
    let count = count.to_be_bytes();
    [
        READ_CD,
        0b001 << 2, // expected sector type: CD-DA
        lba[0],
        lba[1],
        lba[2],
        lba[3],
        count[1],
        count[2],
        count[3],
        0x10, // user data, no headers
        subchannel,
        0,
    ]
}

/// Position given by a Q sub-channel frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QPosition {
    pub track: u8,
    /// 0 in a track's pregap, 1 or more in the track itself.
    pub index: u8,
}

/// Decodes formatted Q: CONTROL/ADR, track and index in BCD, then times.
/// Only ADR 1 frames carry a position; about 1 % of the frames carry the MCN
/// or ISRC instead, and give `None`.
pub fn parse_q(q: &[u8]) -> Option<QPosition> {
    if q.len() < 3 || q[0] & 0x0F != 1 {
        return None;
    }
    Some(QPosition { track: from_bcd(q[1])?, index: from_bcd(q[2])? })
}

fn from_bcd(byte: u8) -> Option<u8> {
    let (high, low) = (byte >> 4, byte & 0x0F);
    (high < 10 && low < 10).then_some(high * 10 + low)
}

pub const SUB_CHANNEL_LEN: usize = 24;
const MCN_FORMAT: u8 = 0x02;
const ISRC_FORMAT: u8 = 0x03;

/// READ SUB-CHANNEL asking for the Media Catalog Number (the disc's barcode).
pub fn read_mcn_cdb() -> [u8; 10] {
    read_sub_channel(MCN_FORMAT, 0)
}

/// READ SUB-CHANNEL asking for the ISRC of `track`.
pub fn read_isrc_cdb(track: u8) -> [u8; 10] {
    read_sub_channel(ISRC_FORMAT, track)
}

fn read_sub_channel(format: u8, track: u8) -> [u8; 10] {
    [READ_SUB_CHANNEL, 0, 0x40, format, 0, 0, track, 0, SUB_CHANNEL_LEN as u8, 0]
}

/// The MCN: 13 digits at byte 9, valid if bit 7 of byte 8 (MCVAL) is set.
pub fn parse_mcn(data: &[u8]) -> Option<String> {
    sub_channel_code(data, 13).filter(|c| c.bytes().all(|b| b.is_ascii_digit()))
}

/// The ISRC: 12 characters at byte 9, valid if bit 7 of byte 8 (TCVAL) is set.
pub fn parse_isrc(data: &[u8]) -> Option<String> {
    sub_channel_code(data, 12).filter(|c| c.bytes().all(|b| b.is_ascii_alphanumeric()))
}

fn sub_channel_code(data: &[u8], len: usize) -> Option<String> {
    if data.len() < 9 + len || data[8] & 0x80 == 0 {
        return None;
    }
    let code = std::str::from_utf8(&data[9..9 + len]).ok()?;
    // Some discs flag an all-zero code as valid.
    (!code.bytes().all(|b| b == b'0')).then(|| code.to_string())
}

pub fn parse_toc(data: &[u8]) -> Result<Toc> {
    if data.len() < 4 {
        return Err(Error::InvalidToc("response too short".into()));
    }
    let len = (u16::from_be_bytes([data[0], data[1]]) as usize + 2).min(data.len());

    let mut tracks = Vec::new();
    let mut lead_out = None;
    for desc in data[4..len].as_chunks::<8>().0 {
        let control = desc[1] & 0x0F;
        let number = desc[2];
        let lba = u32::from_be_bytes([desc[4], desc[5], desc[6], desc[7]]);
        if number == LEAD_OUT {
            lead_out = Some(lba);
        } else {
            tracks.push(Track { number, start: lba, is_audio: control & CONTROL_DATA == 0 });
        }
    }

    let lead_out = lead_out.ok_or_else(|| Error::InvalidToc("no lead-out".into()))?;
    Toc::new(tracks, lead_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(control: u8, number: u8, lba: u32) -> Vec<u8> {
        let mut d = vec![0, 0x10 | control, number, 0];
        d.extend(lba.to_be_bytes());
        d
    }

    #[test]
    fn parses_enhanced_cd_toc() {
        let mut body = vec![1, 3];
        body.extend(descriptor(0x00, 1, 0));
        body.extend(descriptor(0x00, 2, 20_000));
        body.extend(descriptor(0x04, 3, 51_400));
        body.extend(descriptor(0x04, LEAD_OUT, 60_000));
        let mut data = ((body.len()) as u16).to_be_bytes().to_vec();
        data.extend(body);

        let toc = parse_toc(&data).unwrap();
        assert_eq!(toc.lead_out, 60_000);
        assert_eq!(toc.tracks.len(), 3);
        assert!(toc.tracks[1].is_audio);
        assert!(!toc.tracks[2].is_audio);
    }

    #[test]
    fn parses_inquiry() {
        let mut data = vec![0x05, 0, 0, 0, 31, 0, 0, 0];
        data.extend(b"ASUS    SDRW-08U9M-U    A112");
        let info = parse_inquiry(&data).unwrap();
        assert_eq!(info.vendor, "ASUS");
        assert_eq!(info.product, "SDRW-08U9M-U");
        assert_eq!(info.revision, "A112");
    }

    #[test]
    fn parses_q_sub_channel() {
        // Track 2, index 1, as read on a real disc.
        let q = [0x01, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x05, 0x25, 0x60, 0x6f, 0x4b, 0, 0, 0, 0x80];
        assert_eq!(parse_q(&q), Some(QPosition { track: 2, index: 1 }));
        // Pregap of track 10.
        assert_eq!(parse_q(&[0x01, 0x10, 0x00]), Some(QPosition { track: 10, index: 0 }));
        // ADR 2 (MCN) frame.
        assert_eq!(parse_q(&[0x02, 0x00, 0x93]), None);
    }

    #[test]
    fn parses_mcn_and_isrc() {
        let mcn = hex("001500140200000080303039333632343934333039390008");
        assert_eq!(parse_mcn(&mcn).as_deref(), Some("0093624943099"));
        let isrc = hex("001500140330010080555357423131333032343930000800");
        assert_eq!(parse_isrc(&isrc).as_deref(), Some("USWB11302490"));

        let mut invalid = isrc.clone();
        invalid[8] = 0;
        assert_eq!(parse_isrc(&invalid), None);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn parses_capabilities() {
        // As returned by an ASUS SDRW-08U9M-U.
        let data = hex("004a0200000000002a423f37f1772b23108a0100030006e40010108a108a0001");
        let caps = parse_capabilities(&data).unwrap();
        assert_eq!(caps, Capabilities { c2_pointers: true, accurate_stream: true, buffer_kib: 768 });
    }

    #[test]
    fn read_cd_encodes_address() {
        let cdb = read_cd_cdb(0x0001_0203, 27);
        assert_eq!(&cdb[2..9], &[0, 1, 2, 3, 0, 0, 27]);
        assert_eq!(read_cd_c2_cdb(0, 1)[9], 0x14);
    }
}
