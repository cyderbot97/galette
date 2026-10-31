//! CUETools Database (CTDB): a second check besides AccurateRip, often
//! knowing discs that AccurateRip does not, or with few submissions.
//! Its entries also have parity data, which repairs damaged tracks (see
//! [`crate::parity`]).
//!
//! Each entry of the database is one version of the disc: a CRC32 of the
//! whole disc, a CRC32 per track and how many rips confirmed it. The CRCs
//! leave out the first 10 sectors of the disc and its last 10 to 20 sectors
//! (`STRIDE` plus the rest of the length divided by `STRIDE`). An entry may
//! be stored a few samples away from our offset-corrected audio (up to
//! ±`MAX_OFFSET`), so every offset is tried, as CUETools does.

use std::collections::HashMap;

use crc32fast::Hasher;

use crate::parity::{self, Syndromes};
use crate::toc::{SECTOR_BYTES, Toc};
use crate::{Error, Result, http};

const LOOKUP_URL: &str = "http://db.cuetools.net/lookup2.php";
/// Samples per "stride" of CTDB's parity layout: 10 sectors.
const STRIDE: u32 = 10 * 588;
/// Largest offset between an entry and our audio, in samples.
const MAX_OFFSET: u32 = STRIDE / 2 - 1;
const SAMPLE_BYTES: u32 = 4;

/// What the database knows of a disc, before ripping.
#[derive(Debug, Clone, Default)]
pub enum Lookup {
    /// Not asked yet, or the database could not be reached.
    #[default]
    Unavailable,
    NotFound,
    Found(Vec<Entry>),
}

/// The database's verdict on a rip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Unavailable,
    NotFound,
    /// Only whole-disc rips can be checked: the disc CRC needs every track.
    PartialRip,
    Verified(Verification),
}

/// One version of the disc in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// How many rips confirmed it.
    pub confidence: u32,
    pub crc: u32,
    /// One per audio track, when the entry has them.
    pub track_crcs: Option<Vec<u32>>,
    /// The disc layout it was submitted for, as in [`toc_param`].
    pub toc: String,
    /// Where its parity file is, if it has one.
    pub parity_url: Option<String>,
    /// Syndromes per column in the parity file.
    pub npar: usize,
}

/// The disc's layout as the database writes it: the start of every track
/// (data tracks with a minus sign), then the lead-out.
pub fn toc_param(toc: &Toc) -> String {
    let mut parts: Vec<String> =
        toc.tracks.iter().map(|t| format!("{}{}", if t.is_audio { "" } else { "-" }, t.start)).collect();
    parts.push(toc.lead_out.to_string());
    parts.join(":")
}

/// The entries of this disc, `None` if the database does not know it.
pub fn lookup(toc: &Toc) -> Result<Option<Vec<Entry>>> {
    let url = format!("{LOOKUP_URL}?version=3&ctdb=1&fuzzy=1&metadata=none&toc={}", toc_param(toc));
    let Some(data) = http::get_bytes(&http::agent(), &url)? else {
        return Ok(None);
    };
    let text = String::from_utf8(data).map_err(|_| Error::Network("malformed CTDB response".into()))?;
    let entries = parse(&text);
    Ok((!entries.is_empty()).then_some(entries))
}

/// The `<entry … />` elements of a lookup reply.
fn parse(xml: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    for element in xml.split("<entry ").skip(1) {
        let Some(end) = element.find("/>") else { continue };
        // A leading space so that every attribute name follows one.
        let attributes = format!(" {}", &element[..end]);
        let attribute = |name: &str| {
            let key = format!(" {name}=\"");
            let value = &attributes[attributes.find(&key)? + key.len()..];
            Some(value[..value.find('"')?].to_owned())
        };
        let hex = |s: &str| u32::from_str_radix(s, 16).ok();
        let (Some(confidence), Some(crc), Some(toc)) = (
            attribute("confidence").and_then(|c| c.parse().ok()),
            attribute("crc32").as_deref().and_then(hex),
            attribute("toc"),
        ) else {
            continue;
        };
        let track_crcs = attribute("trackcrcs").and_then(|s| s.split(' ').map(hex).collect());
        let parity_url = attribute("hasparity").filter(|u| u.starts_with("http"));
        let npar = attribute("npar").and_then(|n| n.parse().ok()).unwrap_or(0);
        entries.push(Entry { confidence, crc, track_crcs, toc, parity_url, npar });
    }
    entries
}

