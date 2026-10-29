//! MPEG-4 audio files (.m4a) holding one track of encoded packets, laid out
//! as iTunes writes them: `ftyp`, then `moov` (the index of the packets),
//! then `mdat` (the packets). Tags are added afterwards in `moov/udta`.

/// One audio track, ready to be stored.
pub struct Track<'a> {
    /// The codec's sample entry box (`alac`, `mp4a`…), with its configuration.
    pub sample_entry: &'a [u8],
    pub sample_rate: u32,
    /// Every packet holds `frames_per_packet` frames, except perhaps the last.
    pub packets: &'a [Vec<u8>],
    pub frames_per_packet: u32,
    /// Total number of frames (samples per channel).
    pub frames: u32,
}

/// A box: its size, its four-character type, its content.
pub fn boxed(kind: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + content.len());
    out.extend_from_slice(&(8 + content.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(content);
    out
}

/// A "full" box, whose content starts with a version and flags.
fn full(kind: &[u8; 4], version_and_flags: u32, content: &[u8]) -> Vec<u8> {
    boxed(kind, &[&version_and_flags.to_be_bytes()[..], content].concat())
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// The identity transformation, required in movie and track headers.
const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

fn matrix() -> Vec<u8> {
    MATRIX.iter().flat_map(|v| v.to_be_bytes()).collect()
}

pub fn write(track: &Track) -> Vec<u8> {
    let ftyp = boxed(b"ftyp", &concat(&[b"M4A ", &0u32.to_be_bytes(), b"M4A mp42isom"]));
    // The packets follow the index, whose size does not depend on where they start.
    let moov_len = moov(track, 0).len();
    let mdat_start = (ftyp.len() + moov_len + 8) as u32;

    let data_len: usize = track.packets.iter().map(Vec::len).sum();
    let mut file = Vec::with_capacity(mdat_start as usize + data_len);
    file.extend(ftyp);
    file.extend(moov(track, mdat_start));
    file.extend_from_slice(&(8 + data_len as u32).to_be_bytes());
    file.extend_from_slice(b"mdat");
    for packet in track.packets {
        file.extend_from_slice(packet);
    }
    file
}

/// The index of a track whose packets start at `data_offset` in the file.
fn moov(track: &Track, data_offset: u32) -> Vec<u8> {
    let rate = track.sample_rate;
    let duration = track.frames;
    let zero = 0u32.to_be_bytes();

    let mvhd = full(b"mvhd", 0, &concat(&[
        &zero, &zero, // creation and modification times
        &rate.to_be_bytes(),
        &duration.to_be_bytes(),
        &0x0001_0000u32.to_be_bytes(), // rate 1.0
        &0x0100u16.to_be_bytes(),      // volume 1.0
        &[0; 10],
        &matrix(),
        &[0; 24],
        &2u32.to_be_bytes(), // next track ID
    ]));
    // Flags: track enabled, in movie, in preview.
    let tkhd = full(b"tkhd", 7, &concat(&[
        &zero, &zero,
        &1u32.to_be_bytes(), // track ID
        &zero,
        &duration.to_be_bytes(),
        &[0; 8],
        &0u16.to_be_bytes(), // layer
        &0u16.to_be_bytes(), // alternate group
        &0x0100u16.to_be_bytes(),
        &[0; 2],
        &matrix(),
        &zero, &zero, // width and height
    ]));
    let mdhd = full(b"mdhd", 0, &concat(&[
        &zero, &zero,
        &rate.to_be_bytes(),
        &duration.to_be_bytes(),
        &0x55C4u16.to_be_bytes(), // language "und"
        &[0; 2],
    ]));
    let hdlr = full(b"hdlr", 0, &concat(&[&zero, b"soun", &[0; 12], b"SoundHandler\0"]));
    let smhd = full(b"smhd", 0, &[0; 4]);
    let dref = full(b"dref", 0, &concat(&[&1u32.to_be_bytes(), &full(b"url ", 1, &[])]));
    let dinf = boxed(b"dinf", &dref);

    let count = track.packets.len() as u32;
    let stsd = full(b"stsd", 0, &concat(&[&1u32.to_be_bytes(), track.sample_entry]));
    // Packet durations: all full, except perhaps the last one.
    let last = duration - track.frames_per_packet * count.saturating_sub(1);
    let mut runs = vec![(count.saturating_sub(1), track.frames_per_packet), (1, last)];
    runs.retain(|&(n, _)| n > 0);
    if runs.len() == 2 && runs[1].1 == runs[0].1 {
        runs = vec![(count, track.frames_per_packet)];
    }
    let mut stts = (runs.len() as u32).to_be_bytes().to_vec();
    for (n, frames) in runs {
        stts.extend(n.to_be_bytes());
        stts.extend(frames.to_be_bytes());
    }
    let stts = full(b"stts", 0, &stts);
    // All packets in a single chunk.
    let stsc = full(b"stsc", 0, &concat(&[&1u32.to_be_bytes(), &1u32.to_be_bytes(), &count.to_be_bytes(), &1u32.to_be_bytes()]));
    let mut stsz = concat(&[&zero, &count.to_be_bytes()]);
    for packet in track.packets {
        stsz.extend((packet.len() as u32).to_be_bytes());
    }
    let stsz = full(b"stsz", 0, &stsz);
    let stco = full(b"stco", 0, &concat(&[&1u32.to_be_bytes(), &data_offset.to_be_bytes()]));

    let stbl = boxed(b"stbl", &concat(&[&stsd, &stts, &stsc, &stsz, &stco]));
    let minf = boxed(b"minf", &concat(&[&smhd, &dinf, &stbl]));
    let mdia = boxed(b"mdia", &concat(&[&mdhd, &hdlr, &minf]));
    let trak = boxed(b"trak", &concat(&[&tkhd, &mdia]));
    boxed(b"moov", &concat(&[&mvhd, &trak]))
}
