//! Apple Lossless (ALAC) in .m4a files: the lossless codec that Apple Music
//! and iOS play natively. Like FLAC files, every file is decoded back and
//! compared with the audio before it is kept.

use std::io::Cursor;

use alac_encoder::{AlacEncoder, FormatDescription};

use crate::{Error, Result, m4a};

const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: usize = 2;
const FRAMES_PER_PACKET: u32 = 4096;
/// Where the average bitrate sits in the ALAC configuration (ALACSpecificConfig).
const AVERAGE_BITRATE: std::ops::Range<usize> = 16..20;

/// Encodes `audio` (CD audio, little endian) into an .m4a file with `tag`.
pub fn encode(audio: &[u8], tag: &mp4ameta::Tag) -> Result<Vec<u8>> {
    let input = FormatDescription::pcm::<i16>(SAMPLE_RATE as f64, CHANNELS as u32);
    let output = FormatDescription::alac(SAMPLE_RATE as f64, FRAMES_PER_PACKET, CHANNELS as u32);
    let mut encoder = AlacEncoder::new(&output);
    let mut buffer = vec![0u8; output.max_packet_size()];
    let packets: Vec<Vec<u8>> = audio
        .chunks(FRAMES_PER_PACKET as usize * CHANNELS * 2)
        .map(|chunk| {
            let len = encoder.encode(&input, chunk, &mut buffer);
            buffer[..len].to_vec()
        })
        .collect();

    let frames = audio.len() / (CHANNELS * 2);
    // The encoder leaves the average bitrate at zero.
    let mut config = encoder.magic_cookie();
    let bytes: usize = packets.iter().map(Vec::len).sum();
    let bitrate = (bytes as u64 * 8 * SAMPLE_RATE as u64 / frames.max(1) as u64) as u32;
    config[AVERAGE_BITRATE].copy_from_slice(&bitrate.to_be_bytes());
    verify(&config, &packets, audio)?;

    let sample_entry = sample_entry(&config);
    let file = m4a::write(&m4a::Track {
        sample_entry: &sample_entry,
        sample_rate: SAMPLE_RATE,
        packets: &packets,
        frames_per_packet: FRAMES_PER_PACKET,
        frames: frames as u32,
    });
    let mut file = Cursor::new(file);
    tag.write_to(&mut file).map_err(|e| Error::Encode(format!("cannot write the MP4 tags: {e}")))?;
    Ok(file.into_inner())
}

/// The `alac` sample entry: an audio sample description followed by the
/// codec configuration in a box of the same name.
fn sample_entry(config: &[u8]) -> Vec<u8> {
    let mut entry = vec![0u8; 6]; // reserved
    entry.extend(1u16.to_be_bytes()); // data reference index
    entry.extend([0u8; 8]); // version, revision, vendor
    entry.extend((CHANNELS as u16).to_be_bytes());
    entry.extend(16u16.to_be_bytes()); // bits per sample
    entry.extend([0u8; 4]);
    entry.extend((SAMPLE_RATE << 16).to_be_bytes()); // 16.16 fixed point
    entry.extend(m4a::boxed(b"alac", &[&[0u8; 4][..], config].concat()));
    m4a::boxed(b"alac", &entry)
}

/// Decodes the packets and checks that they give back exactly `audio`.
fn verify(config: &[u8], packets: &[Vec<u8>], audio: &[u8]) -> Result<()> {
    let mismatch = |why: String| Error::Encode(format!("ALAC verification failed: {why}"));
    let info = alac::StreamInfo::from_cookie(config).map_err(|e| mismatch(e.to_string()))?;
    let mut decoder = alac::Decoder::new(info);
    let mut out = vec![0i16; FRAMES_PER_PACKET as usize * CHANNELS];
    let mut expected = audio.as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s));
    for (i, packet) in packets.iter().enumerate() {
        let decoded = decoder.decode_packet(packet, &mut out).map_err(|e| mismatch(e.to_string()))?;
        if !decoded.iter().all(|&s| expected.next() == Some(s)) {
            return Err(mismatch(format!("packet {i} differs")));
        }
    }
    if expected.next().is_some() {
        return Err(mismatch("wrong length".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_tags() {
        // 1.5 packets of a stereo ramp, with both extremes of the 16-bit range.
        let mut audio = Vec::new();
        for i in 0..6144i32 {
            let left = ((i * 37) % 65_536 - 32_768) as i16;
            let right = if i % 2 == 0 { i16::MIN } else { i16::MAX };
            audio.extend(left.to_le_bytes());
            audio.extend(right.to_le_bytes());
        }
        let mut tag = mp4ameta::Tag::default();
        tag.set_title("Été");
        tag.set_track(3, 12);
        let file = encode(&audio, &tag).unwrap();

        let read = mp4ameta::Tag::read_from(&mut Cursor::new(&file)).unwrap();
        assert_eq!(read.title(), Some("Été"));
        assert_eq!(read.track(), (Some(3), Some(12)));
        assert_eq!(read.duration(), std::time::Duration::from_nanos(6144 * 1_000_000_000 / 44_100));
        assert!(file.windows(4).any(|w| w == b"alac"));
    }
}
