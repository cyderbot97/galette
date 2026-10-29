//! Extraction of a track's audio from the drive.

use crate::drive::Drive;
use crate::toc::{SECTOR_BYTES, Toc};
use crate::Result;

pub const SAMPLES_PER_SECTOR: i64 = 588;
const BYTES_PER_SAMPLE: i64 = 4;

/// Sectors per READ CD command: 26 × 2352 bytes stays under the 64 KiB that
/// every USB bridge accepts.
const SECTORS_PER_READ: u32 = 26;
const ATTEMPTS: usize = 3;

/// Reads the audio of the track at `index` in [`Toc::tracks`].
///
/// `offset` is the drive's read offset in samples: with an offset of +6, the
/// drive delivers each sample 6 positions too early, so we read 6 samples
/// further. Samples falling outside the disc (before LBA 0 or after the
/// lead-out) cannot be read by most drives and are left silent.
///
/// `progress` is called with the fraction read so far, from 0 to 1.
pub fn read_track(
    drive: &mut Drive,
    toc: &Toc,
    index: usize,
    offset: i32,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<u8>> {
    let sector = SECTOR_BYTES as i64;
    let len = toc.track_sectors(index) as i64 * sector;
    let start = toc.tracks[index].start as i64 * sector + offset as i64 * BYTES_PER_SAMPLE;
    let end = start + len;

    let disc_end = toc.audio_lead_out() as i64 * sector;
    let readable = start.max(0)..end.min(disc_end);
    let first_sector = (readable.start / sector) as u32;
    let last_sector = (readable.end as u64).div_ceil(sector as u64) as u32;

    let mut raw = vec![0u8; (last_sector - first_sector) as usize * SECTOR_BYTES];
    for (i, chunk) in raw.chunks_mut(SECTORS_PER_READ as usize * SECTOR_BYTES).enumerate() {
        let lba = first_sector + i as u32 * SECTORS_PER_READ;
        let count = (chunk.len() / SECTOR_BYTES) as u32;
        read_with_retries(drive, lba, count, chunk)?;
        progress((lba + count - first_sector) as f32 / (last_sector - first_sector) as f32);
    }

    // Copy the part of `raw` that belongs to the track to its place in `audio`.
    let mut audio = vec![0u8; len as usize];
    let raw_start = first_sector as i64 * sector;
    let src = (readable.start - raw_start) as usize..(readable.end - raw_start) as usize;
    let dst = (readable.start - start) as usize;
    audio[dst..dst + src.len()].copy_from_slice(&raw[src]);
    Ok(audio)
}

fn read_with_retries(drive: &mut Drive, lba: u32, count: u32, buf: &mut [u8]) -> Result<()> {
    let mut result = Ok(());
    for _ in 0..ATTEMPTS {
        result = drive.read_audio(lba, count, buf);
        if result.is_ok() {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::drive::Transport;

    /// A drive whose disc holds samples numbered from 1 at LBA 0.
    struct FakeDisc;

    impl Transport for FakeDisc {
        fn read(&mut self, cdb: &[u8], buf: &mut [u8], _: Duration) -> Result<usize> {
            let lba = u32::from_be_bytes(cdb[2..6].try_into().unwrap());
            for (i, sample) in buf.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                *sample = (lba * 588 + i as u32 + 1).to_le_bytes();
            }
            Ok(buf.len())
        }
    }

    fn samples(audio: &[u8]) -> Vec<u32> {
        audio.as_chunks::<4>().0.iter().map(|s| u32::from_le_bytes(*s)).collect()
    }

    fn rip(index: usize, offset: i32) -> Vec<u32> {
        let toc = Toc::parse_musicbrainz("1 2 250 150 200").unwrap();
        let mut drive = Drive::new(Box::new(FakeDisc));
        samples(&read_track(&mut drive, &toc, index, offset, &mut |_| {}).unwrap())
    }

    #[test]
    fn applies_read_offset() {
        let track = rip(1, 6);
        assert_eq!(track.len(), 50 * 588);
        assert_eq!(track[0], 50 * 588 + 7);
        // Past the lead-out: silence.
        assert_eq!(&track[track.len() - 6..], &[0; 6]);
        assert_eq!(track[track.len() - 7], 100 * 588);
    }

    #[test]
    fn negative_offset_at_disc_start() {
        let track = rip(0, -30);
        assert_eq!(&track[..30], &[0; 30]);
        assert_eq!(track[30], 1);
        assert_eq!(track[track.len() - 1], 50 * 588 - 30);
    }
}