/// The CRCs of our audio at every offset, built while the tracks are
/// ripped, in order, without keeping the audio.
pub struct Crcs {
    hasher: Hasher,
    /// Samples given so far.
    position: u32,
    /// Where segments start and end (track boundaries and the disc's
    /// edges), in samples from the start of track 1.
    bounds: Vec<u32>,
    /// The CRC of all the audio before each sample near a bound.
    prefix: HashMap<u32, u32>,
    /// The sample positions whose prefix CRC is needed, in order.
    wanted: Vec<u32>,
    next_wanted: usize,
}

/// The samples of each audio track of `toc`.
pub fn track_samples(toc: &Toc) -> Vec<u32> {
    toc.tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.is_audio)
        .map(|(i, _)| toc.track_sectors(i) * (SECTOR_BYTES as u32 / SAMPLE_BYTES))
        .collect()
}

impl Crcs {
    /// For a rip of all the audio tracks of `toc`.
    pub fn new(toc: &Toc) -> Self {
        let lengths = track_samples(toc);
        let total: u32 = lengths.iter().sum();
        // The disc's CRC leaves out a stride at the start, and at the end a
        // stride plus what does not fill a whole one.
        let skip_end = STRIDE + total % STRIDE;
        let mut bounds = vec![STRIDE];
        let mut start = 0;
        for length in &lengths[..lengths.len().saturating_sub(1)] {
            start += length;
            bounds.push(start);
        }
        bounds.push(total.saturating_sub(skip_end));

        let mut wanted: Vec<u32> = bounds
            .iter()
            .flat_map(|&b| b.saturating_sub(MAX_OFFSET)..=(b + MAX_OFFSET).min(total))
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        Self { hasher: Hasher::new(), position: 0, bounds, prefix: HashMap::new(), wanted, next_wanted: 0 }
    }

    /// Adds the next track's audio (16-bit stereo, little endian).
    pub fn add(&mut self, audio: &[u8]) {
        let end = self.position + (audio.len() as u32) / SAMPLE_BYTES;
        let mut fed = self.position;
        while let Some(&x) = self.wanted.get(self.next_wanted) {
            if x > end {
                break;
            }
            let from = ((fed - self.position) * SAMPLE_BYTES) as usize;
            let to = ((x - self.position) * SAMPLE_BYTES) as usize;
            self.hasher.update(&audio[from..to]);
            fed = x;
            self.prefix.insert(x, self.hasher.clone().finalize());
            self.next_wanted += 1;
        }
        self.hasher.update(&audio[((fed - self.position) * SAMPLE_BYTES) as usize..]);
        self.position = end;
    }

    /// The CRCs of the disc and of each track, at every offset from
    /// `-MAX_OFFSET` to `MAX_OFFSET`. `None` if the audio was not complete.
    pub fn finish(self) -> Option<OffsetCrcs> {
        if self.next_wanted < self.wanted.len() || self.bounds.len() < 2 {
            return None;
        }
        let segment = |a: u32, b: u32| -> Option<Vec<u32>> {
            let shift = ShiftTable::new(u64::from(b.checked_sub(a)?) * u64::from(SAMPLE_BYTES));
            (-(MAX_OFFSET as i64)..=MAX_OFFSET as i64)
                .map(|o| {
                    let x = u32::try_from(i64::from(a) + o).ok()?;
                    let y = u32::try_from(i64::from(b) + o).ok()?;
                    // CRC(x..y) from the CRCs of everything before x and before y.
                    Some(self.prefix.get(&y)? ^ shift.apply(*self.prefix.get(&x)?))
                })
                .collect()
        };
        let last = self.bounds.len() - 1;
        Some(OffsetCrcs {
            disc: segment(self.bounds[0], self.bounds[last])?,
            tracks: self.bounds.windows(2).map(|w| segment(w[0], w[1])).collect::<Option<_>>()?,
            bounds: self.bounds,
        })
    }
}

