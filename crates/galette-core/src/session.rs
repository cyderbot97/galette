//! A whole rip, from the disc in the drive to the album folder: the steps
//! shared by every user interface. Progress is reported through [`Event`]s.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::accuraterip::{self, Pressing, Verdict};
use crate::drive::mmc::Capabilities;
use crate::drive::{Drive, DriveInfo};
use crate::ids::{self, AccurateRipId};
use crate::metadata::coverart::CoverArt;
use crate::metadata::{Release, musicbrainz};
use crate::naming::Naming;
use crate::rip::{self, Mode, Progress};
use crate::toc::Toc;
#[cfg(target_os = "macos")]
use crate::aac;
use crate::cdtext::CdText;
use crate::{Error, Result, aiff, alac, ctdb, cue, flac, gaps, log, mp3, tags, wav};

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
    /// Titles stored on the disc, if any (and if the drive reads them).
    pub cd_text: Option<CdText>,
    /// The disc in the CUETools Database, filled by [`Disc::lookup`].
    pub ctdb: ctdb::Lookup,
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
            ctdb: ctdb::Lookup::Unavailable,
            cd_text: match drive.read_cd_text() {
                Ok(text) => text,
                Err(e) if e.is_drive_failure() => return Err(e),
                // Many drives and discs have none: the command is refused.
                Err(_) => None,
            },
        })
    }

    /// The CD-Text as a release to start from, if the disc has some.
    pub fn cd_text_release(&self) -> Option<Release> {
        let tracks = self.toc.audio_tracks().map(|t| t.number);
        self.cd_text.as_ref().map(|text| text.release(tracks))
    }

    /// Looks the disc up on MusicBrainz and AccurateRip, and the drive in the
    /// AccurateRip offset database.
    pub fn lookup(&mut self) -> Result<()> {
        self.releases = musicbrainz::Client::new().lookup(&self.toc)?;
        self.accuraterip = accuraterip::fetch(&AccurateRipId::new(&self.toc))?;
        let offsets = accuraterip::fetch_drive_offsets()?;
        self.offset = accuraterip::find_drive_offset(&offsets, &self.drive.vendor, &self.drive.product)
            .map(|o| o.offset);
        // A second opinion: its absence must not stop the rip.
        self.ctdb = match ctdb::lookup(&self.toc) {
            Ok(Some(entries)) => ctdb::Lookup::Found(entries),
            Ok(None) => ctdb::Lookup::NotFound,
            Err(_) => ctdb::Lookup::Unavailable,
        };
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
    /// Apple Lossless in an .m4a file, with tags and cover art.
    Alac,
    /// Uncompressed, without any metadata.
    Wav,
    /// Uncompressed (Apple), with tags and cover art.
    Aiff,
    /// Lossy, with tags and cover art.
    Mp3,
    /// Lossy (Apple's encoder) in an .m4a file, with tags and cover art. Only
    /// on macOS, whose system provides the encoder.
    #[cfg(target_os = "macos")]
    Aac,
}

impl Format {
    /// Short name, as in the settings and at the start of the log's format line.
    pub fn id(self) -> &'static str {
        match self {
            Format::Flac => "flac",
            Format::Alac => "alac",
            Format::Wav => "wav",
            Format::Aiff => "aiff",
            Format::Mp3 => "mp3",
            #[cfg(target_os = "macos")]
            Format::Aac => "aac",
        }
    }

    /// The formats this build can write.
    pub const ALL: &[Format] = &[
        Format::Flac,
        Format::Alac,
        #[cfg(target_os = "macos")]
        Format::Aac,
        Format::Mp3,
        Format::Aiff,
        Format::Wav,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Format::Flac => "flac",
            Format::Alac => "m4a",
            Format::Wav => "wav",
            Format::Aiff => "aiff",
            Format::Mp3 => "mp3",
            #[cfg(target_os = "macos")]
            Format::Aac => "m4a",
        }
    }

    /// Whether the files hold tags and cover art.
    pub fn has_tags(self) -> bool {
        self != Format::Wav
    }

    /// File type in a cue sheet. FLAC files are conventionally called WAVE.
    fn cue_type(self) -> &'static str {
        match self {
            Format::Flac | Format::Alac | Format::Wav => "WAVE",
            Format::Aiff => "AIFF",
            #[cfg(target_os = "macos")]
            Format::Aac => "WAVE",
            Format::Mp3 => "MP3",
        }
    }
}

