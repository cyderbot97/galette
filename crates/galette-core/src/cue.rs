//! Cue sheet describing the disc as ripped: one file per track, each pregap
//! at the end of the previous file ("gaps appended", like EAC's default).
//! With it the exact disc can be burnt again or verified by other tools.

use crate::ids;
use crate::metadata::Release;
use crate::toc::{SECTORS_PER_SECOND, Toc};

/// What the cue sheet needs to know about a ripped track.
#[derive(Debug, Clone)]
pub struct TrackFile {
    /// Index in [`Toc::tracks`].
    pub index: usize,
    pub file_name: String,
    /// Length of the pregap in sectors.
    pub pregap: u32,
    pub isrc: Option<String>,
}

/// `tracks` lists every audio track of the disc, in order.
pub fn cue_sheet(toc: &Toc, release: Option<&Release>, mcn: Option<&str>, tracks: &[TrackFile]) -> String {
    let mut cue = String::new();
    let mut line = |text: String| {
        cue.push_str(&text);
        cue.push_str("\r\n");
    };

    if let Some(date) = release.and_then(|r| r.date.as_deref()) {
        line(format!("REM DATE {}", &date[..date.len().min(4)]));
    }
    line(format!("REM DISCID {:08X}", ids::freedb(toc)));
    line(format!("REM COMMENT \"Galette {}\"", env!("CARGO_PKG_VERSION")));
    if let Some(mcn) = mcn {
        line(format!("CATALOG {mcn}"));
    }
    if let Some(release) = release {
        line(format!("PERFORMER {}", quote(&release.artist)));
        line(format!("TITLE {}", quote(&release.title)));
    }

    for (position, file) in tracks.iter().enumerate() {
        let track = &toc.tracks[file.index];
        let info = release.and_then(|r| r.tracks.iter().find(|t| t.number == track.number as u32));
        if position == 0 {
            line(format!("FILE {} WAVE", quote(&file.file_name)));
        }
        line(format!("  TRACK {:02} AUDIO", track.number));
        if let Some(info) = info {
            line(format!("    TITLE {}", quote(&info.title)));
            line(format!("    PERFORMER {}", quote(&info.artist)));
        }
        if let Some(isrc) = &file.isrc {
            line(format!("    ISRC {isrc}"));
        }
        if position == 0 {
            // Before the first track there is no file to hold the pregap.
            if file.pregap > 0 {
                line(format!("    PREGAP {}", msf(file.pregap)));
            }
        } else {
            // The pregap is the end of the previous file.
            if file.pregap > 0 {
                let previous = &tracks[position - 1];
                let length = toc.track_sectors(previous.index);
                line(format!("    INDEX 00 {}", msf(length - file.pregap)));
            }
            line(format!("FILE {} WAVE", quote(&file.file_name)));
        }
        line("    INDEX 01 00:00:00".to_string());
    }
    cue
}

/// Cue sheets have no escaping: double quotes become single ones.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "'"))
}

/// `mm:ss:ff` with 75 frames per second.
fn msf(sectors: u32) -> String {
    let seconds = sectors / SECTORS_PER_SECOND;
    format!("{:02}:{:02}:{:02}", seconds / 60, seconds % 60, sectors % SECTORS_PER_SECOND)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_appended_to_previous_file() {
        let toc = Toc::parse_musicbrainz("1 2 30150 150 10150").unwrap();
        let tracks = [
            TrackFile { index: 0, file_name: "01.flac".into(), pregap: 0, isrc: Some("USWB11302490".into()) },
            TrackFile { index: 1, file_name: "02 \"x\".flac".into(), pregap: 150, isrc: None },
        ];
        let cue = cue_sheet(&toc, None, Some("0093624943099"), &tracks);
        let expected = [
            "REM DISCID 0B019002",
            "CATALOG 0093624943099",
            "FILE \"01.flac\" WAVE",
            "  TRACK 01 AUDIO",
            "    ISRC USWB11302490",
            "    INDEX 01 00:00:00",
            "  TRACK 02 AUDIO",
            "    INDEX 00 02:11:25",
            "FILE \"02 'x'.flac\" WAVE",
            "    INDEX 01 00:00:00",
        ];
        let lines: Vec<&str> = cue.lines().filter(|l| !l.starts_with("REM COMMENT")).collect();
        assert_eq!(lines, expected);
    }
}
