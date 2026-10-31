//! Cue sheet describing the disc as ripped: one file per track, each pregap
//! at the end of the previous file ("gaps appended", like EAC's default).
//! A hidden track before track 1 (HTOA) is its own file, holding track 1's
//! INDEX 00. With it the exact disc can be burnt again or verified by other
//! tools.

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

/// `tracks` lists every audio track of the disc, in order; `file_type` is
/// the cue sheet name of their format (WAVE, AIFF, MP3). `hidden` is the
/// file of the hidden track, if it was ripped.
pub fn cue_sheet(
    toc: &Toc,
    release: Option<&Release>,
    mcn: Option<&str>,
    file_type: &str,
    tracks: &[TrackFile],
    hidden: Option<&str>,
) -> String {
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
            line(format!("FILE {} {file_type}", quote(hidden.unwrap_or(&file.file_name))));
        }
        line(format!("  TRACK {:02} AUDIO", track.number));
        if let Some(info) = info {
            line(format!("    TITLE {}", quote(&info.title)));
            line(format!("    PERFORMER {}", quote(&info.artist)));
        }
        if track.pre_emphasis {
            line("    FLAGS PRE".to_string());
        }
        if let Some(isrc) = &file.isrc {
            line(format!("    ISRC {isrc}"));
        }
        if position == 0 {
            if hidden.is_some() {
                line("    INDEX 00 00:00:00".to_string());
                line(format!("FILE {} {file_type}", quote(&file.file_name)));
            } else if file.pregap > 0 {
                // No file holds the pregap: players make up the silence.
                line(format!("    PREGAP {}", msf(file.pregap)));
            }
        } else {
            // The pregap is the end of the previous file.
            if file.pregap > 0 {
                let previous = &tracks[position - 1];
                let length = toc.track_sectors(previous.index);
                line(format!("    INDEX 00 {}", msf(length - file.pregap)));
            }
            line(format!("FILE {} {file_type}", quote(&file.file_name)));
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
        let cue = cue_sheet(&toc, None, Some("0093624943099"), "WAVE", &tracks, None);
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

    #[test]
    fn hidden_track_and_pre_emphasis() {
        let mut toc = Toc::parse_musicbrainz("1 2 30150 1650 10150").unwrap();
        toc.tracks[0].pre_emphasis = true;
        let tracks = [
            TrackFile { index: 0, file_name: "01.flac".into(), pregap: 1500, isrc: None },
            TrackFile { index: 1, file_name: "02.flac".into(), pregap: 0, isrc: None },
        ];
        let lines = |hidden| {
            let cue = cue_sheet(&toc, None, None, "WAVE", &tracks, hidden);
            cue.lines().skip(2).take(5).map(str::to_owned).collect::<Vec<_>>()
        };
        let expected = ["FILE \"00.flac\" WAVE", "  TRACK 01 AUDIO", "    FLAGS PRE", "    INDEX 00 00:00:00", "FILE \"01.flac\" WAVE"];
        assert_eq!(lines(Some("00.flac")), expected);
        // Not ripped: a pregap of silence instead.
        assert_eq!(lines(None), ["FILE \"01.flac\" WAVE", "  TRACK 01 AUDIO", "    FLAGS PRE", "    PREGAP 00:20:00", "    INDEX 01 00:00:00"]);
    }
}