/// CRC32s at offsets `-MAX_OFFSET..=MAX_OFFSET` (index `offset + MAX_OFFSET`).
pub struct OffsetCrcs {
    disc: Vec<u32>,
    tracks: Vec<Vec<u32>>,
    /// Where each track's CRC starts and ends, as in [`Crcs`].
    bounds: Vec<u32>,
}

impl OffsetCrcs {
    /// The offset at which `entry` matches all the `trusted` tracks, if
    /// there is one such track at least.
    fn entry_offset(&self, entry: &Entry, trusted: &[bool]) -> Option<i32> {
        let theirs = entry.track_crcs.as_ref().filter(|c| c.len() == self.tracks.len())?;
        if !trusted.contains(&true) {
            return None;
        }
        (0..=2 * MAX_OFFSET as usize)
            .find(|&o| {
                self.tracks.iter().zip(theirs).zip(trusted).all(|((ours, theirs), &trusted)| !trusted || ours[o] == *theirs)
            })
            .map(|o| o as i32 - MAX_OFFSET as i32)
    }

    /// The CRC of track `track` at `offset` once `fixes` (words from the
    /// start of track 1, see [`parity::Repair`]) are applied, and the fixes
    /// inside it.
    fn repaired(&self, track: usize, offset: i32, fixes: &[(usize, u16)]) -> (u32, Vec<(usize, u16)>) {
        let a = (i64::from(self.bounds[track]) + i64::from(offset)) as usize * 2;
        let b = (i64::from(self.bounds[track + 1]) + i64::from(offset)) as usize * 2;
        let inside: Vec<(usize, u16)> = fixes.iter().copied().filter(|&(w, _)| (a..b).contains(&w)).collect();
        // CRC32 is linear: CRC(x ^ d) = CRC(x) ^ CRC(d) ^ CRC(zeros).
        let mut delta = vec![0u8; (b - a) * 2];
        let zeros = crc32fast::hash(&delta);
        for &(word, value) in &inside {
            delta[(word - a) * 2..][..2].copy_from_slice(&value.to_le_bytes());
        }
        let crc = self.tracks[track][(offset + MAX_OFFSET as i32) as usize] ^ crc32fast::hash(&delta) ^ zeros;
        (crc, inside)
    }
}

/// A track repaired from an entry's parity data, its CRC then matching the
/// entry's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRepair {
    /// Index among the audio tracks.
    pub track: usize,
    /// Word (16-bit sample of one channel) from the start of the track, and
    /// what to XOR it with.
    pub fixes: Vec<(usize, u16)>,
    /// Confirmations of the entries the repaired track matches.
    pub confidence: u32,
}

/// Parity files downloaded at most, from the most confirmed entries.
const MAX_PARITY_FILES: usize = 3;

/// Tries to repair the `damaged` tracks (index among the audio tracks, and
/// doubtful words from the track's start as in [`crate::rip::TrackRead`]),
/// from the parity of the entries matching the other tracks. Only repairs
/// that the entry's track CRC confirms are returned.
pub fn repair(
    entries: &[Entry],
    toc: &Toc,
    crcs: &OffsetCrcs,
    syndromes: &Syndromes,
    damaged: &[(usize, &[u32])],
) -> Vec<TrackRepair> {
    repair_with(entries, toc, crcs, syndromes, damaged, &mut |e| fetch_parity(e).ok().flatten())
}

