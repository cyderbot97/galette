//! WAV files: a 44-byte RIFF header followed by the raw CD audio.

use std::io::{self, Write};

const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u16 = 2;
const BITS: u16 = 16;

pub fn write(out: &mut impl Write, audio: &[u8]) -> io::Result<()> {
    let data_len = u32::try_from(audio.len()).map_err(|_| io::Error::other("track too long for WAV"))?;
    let block_align = CHANNELS * BITS / 8;

    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_len).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&CHANNELS.to_le_bytes())?;
    out.write_all(&SAMPLE_RATE.to_le_bytes())?;
    out.write_all(&(SAMPLE_RATE * block_align as u32).to_le_bytes())?;
    out.write_all(&block_align.to_le_bytes())?;
    out.write_all(&BITS.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    out.write_all(audio)
}

#[cfg(test)]
mod tests {
    #[test]
    fn header() {
        let mut out = Vec::new();
        super::write(&mut out, &[1, 2, 3, 4]).unwrap();
        assert_eq!(out.len(), 48);
        assert_eq!(&out[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()), 40);
        assert_eq!(u32::from_le_bytes(out[28..32].try_into().unwrap()), 176_400);
        assert_eq!(&out[44..], &[1, 2, 3, 4]);
    }
}