pub struct Settings {
    /// Folder in which the album folder is created.
    pub output: PathBuf,
    pub format: Format,
    pub mp3_quality: mp3::Quality,
    pub mode: Mode,
    pub offset: i32,
    /// Use C2 pointers if the drive has them.
    pub c2: bool,
    pub naming: Naming,
    /// Metadata to use, possibly edited by the user.
    pub release: Option<Release>,
    pub cover: Option<CoverArt>,
    /// Put the cover in the FLAC files.
    pub embed_cover: bool,
    /// Save the cover next to the files, as `cover.jpg`.
    pub save_cover: bool,
    /// Track numbers to rip, all of them if empty.
    pub tracks: Vec<u8>,
    /// Set from another thread to stop the rip.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// Something went wrong without stopping the rip. `Display` gives the
/// English text (CLI); the GUI shows its own translation.
#[derive(Debug, Clone)]
pub enum Warning {
    /// The cover file could not be written next to the tracks.
    CoverNotSaved { path: PathBuf, error: String },
    /// Audio before track 1 (HTOA), `mm:ss:ff` long, which is not ripped.
    HiddenAudio(String),
    /// The drive could not read the disc's catalog number.
    McnUnreadable(String),
    /// The drive could not read the pregaps: tracks start at their index 1.
    PregapsUnreadable(String),
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Warning::CoverNotSaved { path, error } => write!(f, "cannot save {}: {error}", path.display()),
            Warning::HiddenAudio(length) => write!(f, "{length} of hidden audio before track 1 are not ripped"),
            Warning::McnUnreadable(error) => write!(f, "cannot read the MCN: {error}"),
            Warning::PregapsUnreadable(error) => write!(f, "cannot read pregaps: {error}"),
        }
    }
}