fn repair_with(
    entries: &[Entry],
    toc: &Toc,
    crcs: &OffsetCrcs,
    syndromes: &Syndromes,
    damaged: &[(usize, &[u32])],
    fetch: &mut dyn FnMut(&Entry) -> Option<Vec<u8>>,
) -> Vec<TrackRepair> {
    let ours = toc_param(toc);
    let erasures: Vec<usize> = damaged
        .iter()
        .flat_map(|&(track, words)| {
            let start = 2 * track_samples(toc)[..track].iter().map(|&s| s as usize).sum::<usize>();
            words.iter().map(move |&w| start + w as usize)
        })
        .collect();
    let damaged: Vec<usize> = damaged.iter().map(|&(track, _)| track).collect();
    let damaged = damaged.as_slice();
    let trusted: Vec<bool> = (0..crcs.tracks.len()).map(|t| !damaged.contains(&t)).collect();
    let mut candidates: Vec<(&Entry, i32)> = entries
        .iter()
        .filter(|e| e.toc == ours && e.parity_url.is_some())
        .filter_map(|e| Some((e, crcs.entry_offset(e, &trusted)?)))
        .collect();
    candidates.sort_by_key(|(e, _)| std::cmp::Reverse(e.confidence));

    let starts: Vec<usize> = track_samples(toc)
        .iter()
        .scan(0usize, |start, &len| {
            let s = *start;
            *start += len as usize;
            Some(s)
        })
        .collect();
    let mut repairs: Vec<TrackRepair> = Vec::new();
    for (entry, offset) in candidates.into_iter().take(MAX_PARITY_FILES) {
        let remaining: Vec<usize> = damaged.iter().copied().filter(|t| !repairs.iter().any(|r| r.track == *t)).collect();
        if remaining.is_empty() {
            break;
        }
        let Some(data) = fetch(entry) else { continue };
        let Some(found) = parity::repair(syndromes, &data, entry.npar, offset, &erasures) else { continue };
        let theirs = entry.track_crcs.as_ref().expect("entries with an offset have track CRCs");
        for track in remaining {
            let (crc, fixes) = crcs.repaired(track, offset, &found.fixes);
            if fixes.is_empty() || crc != theirs[track] {
                continue;
            }
            let confidence = entries
                .iter()
                .filter(|e| e.toc == ours && e.track_crcs.as_ref().is_some_and(|c| c.get(track) == Some(&crc)))
                .map(|e| e.confidence)
                .sum();
            let start = 2 * starts[track];
            let fixes = fixes.into_iter().map(|(word, value)| (word - start, value)).collect();
            repairs.push(TrackRepair { track, fixes, confidence });
        }
    }
    repairs
}

/// Downloads an entry's parity file: `None` if it has none or it is not
/// the expected size.
fn fetch_parity(entry: &Entry) -> Result<Option<Vec<u8>>> {
    let Some(url) = &entry.parity_url else { return Ok(None) };
    let data = http::get_bytes(&http::agent(), url)?;
    Ok(data.filter(|d| d.len() == entry.npar * parity::ROW * 2))
}

/// `crc` followed by `len` more bytes, all zero: what combining CRCs needs.
/// It is linear in `crc`, so a table per length makes it four lookups.
struct ShiftTable([[u32; 256]; 4]);

impl ShiftTable {
    fn new(len: u64) -> Self {
        let zeros = Hasher::new_with_initial_len(0, len);
        let shift = |crc: u32| {
            let mut h = Hasher::new_with_initial(crc);
            h.combine(&zeros);
            h.finalize()
        };
        let bits: Vec<u32> = (0..32).map(|k| shift(1 << k)).collect();
        let mut table = [[0u32; 256]; 4];
        for (byte, row) in table.iter_mut().enumerate() {
            for (value, cell) in row.iter_mut().enumerate() {
                *cell = (0..8).filter(|b| value >> b & 1 == 1).fold(0, |acc, b| acc ^ bits[byte * 8 + b]);
            }
        }
        Self(table)
    }

