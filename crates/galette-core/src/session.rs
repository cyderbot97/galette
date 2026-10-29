//! A whole rip, from the disc in the drive to the album folder: the steps
//! shared by every user interface. Progress is reported through [`Event`]s.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::accuraterip::{self, Pressing, Verdict};
use crate::drive::mmc::Capabilities;
use crate::drive::{Drive, DriveInfo};
use crate::ids::{self, AccurateRipId};
use crate::metadata::coverart::CoverArt;
use crate::metadata::{Release, musicbrainz};
use crate::rip::{self, Mode, Progress};
use crate::toc::Toc;
use crate::{Error, Result, cue, flac, gaps, log, tags, wav};

/// What we know about the disc in the drive.
#[derive(Clone)]
pub struct Disc {
    pub drive: DriveInfo,
    pub capabilities: Option<Capabilities>,
    pub toc: Toc,
    /// Matching MusicBrainz releases, filled by [`Disc::lookup`].
    pub releases: Vec<Release>,
    /// AccurateRip checksums, `None` if the disc is not in the database.
    pub accuraterip: Option<Vec<Pressing>>,
    /// Read offset of the drive from the AccurateRip database.
    pub offset: Option<i32>,
}

impl Disc {
    /// Reads what the drive itself can tell, without going online.
    pub fn read(drive: &mut Drive) -> Result<Self> {
        Ok(Self {
            drive: drive.info()?,
            capabilities: drive.capabilities().ok().flatten(),
            toc: drive.read_toc()?,
            releases: Vec::new(),
            accuraterip: None,
            offset: None,
        })
    }

    /// Looks the disc up on MusicBrainz and AccurateRip, and the drive in the
    /// AccurateRip offset database.
    pub fn lookup(&mut self) -> Result<()> {
        self.releases = musicbrainz::Client::new().lookup(&self.toc)?;
        self.accuraterip = accuraterip::fetch(&AccurateRipId::new(&self.toc))?;
        let offsets = accuraterip::fetch_drive_offsets()?;
        self.offset = accuraterip::find_drive_offset(&offsets, &self.drive.vendor, &self.drive.product)
            .map(|o| o.offset);
        Ok(())
    }

    pub fn c2(&self) -> bool {
        self.capabilities.is_some_and(|c| c.c2_pointers)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Lossless and compressed, with tags and cover art.
    Flac,
    /// Uncompressed, without any metadata.
    Wav,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Flac => "flac",
            Format::Wav => "wav",
        }
    }
}

pub struct Settings {
    /// Folder in which the album folder is created.
    pub output: PathBuf,
    pub format: Format,
    pub mode: Mode,
    pub offset: i32,
    /// Metadata to use, possibly edited by the user.
    pub release: Option<Release>,
    pub cover: Option<CoverArt>,
    /// Track numbers to rip, all of them if empty.
    pub tracks: Vec<u8>,
    /// Set from another thread to stop the rip.
    pub cancel: Option<Arc<AtomicBool>>,
}

