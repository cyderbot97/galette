//! AAC in .m4a files, encoded by Apple's own encoder (AudioToolbox), the one
//! Apple Music uses: 256 kbit/s constrained VBR, like the iTunes Store's
//! "iTunes Plus" files. macOS only, since the encoder ships with the system.
//!
//! The encoder starts every file with silent "priming" frames; the file says
//! how many (edit list and `iTunSMPB` tag) so that players skip them and
//! albums play without gaps.

use std::ffi::c_void;
use std::io::Cursor;
use std::ptr;

use crate::{Error, Result, m4a};

const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u32 = 2;
const BYTES_PER_FRAME: u32 = CHANNELS * 2;
const FRAMES_PER_PACKET: u32 = 1024;
const BITRATE: u32 = 256_000;

// ---------- AudioToolbox FFI (AudioConverter.h, CoreAudioTypes.h) ----------

type OSStatus = i32;
type AudioConverterRef = *mut c_void;

const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

const FORMAT_LINEAR_PCM: u32 = fourcc(b"lpcm");
const FORMAT_MPEG4_AAC: u32 = fourcc(b"aac ");
/// Signed integer samples, packed, little endian (no big-endian flag).
const PCM_FLAGS: u32 = 0x4 | 0x8;

const PROPERTY_BITRATE: u32 = fourcc(b"brat");
const PROPERTY_QUALITY: u32 = fourcc(b"cdqu");
const PROPERTY_BITRATE_MODE: u32 = fourcc(b"acbf");
const PROPERTY_MAX_PACKET_SIZE: u32 = fourcc(b"xops");
const PROPERTY_MAGIC_COOKIE: u32 = fourcc(b"cmgc");
const PROPERTY_PRIME_INFO: u32 = fourcc(b"prim");
const QUALITY_MAX: u32 = 0x7F;
const BITRATE_MODE_VBR_CONSTRAINED: u32 = 2;

#[repr(C)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct AudioBuffer {
    channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    count: u32,
    buffers: [AudioBuffer; 1],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AudioStreamPacketDescription {
    start_offset: i64,
    variable_frames: u32,
    data_byte_size: u32,
}

#[repr(C)]
#[derive(Default)]
struct PrimeInfo {
    leading_frames: u32,
    trailing_frames: u32,
}

type InputProc = extern "C" fn(
    AudioConverterRef,
    *mut u32,
    *mut AudioBufferList,
    *mut *mut AudioStreamPacketDescription,
    *mut c_void,
) -> OSStatus;

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioConverterNew(
        source: *const AudioStreamBasicDescription,
        destination: *const AudioStreamBasicDescription,
        converter: *mut AudioConverterRef,
    ) -> OSStatus;
    fn AudioConverterDispose(converter: AudioConverterRef) -> OSStatus;
    fn AudioConverterSetProperty(converter: AudioConverterRef, id: u32, size: u32, data: *const c_void) -> OSStatus;
    fn AudioConverterGetProperty(converter: AudioConverterRef, id: u32, size: *mut u32, data: *mut c_void) -> OSStatus;
    fn AudioConverterGetPropertyInfo(converter: AudioConverterRef, id: u32, size: *mut u32, writable: *mut u8) -> OSStatus;
    fn AudioConverterFillComplexBuffer(
        converter: AudioConverterRef,
        input: InputProc,
        user_data: *mut c_void,
        packets: *mut u32,
        output: *mut AudioBufferList,
        descriptions: *mut AudioStreamPacketDescription,
    ) -> OSStatus;
}

fn check(status: OSStatus, what: &str) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::Encode(format!("AAC encoder: {what} failed (OSStatus {status})")))
    }
}

/// An AudioConverter, disposed of when dropped.
struct Converter(AudioConverterRef);

impl Drop for Converter {
    fn drop(&mut self) {
        // SAFETY: the converter was created by AudioConverterNew and is not used after this.
        unsafe { AudioConverterDispose(self.0) };
    }
}

