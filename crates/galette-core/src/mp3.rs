//! MP3 files, encoded by LAME and tagged with ID3v2.4.
//!
//! LAME puts a "LAME tag" in the first frame: the encoder delay and padding
//! that players need to play consecutive tracks without a gap between them.

use mp3lame_encoder::{Bitrate, Builder, FlushGap, InterleavedPcm, Quality as Lame, VbrMode};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// Variable bitrate, best quality (LAME -V0, about 245 kbit/s).
    #[default]
    V0,
    /// Variable bitrate, transparent for most listeners (-V2, about 190 kbit/s).
    V2,
    /// Constant 320 kbit/s.
    Cbr320,
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Quality::V0 => "V0",
            Quality::V2 => "V2",
            Quality::Cbr320 => "CBR 320 kbit/s",
        }
    }
}

/// Encodes `audio` (CD audio, little endian) into an MP3 file starting with `tag`.
pub fn encode(audio: &[u8], quality: Quality, tag: &id3::Tag) -> Result<Vec<u8>> {
    let failed = |why: String| Error::Encode(format!("MP3 encoding failed: {why}"));
    let samples: Vec<i16> = audio.as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s)).collect();

    let mut builder = Builder::new().ok_or_else(|| failed("cannot start LAME".into()))?;
    builder.set_num_channels(2).map_err(|e| failed(e.to_string()))?;
    builder.set_sample_rate(44_100).map_err(|e| failed(e.to_string()))?;
    builder.set_quality(Lame::NearBest).map_err(|e| failed(e.to_string()))?;
    match quality {
        Quality::V0 | Quality::V2 => {
            builder.set_vbr_mode(VbrMode::Mtrh).map_err(|e| failed(e.to_string()))?;
            let level = if quality == Quality::V0 { Lame::Best } else { Lame::NearBest };
            builder.set_vbr_quality(level).map_err(|e| failed(e.to_string()))?;
        }
        Quality::Cbr320 => {
            builder.set_vbr_mode(VbrMode::Off).map_err(|e| failed(e.to_string()))?;
            builder.set_brate(Bitrate::Kbps320).map_err(|e| failed(e.to_string()))?;
        }
    }
    builder.set_to_write_vbr_tag(true).map_err(|e| failed(e.to_string()))?;
    let mut encoder = builder.build().map_err(|e| failed(e.to_string()))?;

    let mut mp3 = Vec::with_capacity(mp3lame_encoder::max_required_buffer_size(samples.len() / 2) + 7200);
    encoder.encode_to_vec(InterleavedPcm(&samples), &mut mp3).map_err(|e| failed(e.to_string()))?;
    encoder.flush_to_vec::<FlushGap>(&mut mp3).map_err(|e| failed(e.to_string()))?;

    // LAME left room for its tag in the first frame of the stream.
    let mut lame_tag = Vec::with_capacity(encoder.lame_tag_size());
    if encoder.lame_tag_encode_to_vec(&mut lame_tag).is_some() {
        mp3.get_mut(..lame_tag.len()).ok_or_else(|| failed("no room for the LAME tag".into()))?.copy_from_slice(&lame_tag);
    }

    let mut file = Vec::with_capacity(mp3.len() + 1024);
    tag.write_to(&mut file, id3::Version::Id3v24).map_err(|e| failed(e.to_string()))?;
    file.extend_from_slice(&mp3);
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use id3::TagLike;

    #[test]
    fn tagged_gapless_file() {
        // One second of a 440 Hz tone.
        let audio: Vec<u8> = (0..44_100)
            .flat_map(|i| {
                let s = ((i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 10_000.0) as i16;
                [s.to_le_bytes(), s.to_le_bytes()].concat()
            })
            .collect();
        let mut tag = id3::Tag::new();
        tag.set_title("Été");
        let file = encode(&audio, Quality::V2, &tag).unwrap();

        let read = id3::Tag::read_from2(std::io::Cursor::new(&file)).unwrap();
        assert_eq!(read.title(), Some("Été"));
        // The first frame after the tag holds the LAME tag ("Info" or "Xing").
        let tag_len = 10 + file[6..10].iter().fold(0usize, |n, &b| n << 7 | b as usize);
        let frames = &file[tag_len..];
        assert_eq!((frames[0], frames[1] & 0xE0), (0xFF, 0xE0), "frame sync");
        assert!(frames.windows(4).take(200).any(|w| w == b"Info" || w == b"Xing"));
        assert!(frames.windows(4).take(400).any(|w| w == b"LAME"));
    }
}
