//! The rip log: a plain-text report of how the disc was ripped and verified,
//! kept next to the files as a proof of their quality.

use std::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::accuraterip::{Checksums, Verdict};
use crate::drive::DriveInfo;
use crate::ids::{self, AccurateRipId};
use crate::metadata::Release;
use crate::rip::ReadReport;
use crate::toc::{self, Toc};

pub struct RipLog<'a> {
    pub date: SystemTime,
    pub drive: &'a DriveInfo,
    pub offset: i32,
    /// How the disc was read, e.g. "secure, C2 pointers".
    pub read_mode: &'a str,
    pub format: &'a str,
    pub toc: &'a Toc,
    pub release: Option<&'a Release>,
    pub mcn: Option<&'a str>,
    /// Number of checksum sets in AccurateRip, `None` if the disc is unknown.
    pub accuraterip_sets: Option<usize>,
    pub tracks: &'a [TrackLog],
}

pub struct TrackLog {
    pub number: u8,
    /// First sector of the track.
    pub start: u32,
    pub file_name: String,
    pub pregap: u32,
    pub isrc: Option<String>,
    /// Highest sample, from 0 to 1.
    pub peak: f32,
    /// CRC32 of the audio, as in EAC logs.
    pub crc: u32,
    pub checksums: Checksums,
    pub verdict: Verdict,
    pub read: ReadReport,
}

/// Highest absolute sample value of CD audio, from 0 to 1.
pub fn peak_level(audio: &[u8]) -> f32 {
    let peak = audio.as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s).unsigned_abs()).max().unwrap_or(0);
    peak as f32 / 32_768.0
}

pub fn copy_crc(audio: &[u8]) -> u32 {
    crc32fast::hash(audio)
}

impl RipLog<'_> {
    pub fn render(&self) -> String {
        let mut log = String::new();
        let mut line = |text: String| {
            log.push_str(&text);
            log.push_str("\r\n");
        };
        let none = || "-".to_string();

        line(format!("Galette {} rip log, {}", env!("CARGO_PKG_VERSION"), format_utc(self.date)));
        line(String::new());
        if let Some(r) = self.release {
            line(format!("{} / {}", r.artist, r.title));
            if r.id.is_empty() {
                line("Metadata entered by hand".to_string());
            } else {
                line(format!("MusicBrainz release  https://musicbrainz.org/release/{}", r.id));
            }
        } else {
            line("Unknown disc (not found on MusicBrainz)".to_string());
        }
        line(String::new());
        line(format!("Drive                {} {} ({})", self.drive.vendor, self.drive.product, self.drive.revision));
        line(format!("Read offset          {:+} samples", self.offset));
        line(format!("Read mode            {}", self.read_mode));
        line(format!("Output format        {}", self.format));
        line(String::new());

        let ar = AccurateRipId::new(self.toc);
        line(format!("MusicBrainz Disc ID  {}", ids::musicbrainz(self.toc)));
        line(format!("AccurateRip ID       {:03}-{:08x}-{:08x}-{:08x}", ar.track_count, ar.id1, ar.id2, ar.freedb));
        line(format!("MCN                  {}", self.mcn.map(str::to_string).unwrap_or_else(none)));
        line(String::new());

        line("TOC".to_string());
        line("  Track |   Start  |  Length  | Start sector | End sector".to_string());
        for (i, track) in self.toc.tracks.iter().enumerate() {
            let length = self.toc.track_sectors(i);
            line(format!(
                "  {:>5} | {} | {} | {:>12} | {:>10}{}",
                track.number,
                toc::format_msf(track.start),
                toc::format_msf(length),
                track.start,
                track.start + length - 1,
                if track.is_audio { "" } else { "  (data)" }
            ));
        }
        line(String::new());

        for t in self.tracks {
            line(format!("Track {:02}", t.number));
            line(format!("  File               {}", t.file_name));
            line(format!("  Pregap             {}", toc::format_msf(t.pregap)));
            line(format!("  ISRC               {}", t.isrc.clone().unwrap_or_else(none)));
            line(format!("  Read               {}", read_summary(&t.read)));
            for (label, lbas) in [("Suspicious", &t.read.suspicious), ("Unreadable", &t.read.unreadable)] {
                for range in ranges(lbas) {
                    let position = |lba: u32| toc::format_msf(lba.saturating_sub(t.start));
                    if range.len() == 1 {
                        line(format!("  {label}         {}", position(range.start)));
                    } else {
                        let end = position(range.end - 1);
                        line(format!("  {label}         {} - {end} ({} sectors)", position(range.start), range.len()));
                    }
                }
            }
            line(format!("  Peak level         {:.1} %", t.peak * 100.0));
            line(format!("  Copy CRC           {:08X}", t.crc));
            line(format!("  AccurateRip v1     {:08X}", t.checksums.v1));
            line(format!("  AccurateRip v2     {:08X}", t.checksums.v2));
            line(format!("  Result             {}", verdict_text(t.verdict)));
            line(String::new());
        }

        let damaged = self.tracks.iter().filter(|t| !t.read.is_clean()).count();
        if damaged == 0 {
            line("No read errors.".to_string());
        } else {
            line(format!("{damaged} track(s) with suspicious or unreadable positions."));
        }
        let accurate = self.tracks.iter().filter(|t| matches!(t.verdict, Verdict::Accurate { .. })).count();
        match self.accuraterip_sets {
            None => line("The disc is not in the AccurateRip database: the rip could not be verified.".to_string()),
            Some(sets) => {
                line(format!("AccurateRip: {sets} checksum set(s) for this disc."));
                line(format!("{accurate} of {} track(s) accurately ripped.", self.tracks.len()));
            }
        }
        log
    }
}