pub enum Event<'a> {
    Subcode { pregaps: usize, isrcs: usize, mcn: Option<&'a str> },
    Warning(String),
    TrackStarted { number: u8, title: &'a str },
    TrackProgress { number: u8, progress: Progress },
    Encoding { number: u8 },
    TrackDone(&'a log::TrackLog),
}

pub struct Outcome {
    pub album_dir: PathBuf,
    pub tracks: Vec<log::TrackLog>,
    pub log_path: PathBuf,
    pub cue_path: Option<PathBuf>,
}

impl Outcome {
    pub fn accurate(&self) -> usize {
        self.tracks.iter().filter(|t| matches!(t.verdict, Verdict::Accurate { .. })).count()
    }
}

pub fn read_mode_text(mode: Mode, c2: bool) -> String {
    match mode {
        Mode::Burst => "burst (single read)".to_string(),
        Mode::Secure => format!(
            "secure ({}, up to {} re-reads, {} identical reads required)",
            if c2 { "C2 pointers" } else { "no C2 pointers" },
            rip::MAX_REREADS,
            rip::REQUIRED_MATCHES
        ),
    }
}

/// Folder name of an album, also used for its log and cue sheet.
pub fn album_name(release: Option<&Release>, toc: &Toc) -> String {
    match release {
        Some(r) => sanitize(&format!("{} - {}", r.artist, r.title)),
        None => format!("Disc {}", ids::musicbrainz(toc)),
    }
}

pub fn rip(drive: &mut Drive, disc: &Disc, settings: &Settings, on_event: &mut dyn FnMut(Event)) -> Result<Outcome> {
    let toc = &disc.toc;
    let release = settings.release.as_ref();
    let base_name = album_name(release, toc);
    let album_dir = settings.output.join(&base_name);
    fs::create_dir_all(&album_dir).map_err(|e| with_path(e, "cannot create", &album_dir))?;
    if let Some(cover) = &settings.cover {
        let path = album_dir.join(format!("cover.{}", cover.extension()));
        if let Err(e) = fs::write(&path, &cover.data) {
            on_event(Event::Warning(format!("cannot save {}: {e}", path.display())));
        }
    }

    let subcode = read_subcode(drive, toc, on_event);
    on_event(Event::Subcode {
        pregaps: subcode.pregaps.iter().filter(|&&g| g > 0).count(),
        isrcs: subcode.isrcs.iter().flatten().count(),
        mcn: subcode.mcn.as_deref(),
    });
    if toc.tracks[0].start > 0 {
        let hidden = crate::toc::format_msf(toc.tracks[0].start);
        on_event(Event::Warning(format!("{hidden} of hidden audio before track 1 are not ripped")));
    }

    let disc_id = ids::musicbrainz(toc);
    let audio_count = toc.audio_tracks().count();
    let pressings = disc.accuraterip.as_deref();
    let options = rip::Options {
        mode: settings.mode,
        offset: settings.offset,
        c2: disc.c2(),
        cache_kib: disc.capabilities.map_or(0, |c| c.buffer_kib as u32),
        cancel: settings.cancel.as_deref(),
    };
    let mut track_logs = Vec::new();
    let mut cue_tracks = Vec::new();
    for (position, (index, track)) in toc.tracks.iter().enumerate().filter(|(_, t)| t.is_audio).enumerate() {
        if !settings.tracks.is_empty() && !settings.tracks.contains(&track.number) {
            continue;
        }
        let number = track.number;
        let title = release
            .and_then(|r| r.tracks.iter().find(|t| t.number == number as u32))
            .map(|t| t.title.clone())
            .unwrap_or_else(|| format!("Track {number}"));
        on_event(Event::TrackStarted { number, title: &title });
        let isrc = subcode.isrcs[index].clone();

        let mut is_accurate = |audio: &[u8]| {
            let sums = accuraterip::checksums(audio, position + 1, audio_count);
            matches!(accuraterip::verify(pressings, position, sums), Verdict::Accurate { .. })
        };
        let read = rip::read_track(drive, toc, index, options, &mut is_accurate, &mut |progress| {
            on_event(Event::TrackProgress { number, progress })
        })?;
        let audio = read.audio;

        let file_name = format!("{number:02} - {}.{}", sanitize(&title), settings.format.extension());
        let path = album_dir.join(&file_name);
        let file = match settings.format {
            Format::Flac => {
                on_event(Event::Encoding { number });
                let mut tags = tags::vorbis_comments(release, number as u32, audio_count as u32, &disc_id);
                if let Some(isrc) = &isrc {
                    tags.push(("ISRC", isrc.clone()));
                }
                flac::encode(&audio, &tags, settings.cover.as_ref())?
            }
            Format::Wav => {
                let mut file = Vec::with_capacity(audio.len() + 44);
                wav::write(&mut file, &audio)?;
                file
            }
        };
        fs::write(&path, file).map_err(|e| with_path(e, "cannot write", &path))?;

        let checksums = accuraterip::checksums(&audio, position + 1, audio_count);
        let track_log = log::TrackLog {
            number,
            start: track.start,
            file_name: file_name.clone(),
            pregap: subcode.pregaps[index],
            isrc: isrc.clone(),
            peak: log::peak_level(&audio),
            crc: log::copy_crc(&audio),
            checksums,
            verdict: accuraterip::verify(pressings, position, checksums),
            read: read.report,
        };
        on_event(Event::TrackDone(&track_log));
        track_logs.push(track_log);
        cue_tracks.push(cue::TrackFile { index, file_name, pregap: subcode.pregaps[index], isrc });
    }

    let read_mode = read_mode_text(settings.mode, disc.c2());
    let rip_log = log::RipLog {
        date: SystemTime::now(),
        drive: &disc.drive,
        offset: settings.offset,
        read_mode: &read_mode,
        format: settings.format.extension(),
        toc,
        release,
        mcn: subcode.mcn.as_deref(),
        accuraterip_sets: disc.accuraterip.as_ref().map(Vec::len),
        tracks: &track_logs,
    };
    // A partial rip must not replace the log of the full one.
    let log_name = if track_logs.len() == audio_count {
        format!("{base_name}.log")
    } else {
        let numbers: Vec<String> = track_logs.iter().map(|t| t.number.to_string()).collect();
        format!("{base_name} (tracks {}).log", numbers.join(", "))
    };
    let log_path = album_dir.join(log_name);
    fs::write(&log_path, rip_log.render()).map_err(|e| with_path(e, "cannot write", &log_path))?;

    // A cue sheet of part of the disc would describe a disc that does not exist.
    let cue_path = if cue_tracks.len() == audio_count {
        let path = album_dir.join(format!("{base_name}.cue"));
        let sheet = cue::cue_sheet(toc, release, subcode.mcn.as_deref(), &cue_tracks);
        fs::write(&path, sheet).map_err(|e| with_path(e, "cannot write", &path))?;
        Some(path)
    } else {
        None
    };

    Ok(Outcome { album_dir, tracks: track_logs, log_path, cue_path })
}

/// What the sub-channel tells about the disc, beyond the TOC.
struct Subcode {
    mcn: Option<String>,
    /// Per entry of `toc.tracks`.
    isrcs: Vec<Option<String>>,
    pregaps: Vec<u32>,
}

/// Some drives do not support these commands: the rip goes on without them.
fn read_subcode(drive: &mut Drive, toc: &Toc, on_event: &mut dyn FnMut(Event)) -> Subcode {
    let mcn = drive.read_mcn().unwrap_or_else(|e| {
        on_event(Event::Warning(format!("cannot read the MCN: {e}")));
        None
    });
    let isrcs = toc
        .tracks
        .iter()
        .map(|t| if t.is_audio { drive.read_isrc(t.number).ok().flatten() } else { None })
        .collect();
    let pregaps = gaps::detect_pregaps(toc, &mut |lba| drive.read_q(lba)).unwrap_or_else(|e| {
        on_event(Event::Warning(format!("cannot read pregaps: {e}")));
        vec![0; toc.tracks.len()]
    });
    Subcode { mcn, isrcs, pregaps }
}

fn with_path(e: io::Error, what: &str, path: &Path) -> Error {
    Error::Io(io::Error::new(e.kind(), format!("{what} {}: {e}", path.display())))
}

/// Replaces characters that are not allowed in file names on Windows, macOS or Linux.
pub fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect::<String>()
        .trim_end_matches(['.', ' '])
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_file_names() {
        assert_eq!(sanitize("AC/DC: Back in Black?"), "AC_DC_ Back in Black_");
        assert_eq!(sanitize("Etc..."), "Etc");
    }
}
