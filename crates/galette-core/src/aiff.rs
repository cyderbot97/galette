//! AIFF files: Apple's uncompressed format. The samples are stored big endian
//! and, as in MP3 files, the tags go in an ID3 chunk.

use std::io::{self, Write};

const CHANNELS: u16 = 2;
const BITS: u16 = 16;
/// 44 100 as an 80-bit IEEE 754 extended float, as the COMM chunk wants it.
const SAMPLE_RATE: [u8; 10] = [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0];

/// Writes `audio` (CD audio, little endian) and an optional ID3 tag.
pub fn write(out: &mut impl Write, audio: &[u8], id3: Option<&[u8]>) -> io::Result<()> {
    let too_long = || io::Error::other("track too long for AIFF");
    let data_len = u32::try_from(audio.len()).map_err(|_| too_long())?;
    let id3 = id3.unwrap_or_default();
    // Chunks are padded to an even length.
    let id3_len = id3.len() as u32;
    let id3_chunk = if id3.is_empty() { 0 } else { 8 + id3_len + id3_len % 2 };
    let form_len = (4 + 8 + 18 + 8 + 8u32).checked_add(data_len).and_then(|n| n.checked_add(id3_chunk));

    out.write_all(b"FORM")?;
    out.write_all(&form_len.ok_or_else(too_long)?.to_be_bytes())?;
    out.write_all(b"AIFF")?;

    out.write_all(b"COMM")?;
    out.write_all(&18u32.to_be_bytes())?;
    out.write_all(&CHANNELS.to_be_bytes())?;
    out.write_all(&(data_len / (CHANNELS * BITS / 8) as u32).to_be_bytes())?;
    out.write_all(&BITS.to_be_bytes())?;
    out.write_all(&SAMPLE_RATE)?;

    out.write_all(b"SSND")?;
    out.write_all(&(8 + data_len).to_be_bytes())?;
    out.write_all(&[0; 8])?; // offset and block size, unused
    let swapped: Vec<u8> = audio.as_chunks::<2>().0.iter().flat_map(|&[lo, hi]| [hi, lo]).collect();
    out.write_all(&swapped)?;

    if !id3.is_empty() {
        out.write_all(b"ID3 ")?;
        out.write_all(&id3_len.to_be_bytes())?;
        out.write_all(id3)?;
        if id3_len % 2 == 1 {
            out.write_all(&[0])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let audio = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let mut file = Vec::new();
        write(&mut file, &audio, Some(b"ID3")).unwrap();

        assert_eq!(&file[..4], b"FORM");
        assert_eq!(u32::from_be_bytes(file[4..8].try_into().unwrap()) as usize, file.len() - 8);
        assert_eq!(&file[8..16], b"AIFFCOMM");
        // Two stereo frames.
        assert_eq!(u32::from_be_bytes(file[22..26].try_into().unwrap()), 2);
        assert_eq!(&file[38..42], b"SSND");
        assert_eq!(&file[54..62], [0x02, 0x01, 0x04, 0x03, 0x06, 0x05, 0x08, 0x07]);
        assert_eq!(&file[62..70], b"ID3 \0\0\0\x03");
        assert_eq!(file.len(), 74);
    }
}