impl Converter {
    fn new() -> Result<Self> {
        let pcm = AudioStreamBasicDescription {
            sample_rate: SAMPLE_RATE as f64,
            format_id: FORMAT_LINEAR_PCM,
            format_flags: PCM_FLAGS,
            bytes_per_packet: BYTES_PER_FRAME,
            frames_per_packet: 1,
            bytes_per_frame: BYTES_PER_FRAME,
            channels_per_frame: CHANNELS,
            bits_per_channel: 16,
            reserved: 0,
        };
        // Zero fields are filled in by the encoder.
        let aac = AudioStreamBasicDescription {
            sample_rate: SAMPLE_RATE as f64,
            format_id: FORMAT_MPEG4_AAC,
            format_flags: 0,
            bytes_per_packet: 0,
            frames_per_packet: FRAMES_PER_PACKET,
            bytes_per_frame: 0,
            channels_per_frame: CHANNELS,
            bits_per_channel: 0,
            reserved: 0,
        };
        let mut converter = ptr::null_mut();
        // SAFETY: both descriptions are valid for the call, `converter` receives the result.
        check(unsafe { AudioConverterNew(&pcm, &aac, &mut converter) }, "creation")?;
        let converter = Converter(converter);
        // The mode must be set before the bitrate, which it constrains.
        converter.set(PROPERTY_BITRATE_MODE, BITRATE_MODE_VBR_CONSTRAINED, "VBR mode")?;
        converter.set(PROPERTY_BITRATE, BITRATE, "bitrate")?;
        converter.set(PROPERTY_QUALITY, QUALITY_MAX, "quality")?;
        Ok(converter)
    }

    fn set(&self, id: u32, value: u32, what: &str) -> Result<()> {
        // SAFETY: `value` is a UInt32, as these properties expect.
        let status = unsafe { AudioConverterSetProperty(self.0, id, 4, (&raw const value).cast()) };
        check(status, what)
    }

    /// A fixed-size property.
    fn get<T: Default>(&self, id: u32, what: &str) -> Result<T> {
        let mut value = T::default();
        let mut size = size_of::<T>() as u32;
        // SAFETY: `value` has room for `size` bytes.
        check(unsafe { AudioConverterGetProperty(self.0, id, &mut size, (&raw mut value).cast()) }, what)?;
        Ok(value)
    }

    /// A property of variable size, as raw bytes.
    fn get_bytes(&self, id: u32, what: &str) -> Result<Vec<u8>> {
        let mut size = 0;
        // SAFETY: plain out-parameters; a null `writable` is allowed.
        check(unsafe { AudioConverterGetPropertyInfo(self.0, id, &mut size, ptr::null_mut()) }, what)?;
        let mut bytes = vec![0u8; size as usize];
        // SAFETY: `bytes` has room for `size` bytes.
        check(unsafe { AudioConverterGetProperty(self.0, id, &mut size, bytes.as_mut_ptr().cast()) }, what)?;
        bytes.truncate(size as usize);
        Ok(bytes)
    }
}

/// What the input callback reads from: the audio not yet given to the encoder.
struct Input<'a> {
    audio: &'a [u8],
}

extern "C" fn supply_input(
    _converter: AudioConverterRef,
    packets: *mut u32,
    data: *mut AudioBufferList,
    _descriptions: *mut *mut AudioStreamPacketDescription,
    user_data: *mut c_void,
) -> OSStatus {
    // SAFETY: AudioToolbox passes back the `Input` given to FillComplexBuffer and
    // valid pointers for the requested packet count and the buffer list.
    unsafe {
        let input = &mut *user_data.cast::<Input>();
        let frames = (*packets as usize).min(input.audio.len() / BYTES_PER_FRAME as usize);
        let (now, later) = input.audio.split_at(frames * BYTES_PER_FRAME as usize);
        input.audio = later;
        // Zero frames tells the encoder the audio is over.
        *packets = frames as u32;
        let buffer = &mut (*data).buffers[0];
        buffer.channels = CHANNELS;
        buffer.data_byte_size = now.len() as u32;
        // The encoder only reads from it.
        buffer.data = now.as_ptr().cast_mut().cast();
    }
    0
}

