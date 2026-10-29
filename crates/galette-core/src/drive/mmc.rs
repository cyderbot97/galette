//! Building and decoding MMC commands (SCSI Multimedia Commands, as in the
//! T10 MMC-6 specification). Platform independent, hence testable without a drive.

use super::DriveInfo;
use crate::toc::{Toc, Track};
use crate::{Error, Result};

const INQUIRY: u8 = 0x12;
const READ_TOC: u8 = 0x43;
const READ_CD: u8 = 0xBE;

/// Track number of the lead-out in a TOC response.
const LEAD_OUT: u8 = 0xAA;
/// Bit of the CONTROL field marking a data track.
const CONTROL_DATA: u8 = 0x04;

/// Header + 99 tracks + lead-out, 8 bytes each.
pub const TOC_BUFFER_LEN: usize = 4 + 100 * 8;

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
        0,    // no sub-channel
        0,
    ]
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
    fn read_cd_encodes_address() {
        let cdb = read_cd_cdb(0x0001_0203, 27);
        assert_eq!(&cdb[2..9], &[0, 1, 2, 3, 0, 0, 27]);
    }
}