    fn apply(&self, crc: u32) -> u32 {
        let t = &self.0;
        t[0][(crc & 0xFF) as usize]
            ^ t[1][(crc >> 8 & 0xFF) as usize]
            ^ t[2][(crc >> 16 & 0xFF) as usize]
            ^ t[3][(crc >> 24) as usize]
    }
}

/// What the database says of a rip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    /// Confirmations of the entries for this layout, all versions together.
    pub total: u32,
    /// Confirmations of the entries matching the whole disc.
    pub disc: u32,
    /// For each track, confirmations of the entries it matches.
    pub tracks: Vec<u32>,
}

/// Compares our CRCs with the entries made for the same layout as `toc`.
pub fn verify(entries: &[Entry], toc: &Toc, crcs: &OffsetCrcs) -> Verification {
    let ours = toc_param(toc);
    let mut result = Verification { total: 0, disc: 0, tracks: vec![0; crcs.tracks.len()] };
    for entry in entries.iter().filter(|e| e.toc == ours) {
        result.total += entry.confidence;
        if crcs.disc.contains(&entry.crc) {
            result.disc += entry.confidence;
            result.tracks.iter_mut().for_each(|t| *t += entry.confidence);
            continue;
        }
        // Otherwise each track on its own (e.g. one damaged track in that entry).
        if let Some(track_crcs) = entry.track_crcs.as_ref().filter(|c| c.len() == crcs.tracks.len()) {
            for ((confidence, ours), theirs) in result.tracks.iter_mut().zip(&crcs.tracks).zip(track_crcs) {
                if ours.contains(theirs) {
                    *confidence += entry.confidence;
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toc::Track;

    /// A disc of three short audio tracks filled with varying samples.
    fn disc() -> (Toc, Vec<Vec<u8>>) {
        let starts = [0u32, 30, 75];
        let toc = Toc::new(
            starts.iter().enumerate().map(|(i, &s)| Track { number: i as u8 + 1, start: s, is_audio: true, pre_emphasis: false }).collect(),
            110,
        )
        .unwrap();
        let mut seed = 7u32;
        let audio: Vec<u8> = (0..110 * SECTOR_BYTES)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (seed >> 16) as u8
            })
            .collect();
        let sector = |s: u32| s as usize * SECTOR_BYTES;
        let tracks = vec![
            audio[..sector(30)].to_vec(),
            audio[sector(30)..sector(75)].to_vec(),
            audio[sector(75)..].to_vec(),
        ];
        (toc, tracks)
    }

    fn crc(bytes: &[u8]) -> u32 {
        crc32fast::hash(bytes)
    }

    #[test]
    fn finds_entries_at_another_offset() {
        let (toc, tracks) = disc();
        let all = tracks.concat();
        let total = all.len() as u32 / SAMPLE_BYTES;
        let end = total - (STRIDE + total % STRIDE);
        let bytes = |a: u32, b: u32, o: i32| &all[((a as i32 + o) * 4) as usize..((b as i32 + o) * 4) as usize];

        // An entry stored 6 samples away, and one for another layout.
        let o = 6;
        let t2 = 30 * 588;
        let t3 = 75 * 588;
        let entry = Entry {
            confidence: 12,
            crc: crc(bytes(STRIDE, end, o)),
            track_crcs: Some(vec![crc(bytes(STRIDE, t2, o)), crc(bytes(t2, t3, o)), crc(bytes(t3, end, o))]),
            toc: toc_param(&toc),
            parity_url: None,
            npar: 0,
        };
        // An entry whose disc CRC differs (another pressing's last track).
        let partial = Entry { confidence: 3, crc: 1, track_crcs: Some(vec![entry.track_crcs.as_ref().unwrap()[0], 2, 3]), ..entry.clone() };
        let other_layout = Entry { toc: "0:1:2".into(), ..entry.clone() };

        let mut crcs = Crcs::new(&toc);
        for track in &tracks {
            crcs.add(track);
        }
        let crcs = crcs.finish().unwrap();
        let result = verify(&[entry, partial, other_layout], &toc, &crcs);
        assert_eq!(result, Verification { total: 15, disc: 12, tracks: vec![15, 12, 12] });
    }

    #[test]
    fn repairs_a_damaged_track() {
        let (toc, tracks) = disc();
        let all = tracks.concat();
        let total = all.len() as u32 / SAMPLE_BYTES;
        let end = total - (STRIDE + total % STRIDE);
        let bytes = |a: u32, b: u32, o: i32| &all[((a as i32 + o) * 4) as usize..((b as i32 + o) * 4) as usize];
        let (o, t2, t3) = (6, 30 * 588, 75 * 588);
        let entry = Entry {
            confidence: 12,
            crc: crc(bytes(STRIDE, end, o)),
            track_crcs: Some(vec![crc(bytes(STRIDE, t2, o)), crc(bytes(t2, t3, o)), crc(bytes(t3, end, o))]),
            toc: toc_param(&toc),
            parity_url: Some("http://example.com/1".into()),
            npar: parity::NPAR,
        };
        let file = parity::parity_file(&all, o);

        // Track 2 scratched: a few words wrong in two places.
        let mut damaged = tracks.clone();
        for word in (1000..1010).chain([20_000]) {
            damaged[1][2 * word] ^= 0x40;
        }
        let mut crcs = Crcs::new(&toc);
        let mut syndromes = Syndromes::new(total as usize).unwrap();
        for track in &damaged {
            crcs.add(track);
            syndromes.add(track);
        }
        let crcs = crcs.finish().unwrap();
        let repairs = repair_with(&[entry], &toc, &crcs, &syndromes, &[(1, &[])], &mut |_| Some(file.clone()));
        assert_eq!(repairs.len(), 1);
        assert_eq!((repairs[0].track, repairs[0].confidence, repairs[0].fixes.len()), (1, 12, 11));
        for &(word, value) in &repairs[0].fixes {
            damaged[1][2 * word] ^= value as u8;
            damaged[1][2 * word + 1] ^= (value >> 8) as u8;
        }
        assert!(damaged[1] == tracks[1]);
    }

    #[test]
    fn incomplete_audio_gives_nothing() {
        let (toc, tracks) = disc();
        let mut crcs = Crcs::new(&toc);
        crcs.add(&tracks[0]);
        assert!(crcs.finish().is_none());
    }

    #[test]
    fn parses_lookup_replies() {
        let xml = r#"<ctdb xmlns="http://db.cuetools.net/ns/mmd-1.0#">
 <entry confidence="5343" crc32="a738c57b" hasparity="http://p.cuetools.net/719472" id="719472" npar="8" stride="5880" syndrome="AAA=" toc="0:15433:166862" trackcrcs="5c3f254e 1d472275" />
 <entry confidence="1" crc32="85de5b9f" id="2494293" toc="0:15433:166862" />
</ctdb>"#;
        let entries = parse(xml);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].confidence, 5343);
        assert_eq!(entries[0].crc, 0xa738c57b);
        assert_eq!(entries[0].track_crcs, Some(vec![0x5c3f254e, 0x1d472275]));
        assert_eq!(entries[1].track_crcs, None);
        assert_eq!(entries[1].toc, "0:15433:166862");
        assert_eq!((entries[0].parity_url.as_deref(), entries[0].npar), (Some("http://p.cuetools.net/719472"), 8));
        assert_eq!((entries[1].parity_url.as_deref(), entries[1].npar), (None, 0));
    }
}