/// Encodes `audio` (CD audio, little endian) into an .m4a file with `tag`.
pub fn encode(audio: &[u8], tag: &mp4ameta::Tag) -> Result<Vec<u8>> {
    let converter = Converter::new()?;
    let max_packet: u32 = converter.get(PROPERTY_MAX_PACKET_SIZE, "maximum packet size")?;

    const PACKETS_PER_CALL: u32 = 64;
    let mut buffer = vec![0u8; (max_packet * PACKETS_PER_CALL) as usize];
    let mut descriptions = vec![AudioStreamPacketDescription::default(); PACKETS_PER_CALL as usize];
    let mut input = Input { audio };
    let mut packets = Vec::new();
    loop {
        let mut list = AudioBufferList {
            count: 1,
            buffers: [AudioBuffer { channels: CHANNELS, data_byte_size: buffer.len() as u32, data: buffer.as_mut_ptr().cast() }],
        };
        let mut count = PACKETS_PER_CALL;
        // SAFETY: `list` points to `buffer`, which holds PACKETS_PER_CALL packets of the
        // maximum size, `descriptions` has as many entries, and `input` outlives the call.
        let status = unsafe {
            AudioConverterFillComplexBuffer(
                converter.0,
                supply_input,
                (&raw mut input).cast(),
                &mut count,
                &mut list,
                descriptions.as_mut_ptr(),
            )
        };
        check(status, "encoding")?;
        if count == 0 {
            break;
        }
        for d in &descriptions[..count as usize] {
            let start = d.start_offset as usize;
            packets.push(buffer[start..start + d.data_byte_size as usize].to_vec());
        }
    }

    let prime: PrimeInfo = converter.get(PROPERTY_PRIME_INFO, "priming information")?;
    let frames = (audio.len() / BYTES_PER_FRAME as usize) as u32;
    if packets.len() as u64 * u64::from(FRAMES_PER_PACKET) < u64::from(prime.leading_frames + frames) {
        return Err(Error::Encode("AAC encoder: the audio was cut short".into()));
    }
    let sample_entry = sample_entry(&esds(&converter.get_bytes(PROPERTY_MAGIC_COOKIE, "configuration")?)?);

    let file = m4a::write(&m4a::Track {
        sample_entry: &sample_entry,
        sample_rate: SAMPLE_RATE,
        packets: &packets,
        frames_per_packet: FRAMES_PER_PACKET,
        frames,
        priming: prime.leading_frames,
    });
    let mut tag = tag.clone();
    tag.set_data(
        mp4ameta::FreeformIdent::new_static("com.apple.iTunes", "iTunSMPB"),
        mp4ameta::Data::Utf8(itunsmpb(prime.leading_frames, packets.len() as u32, frames)),
    );
    let mut file = Cursor::new(file);
    tag.write_to(&mut file).map_err(|e| Error::Encode(format!("cannot write the MP4 tags: {e}")))?;
    Ok(file.into_inner())
}

/// The `esds` box content from the encoder's "magic cookie": an MPEG-4
/// elementary stream descriptor (tag 3), which is what the box holds.
fn esds(cookie: &[u8]) -> Result<Vec<u8>> {
    if cookie.first() != Some(&3) {
        return Err(Error::Encode("AAC encoder: unexpected configuration".into()));
    }
    Ok([&[0u8; 4][..], cookie].concat()) // version and flags
}

/// The `mp4a` sample entry: an audio sample description and the `esds` box.
fn sample_entry(esds: &[u8]) -> Vec<u8> {
    let mut entry = vec![0u8; 6]; // reserved
    entry.extend(1u16.to_be_bytes()); // data reference index
    entry.extend([0u8; 8]); // version, revision, vendor
    entry.extend((CHANNELS as u16).to_be_bytes());
    entry.extend(16u16.to_be_bytes()); // bits per sample
    entry.extend([0u8; 4]);
    entry.extend((SAMPLE_RATE << 16).to_be_bytes()); // 16.16 fixed point
    entry.extend(m4a::boxed(b"esds", esds));
    m4a::boxed(b"mp4a", &entry)
}

/// iTunes' gapless information: priming frames, padding frames at the end and
/// the real length, in hexadecimal.
fn itunsmpb(priming: u32, packets: u32, frames: u32) -> String {
    let padding = packets * FRAMES_PER_PACKET - priming - frames;
    let mut text = format!(" 00000000 {priming:08X} {padding:08X} {frames:016X}");
    for _ in 0..8 {
        text.push_str(" 00000000");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_with_tags_and_gapless_info() {
        // Two seconds of noise (a pure tone would need far fewer bits), with an
        // odd length so that the last packet is padded.
        let frames = 2 * SAMPLE_RATE + 123;
        let mut seed = 1u32;
        let mut audio = Vec::new();
        for _ in 0..frames * CHANNELS {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            audio.extend(((seed >> 16) as i16 / 4).to_le_bytes());
        }
        let mut tag = mp4ameta::Tag::default();
        tag.set_title("Été");
        let file = encode(&audio, &tag).unwrap();

        let read = mp4ameta::Tag::read_from(&mut Cursor::new(&file)).unwrap();
        assert_eq!(read.title(), Some("Été"));
        let smpb = read.strings_of(&mp4ameta::FreeformIdent::new_static("com.apple.iTunes", "iTunSMPB")).next().unwrap();
        assert!(smpb.contains(&format!("{frames:016X}")), "{smpb}");
        assert!(file.windows(4).any(|w| w == b"esds"));
        assert!(file.windows(4).any(|w| w == b"elst"));
        // Roughly 256 kbit/s of audio.
        let kbps = file.len() as f64 * 8.0 / 2.0 / 1000.0;
        assert!((200.0..330.0).contains(&kbps), "{kbps} kbit/s");
        if let Some(path) = std::env::var_os("GALETTE_AAC_SAMPLE") {
            std::fs::write(path, &file).unwrap();
        }
    }

    #[test]
    fn gapless_info_format() {
        assert_eq!(
            itunsmpb(2112, 3, 960),
            " 00000000 00000840 00000000 00000000000003C0 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000"
        );
    }
}
