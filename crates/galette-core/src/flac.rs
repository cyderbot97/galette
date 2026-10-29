//! FLAC encoding, with tags and cover art.
//!
//! Every file is decoded again after encoding and compared with the ripped
//! audio, so an encoder bug can never silently damage a rip.

use std::io::{Cursor, Read};

use flacenc::component::BitRepr;
use flacenc::error::Verify;
use metaflac::block::{Block, PictureType};

use crate::metadata::coverart::CoverArt;
use crate::{Error, Result};

const CHANNELS: usize = 2;
const BITS_PER_SAMPLE: usize = 16;
const SAMPLE_RATE: usize = 44_100;
/// Free space left after the metadata, so that tags can be edited later
/// without rewriting the whole file.
const PADDING: u32 = 8192;

/// Encodes CD audio (16-bit stereo, little endian) to a complete FLAC file.
pub fn encode(audio: &[u8], tags: &[(&str, String)], cover: Option<&CoverArt>) -> Result<Vec<u8>> {
    let samples: Vec<i32> = audio.as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s) as i32).collect();

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| Error::Encode(format!("{e:?}")))?;
    let source = flacenc::source::MemSource::from_samples(&samples, CHANNELS, BITS_PER_SAMPLE, SAMPLE_RATE);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| Error::Encode(format!("{e:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| Error::Encode(format!("{e:?}")))?;
    let encoded = sink.as_slice();

    // Rebuild the metadata: flacenc writes STREAMINFO, we add the rest.
    let mut reader = Cursor::new(encoded);
    let mut tag = metaflac::Tag::read_from(&mut reader).map_err(|e| Error::Encode(e.to_string()))?;
    // flacenc counts the shorter last frame in the minimum block size, which
    // the format excludes. libFLAC then takes the stream for variable-blocksize
    // and computes wrong sample positions, which breaks seeking.
    if let Some(mut info) = tag.get_streaminfo().cloned() {
        info.min_block_size = info.max_block_size;
        tag.set_streaminfo(info);
    }
    let comments = tag.vorbis_comments_mut();
    comments.vendor_string = concat!("Galette ", env!("CARGO_PKG_VERSION")).to_string();
    for (key, value) in tags {
        comments.comments.entry(key.to_string()).or_default().push(value.clone());
    }
    if let Some(cover) = cover {
        let mut picture = metaflac::block::Picture::new();
        picture.picture_type = PictureType::CoverFront;
        picture.mime_type = cover.mime_type.to_string();
        picture.width = cover.width;
        picture.height = cover.height;
        picture.depth = 24;
        picture.data = cover.data.clone();
        tag.push_block(Block::Picture(picture));
    }
    tag.push_block(Block::Padding(PADDING));

    let mut file = Vec::with_capacity(encoded.len() + PADDING as usize);
    tag.write_to(&mut file).map_err(|e| Error::Encode(e.to_string()))?;
    reader.read_to_end(&mut file)?;

    verify(&file, &samples)?;
    Ok(file)
}

/// Decodes `file` and checks that it holds exactly `expected`.
fn verify(file: &[u8], expected: &[i32]) -> Result<()> {
    let mismatch = |why: String| Error::Encode(format!("FLAC verification failed: {why}"));
    let mut reader = claxon::FlacReader::new(file).map_err(|e| mismatch(e.to_string()))?;
    let mut count = 0;
    for (decoded, &original) in reader.samples().zip(expected) {
        if decoded.map_err(|e| mismatch(e.to_string()))? != original {
            return Err(mismatch(format!("sample {count} differs")));
        }
        count += 1;
    }
    if count != expected.len() || reader.streaminfo().samples != Some((expected.len() / CHANNELS) as u64) {
        return Err(mismatch("wrong length".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_tags() {
        // One second of a stereo ramp, with both extremes of the 16-bit range.
        let mut audio = Vec::new();
        for i in 0..44_100i32 {
            let left = ((i * 37) % 65_536 - 32_768) as i16;
            let right = if i % 2 == 0 { i16::MIN } else { i16::MAX };
            audio.extend(left.to_le_bytes());
            audio.extend(right.to_le_bytes());
        }
        let tags = [("TITLE", "Été".to_string()), ("ARTIST", "A".to_string()), ("ARTIST", "B".to_string())];
        let file = encode(&audio, &tags, None).unwrap();

        let tag = metaflac::Tag::read_from(&mut Cursor::new(&file)).unwrap();
        assert_eq!(tag.get_vorbis("TITLE").unwrap().collect::<Vec<_>>(), ["Été"]);
        assert_eq!(tag.get_vorbis("ARTIST").unwrap().collect::<Vec<_>>(), ["A", "B"]);
        assert!(file.len() < audio.len());
        // 44 100 samples do not fill the last 4096-sample frame.
        let info = tag.get_streaminfo().unwrap();
        assert_eq!((info.min_block_size, info.max_block_size), (4096, 4096));
    }
}