pub fn read_summary(read: &ReadReport) -> String {
    let mut parts = vec![if read.passes == 1 { "1 pass".to_string() } else { format!("{} passes", read.passes) }];
    if read.c2_sectors > 0 {
        parts.push(format!("{} sector(s) with C2 errors", read.c2_sectors));
    }
    if read.mismatches > 0 {
        parts.push(format!("{} sector(s) read differently", read.mismatches));
    }
    if read.rereads > 0 {
        parts.push(format!("{} re-read(s)", read.rereads));
    }
    if !read.suspicious.is_empty() {
        parts.push(format!("{} SUSPICIOUS", read.suspicious.len()));
    }
    if !read.unreadable.is_empty() {
        parts.push(format!("{} UNREADABLE", read.unreadable.len()));
    }
    if read.cleared_by_accuraterip > 0 {
        parts.push(format!("{} uncertain sector(s) proven right by AccurateRip", read.cleared_by_accuraterip));
    }
    parts.join(", ")
}

pub fn verdict_text(verdict: Verdict) -> String {
    match verdict {
        Verdict::Accurate { version, confidence } => format!("accurately ripped (v{version}, confidence {confidence})"),
        Verdict::Mismatch => "NOT ACCURATE: no matching checksum in AccurateRip".to_string(),
        Verdict::Unknown => "not verified".to_string(),
    }
}

/// Groups sorted LBAs into ranges of consecutive sectors.
fn ranges(lbas: &[u32]) -> Vec<std::ops::Range<u32>> {
    let mut ranges: Vec<std::ops::Range<u32>> = Vec::new();
    for &lba in lbas {
        match ranges.last_mut() {
            Some(range) if range.end == lba => range.end += 1,
            _ => ranges.push(lba..lba + 1),
        }
    }
    ranges
}

/// `YYYY-MM-DD hh:mm:ss UTC`, without pulling in a date library.
fn format_utc(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    let mut out = String::new();
    write!(out, "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC", rest / 3600, rest / 60 % 60, rest % 60).unwrap();
    out
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn utc_dates() {
        assert_eq!(format_utc(UNIX_EPOCH), "1970-01-01 00:00:00 UTC");
        // 2024-02-29 12:34:56, a leap day.
        assert_eq!(format_utc(UNIX_EPOCH + Duration::from_secs(1_709_210_096)), "2024-02-29 12:34:56 UTC");
    }

    #[test]
    fn groups_positions() {
        assert_eq!(ranges(&[3, 4, 5, 9, 20, 21]), [3..6, 9..10, 20..22]);
        assert!(ranges(&[]).is_empty());
    }

    #[test]
    fn peak() {
        assert_eq!(peak_level(&[0x00, 0x80, 0x10, 0x00]), 1.0);
        assert_eq!(peak_level(&[0x00, 0x40]), 0.5);
    }
}