pub enum Event<'a> {
    /// Reading pregaps, ISRCs and MCN, before the first track.
    ReadingSubcode,
    Subcode { pregaps: usize, isrcs: usize, mcn: Option<&'a str> },
    Warning(Warning),
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
    pub ctdb: ctdb::Status,
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

pub fn rip(drive: &mut Drive, disc: &Disc, settings: &Settings, on_event: &mut dyn FnMut(Event)) -> Result<Outcome> {
    let toc = &disc.toc;
    let release = settings.release.as_ref();
    let album_dir = album_dir(settings.output.join(settings.naming.folder(release, toc)), settings.format);
    // The log and cue sheet are named after the album folder.
    let base_name = album_dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
    fs::create_dir_all(&album_dir).map_err(|e| with_path(e, "cannot create", &album_dir))?;
    if let Some(cover) = settings.cover.as_ref().filter(|_| settings.save_cover) {
        let path = album_dir.join(format!("cover.{}", cover.extension()));
        if let Err(e) = fs::write(&path, &cover.data) {
            on_event(Event::Warning(Warning::CoverNotSaved { path: path.clone(), error: e.to_string() }));
        }
    }

    on_event(Event::ReadingSubcode);
    let cancelled = || -> Result<()> {
        match &settings.cancel {
            Some(c) if c.load(Ordering::Relaxed) => Err(Error::Cancelled),
            _ => Ok(()),
        }
    };
    let subcode = read_subcode(drive, toc, &cancelled, on_event)?;
    on_event(Event::Subcode {
        pregaps: subcode.pregaps.iter().filter(|&&g| g > 0).count(),
        isrcs: subcode.isrcs.iter().flatten().count(),
        mcn: subcode.mcn.as_deref(),
    });
    if toc.tracks[0].start > 0 {
        let hidden = crate::toc::format_msf(toc.tracks[0].start);
        on_event(Event::Warning(Warning::HiddenAudio(hidden)));
    }

    let disc_id = ids::musicbrainz(toc);
    let audio_count = toc.audio_tracks().count();
    let pressings = disc.accuraterip.as_deref();
    let options = rip::Options {
        mode: settings.mode,
        offset: settings.offset,
        c2: settings.c2 && disc.c2(),
        cache_kib: disc.capabilities.map_or(0, |c| c.buffer_kib as u32),
        cancel: settings.cancel.as_deref(),
    };
    let mut track_logs = Vec::new();
    let mut cue_tracks = Vec::new();
    // CTDB checks the whole disc at once, from CRCs gathered track by track.
    let whole_disc = toc.audio_tracks().all(|t| settings.tracks.is_empty() || settings.tracks.contains(&t.number));
    let mut ctdb_crcs = (whole_disc && matches!(disc.ctdb, ctdb::Lookup::Found(_))).then(|| ctdb::Crcs::new(toc));
    for (position, (index, track)) in toc.tracks.iter().enumerate().filter(|(_, t)| t.is_audio).enumerate() {
        if !settings.tracks.is_empty() && !settings.tracks.contains(&track.number) {
            continue;
        }
        cancelled()?;
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
        if let Some(crcs) = &mut ctdb_crcs {
            crcs.add(&audio);
        }

        let file_name = format!("{}.{}", settings.naming.file(release, toc, number), settings.format.extension());
        let path = album_dir.join(&file_name);
        cancelled()?;
        if settings.format != Format::Wav {
            on_event(Event::Encoding { number });
        }
        let mut tags = tags::vorbis_comments(release, number as u32, audio_count as u32, &disc_id);
        if let Some(isrc) = &isrc {
            tags.push(("ISRC", isrc.clone()));
        }
        let file = encode(settings, &audio, &tags)?;
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

    let ctdb = match (&disc.ctdb, ctdb_crcs.and_then(ctdb::Crcs::finish)) {
        (ctdb::Lookup::Unavailable, _) => ctdb::Status::Unavailable,
        (ctdb::Lookup::NotFound, _) => ctdb::Status::NotFound,
        (ctdb::Lookup::Found(entries), Some(crcs)) => ctdb::Status::Verified(ctdb::verify(entries, toc, &crcs)),
        (ctdb::Lookup::Found(_), None) => ctdb::Status::PartialRip,
    };

    let read_mode = read_mode_text(settings.mode, settings.c2 && disc.c2());
    let rip_log = log::RipLog {
        date: SystemTime::now(),
        drive: &disc.drive,
        offset: settings.offset,
        read_mode: &read_mode,
        format: &format_text(settings),
        toc,
        release,
        mcn: subcode.mcn.as_deref(),
        cd_text: disc.cd_text.as_ref(),
        ctdb: &ctdb,
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
        let sheet = cue::cue_sheet(toc, release, subcode.mcn.as_deref(), settings.format.cue_type(), &cue_tracks);
        fs::write(&path, sheet).map_err(|e| with_path(e, "cannot write", &path))?;
        Some(path)
    } else {
        None
    };

    Ok(Outcome { album_dir, tracks: track_logs, log_path, cue_path, ctdb })
}

/// A track's file in the chosen format, from its audio and Vorbis comments.
fn encode(settings: &Settings, audio: &[u8], tags: &[(&str, String)]) -> Result<Vec<u8>> {
    let cover = settings.cover.as_ref().filter(|_| settings.embed_cover);
    let mut file = Vec::with_capacity(audio.len() / 2);
    match settings.format {
        Format::Flac => file = flac::encode(audio, tags, cover)?,
        Format::Alac => file = alac::encode(audio, &tags::mp4(tags, cover))?,
        Format::Wav => wav::write(&mut file, audio)?,
        Format::Aiff => {
            let mut id3 = Vec::new();
            tags::id3(tags, cover)
                .write_to(&mut id3, id3::Version::Id3v24)
                .map_err(|e| Error::Encode(format!("cannot write the ID3 tag: {e}")))?;
            aiff::write(&mut file, audio, Some(&id3))?;
        }
        Format::Mp3 => file = mp3::encode(audio, settings.mp3_quality, &tags::id3(tags, cover))?,
        #[cfg(target_os = "macos")]
        Format::Aac => file = aac::encode(audio, &tags::mp4(tags, cover))?,
    }
    Ok(file)
}

/// The output format as written in the log.
fn format_text(settings: &Settings) -> String {
    match settings.format {
        Format::Mp3 => format!("mp3 (LAME {}, {})", mp3lame_encoder::mp3lame_version(), settings.mp3_quality.label()),
        Format::Alac => "alac (m4a)".to_string(),
        #[cfg(target_os = "macos")]
        Format::Aac => "aac (m4a, Apple AudioToolbox, 256 kbit/s VBR)".to_string(),
        format => format.id().to_string(),
    }
}

/// What the sub-channel tells about the disc, beyond the TOC.
struct Subcode {
    mcn: Option<String>,
    /// Per entry of `toc.tracks`.
    isrcs: Vec<Option<String>>,
    pregaps: Vec<u32>,
}

/// Some drives do not support these commands: the rip goes on without them,
/// unless the drive stopped responding altogether.
fn read_subcode(
    drive: &mut Drive,
    toc: &Toc,
    cancelled: &dyn Fn() -> Result<()>,
    on_event: &mut dyn FnMut(Event),
) -> Result<Subcode> {
    let mcn = match drive.read_mcn() {
        Ok(mcn) => mcn,
        Err(e) if e.is_drive_failure() => return Err(e),
        Err(e) => {
            on_event(Event::Warning(Warning::McnUnreadable(e.to_string())));
            None
        }
    };
    let mut isrcs = Vec::new();
    for track in &toc.tracks {
        cancelled()?;
        isrcs.push(match track.is_audio {
            true => match drive.read_isrc(track.number) {
                Err(e) if e.is_drive_failure() => return Err(e),
                result => result.ok().flatten(),
            },
            false => None,
        });
    }
    let pregaps = match gaps::detect_pregaps(toc, &mut |lba| cancelled().and_then(|()| drive.read_q(lba))) {
        Ok(pregaps) => pregaps,
        Err(e) if e.is_drive_failure() || matches!(e, Error::Cancelled) => return Err(e),
        Err(e) => {
            on_event(Event::Warning(Warning::PregapsUnreadable(e.to_string())));
            vec![0; toc.tracks.len()]
        }
    };
    Ok(Subcode { mcn, isrcs, pregaps })
}

/// The album folder, unless it already holds a rip in another format: then
/// "<folder> (FORMAT)", so that no file of that rip is replaced (ALAC and AAC
/// files have the same names, and so do every format's log and cue sheet).
fn album_dir(dir: PathBuf, format: Format) -> PathBuf {
    if other_format_in(&dir, format) {
        let mut name = dir.file_name().unwrap_or_default().to_os_string();
        name.push(format!(" ({})", format.id().to_uppercase()));
        dir.with_file_name(name)
    } else {
        dir
    }
}

/// Whether a log in `dir` tells of a rip in a format other than `format`.
fn other_format_in(dir: &Path, format: Format) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "log"))
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .filter_map(|log| log_format(&log).map(str::to_owned))
        .any(|id| id != format.id())
}

/// The format's short name in a log: "Output format        alac (m4a)" gives "alac".
fn log_format(log: &str) -> Option<&str> {
    log.lines().find_map(|line| line.strip_prefix("Output format")?.split_whitespace().next())
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

    #[test]
    fn keeps_a_rip_in_another_format() {
        let output = std::env::temp_dir().join(format!("galette-test-{}", std::process::id()));
        let dir = output.join("Artist - Album");
        // A new folder, then one holding a rip in the same format: used as is.
        assert_eq!(album_dir(dir.clone(), Format::Alac), dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Artist - Album.log"), "Galette\nOutput format        alac (m4a)\n").unwrap();
        assert_eq!(album_dir(dir.clone(), Format::Alac), dir);
        // Another format goes next to it.
        assert_eq!(album_dir(dir.clone(), Format::Mp3), output.join("Artist - Album (MP3)"));
        fs::remove_dir_all(&output).unwrap();
    }
}
