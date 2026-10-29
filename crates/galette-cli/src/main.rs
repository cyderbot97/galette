use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};

use galette_core::accuraterip::{self, Verdict};
use galette_core::drive::{self, Drive};
use galette_core::ids::{self, AccurateRipId};
use galette_core::metadata::coverart::{self, CoverArt};
use galette_core::metadata::{Release, musicbrainz};
use galette_core::{cue, flac, gaps, log, rip, tags};
use galette_core::toc::{self, Toc};
use galette_core::wav;

#[derive(Parser)]
#[command(name = "galette", version, about = "Secure audio CD ripper")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read the disc's TOC, compute its identifiers and look it up on MusicBrainz.
    Info {
        /// CD drive to use.
        #[arg(short, long, default_value = drive::default_device())]
        device: String,

        /// Use this TOC instead of reading a drive, in MusicBrainz notation:
        /// "first last lead-out offset1 offset2 ..." (offsets include the 150-sector pregap).
        #[arg(long, conflicts_with = "device")]
        toc: Option<String>,

        /// Do not query MusicBrainz.
        #[arg(long)]
        offline: bool,
    },

    /// Rip the disc to FLAC (or WAV) files and verify them with AccurateRip.
    Rip(RipArgs),
}

#[derive(Args)]
struct RipArgs {
    /// CD drive to use.
    #[arg(short, long, default_value = drive::default_device())]
    device: String,

    /// Folder in which the album folder is created.
    #[arg(short, long, default_value = ".")]
    output: PathBuf,

    /// Audio format of the ripped tracks.
    #[arg(short, long, value_enum, default_value_t = Format::Flac)]
    format: Format,

    /// Burst reads each sector once; secure re-reads until the audio is
    /// proven by AccurateRip or confirmed by identical reads.
    #[arg(short, long, value_enum, default_value_t = ReadMode::Secure)]
    mode: ReadMode,

    /// Read offset of the drive in samples. Looked up in the AccurateRip
    /// database when not given.
    #[arg(long, allow_hyphen_values = true)]
    offset: Option<i32>,

    /// Which MusicBrainz release to use when several match (see `galette info`).
    #[arg(long, default_value_t = 1)]
    release: usize,

    /// Only rip these tracks (e.g. --tracks 1,3).
    #[arg(long, value_delimiter = ',')]
    tracks: Vec<u8>,

    /// Do not download the cover art.
    #[arg(long)]
    no_cover: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// Lossless and compressed, with tags and cover art.
    Flac,
    /// Uncompressed, without any metadata.
    Wav,
}

#[derive(Clone, Copy, ValueEnum)]
enum ReadMode {
    Burst,
    Secure,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Format::Flac => "flac",
            Format::Wav => "wav",
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { device, toc, offline } => info(&device, toc.as_deref(), offline),
        Command::Rip(args) => rip(&args),
    }
}

fn read_toc(drive: &mut Drive, device: &str) -> Result<Toc> {
    drive.read_toc().with_context(|| format!("cannot read the TOC from {device}"))
}

fn info(device: &str, toc: Option<&str>, offline: bool) -> Result<()> {
    let toc = match toc {
        Some(text) => Toc::parse_musicbrainz(text)?,
        None => read_toc(&mut Drive::open(Some(device))?, device)?,
    };

    println!("Track  Start      Length    Type");
    for (i, track) in toc.tracks.iter().enumerate() {
        println!(
            "{:>5}  {:>9}  {}  {}",
            track.number,
            track.start,
            toc::format_msf(toc.track_sectors(i)),
            if track.is_audio { "audio" } else { "data" }
        );
    }
    println!("Lead-out {:>9}", toc.lead_out);
    println!();

    let ar = AccurateRipId::new(&toc);
    println!("MusicBrainz Disc ID  {}", ids::musicbrainz(&toc));
    println!("FreeDB ID            {:08x}", ids::freedb(&toc));
    println!("AccurateRip ID       {:03}-{:08x}-{:08x}-{:08x}", ar.track_count, ar.id1, ar.id2, ar.freedb);

    if offline {
        return Ok(());
    }

    let releases = musicbrainz::Client::new().lookup(&toc).context("MusicBrainz lookup failed")?;
    println!();
    if releases.is_empty() {
        println!("This disc is not in MusicBrainz yet.");
        return Ok(());
    }
    println!("{} matching release(s) on MusicBrainz:", releases.len());
    for (i, release) in releases.iter().enumerate() {
        let details: Vec<&str> = [&release.date, &release.country, &release.label, &release.catalog_number]
            .into_iter()
            .filter_map(|d| d.as_deref())
            .collect();
        println!();
        println!("[{}] {} – {}  ({})", i + 1, release.artist, release.title, details.join(", "));
        if release.disc_count > 1 {
            println!("    Disc {}/{}", release.disc_number, release.disc_count);
        }
        println!("    https://musicbrainz.org/release/{}", release.id);
        for track in &release.tracks {
            let length = track
                .length_ms
                .map(|ms| format!("{}:{:02}", ms / 60_000, ms / 1000 % 60))
                .unwrap_or_default();
            println!("    {:>2}. {}  {}", track.number, track.title, length);
        }
    }
    Ok(())
}

fn rip(args: &RipArgs) -> Result<()> {
    let device = args.device.as_str();
    let mut drive = Drive::open(Some(device))?;
    let info = drive.info().context("cannot identify the drive")?;
    let toc = read_toc(&mut drive, device)?;
    println!("Drive    {} {} ({})", info.vendor, info.product, info.revision);
    let caps = drive.capabilities().ok().flatten();
    let c2 = caps.is_some_and(|c| c.c2_pointers);
    let read_mode = match args.mode {
        ReadMode::Burst => "burst (single read)".to_string(),
        ReadMode::Secure => format!(
            "secure ({}, up to {} re-reads, {} identical reads required)",
            if c2 { "C2 pointers" } else { "no C2 pointers" },
            rip::MAX_REREADS,
            rip::REQUIRED_MATCHES
        ),
    };
    println!("Mode     {read_mode}");

    let offset = match args.offset {
        Some(offset) => offset,
        None => {
            let offsets = accuraterip::fetch_drive_offsets().context("cannot download drive offsets")?;
            let Some(found) = accuraterip::find_drive_offset(&offsets, &info.vendor, &info.product) else {
                bail!("this drive is not in the AccurateRip offset database; give its offset with --offset");
            };
            found.offset
        }
    };
    println!("Offset   {offset:+} samples");

    let releases = musicbrainz::Client::new().lookup(&toc).context("MusicBrainz lookup failed")?;
    let release = match releases.len() {
        0 => {
            println!("Album    not found on MusicBrainz, tracks will be numbered only");
            None
        }
        n if args.release == 0 || args.release > n => bail!("--release must be between 1 and {n}"),
        _ => releases.into_iter().nth(args.release - 1),
    };
    if let Some(r) = &release {
        println!("Album    {} – {}", r.artist, r.title);
    }

    let ar_id = AccurateRipId::new(&toc);
    let pressings = accuraterip::fetch(&ar_id).context("AccurateRip lookup failed")?;
    match &pressings {
        Some(p) => println!("AccurateRip  {} set(s) of checksums for this disc", p.len()),
        None => println!("AccurateRip  disc not in the database, rips cannot be verified"),
    }

    let album_dir = args.output.join(match &release {
        Some(r) => sanitize(&format!("{} - {}", r.artist, r.title)),
        None => format!("Disc {}", ids::musicbrainz(&toc)),
    });
    fs::create_dir_all(&album_dir).with_context(|| format!("cannot create {}", album_dir.display()))?;
    println!("Output   {}", album_dir.display());

    let cover = match (&release, args.format) {
        (Some(r), Format::Flac) if !args.no_cover => fetch_cover(r, &album_dir),
        _ => None,
    };

    print!("Subcode  reading pregaps, ISRCs and MCN…");
    std::io::stdout().flush().ok();
    let subcode = read_subcode(&mut drive, &toc);
    let gaps = subcode.pregaps.iter().filter(|&&g| g > 0).count();
    println!(
        "\rSubcode  {gaps} pregap(s), {} ISRC(s), MCN {}          ",
        subcode.isrcs.iter().flatten().count(),
        subcode.mcn.as_deref().unwrap_or("none")
    );
    if toc.tracks[0].start > 0 {
        println!("Warning  {} of hidden audio before track 1 are not ripped", toc::format_msf(toc.tracks[0].start));
    }
    println!();

    let disc_id = ids::musicbrainz(&toc);
    let audio_count = toc.audio_tracks().count();
    let mut track_logs = Vec::new();
    let mut cue_tracks = Vec::new();
    let started = Instant::now();
    for (position, (index, track)) in toc.tracks.iter().enumerate().filter(|(_, t)| t.is_audio).enumerate() {
        if !args.tracks.is_empty() && !args.tracks.contains(&track.number) {
            continue;
        }
        let title = release
            .as_ref()
            .and_then(|r| r.tracks.iter().find(|t| t.number == track.number as u32))
            .map(|t| t.title.clone())
            .unwrap_or_else(|| format!("Track {}", track.number));
        let label = format!("{:>2}. {title}", track.number);
        let isrc = subcode.isrcs[index].clone();

        let options = rip::Options {
            mode: match args.mode {
                ReadMode::Burst => rip::Mode::Burst,
                ReadMode::Secure => rip::Mode::Secure,
            },
            offset,
            c2,
            cache_kib: caps.map_or(0, |c| c.buffer_kib as u32),
        };
        let mut is_accurate = |audio: &[u8]| {
            let sums = accuraterip::checksums(audio, position + 1, audio_count);
            matches!(accuraterip::verify(pressings.as_deref(), position, sums), Verdict::Accurate { .. })
        };
        let read = rip::read_track(&mut drive, &toc, index, options, &mut is_accurate, &mut |p| {
            match p {
                rip::Progress::Reading(f) => print!("\r{label}  reading {:>3}%   ", (f * 100.0) as u32),
                rip::Progress::Verifying(f) => print!("\r{label}  verifying {:>3}%   ", (f * 100.0) as u32),
                rip::Progress::Rereading { attempt, sectors } => {
                    print!("\r{label}  re-reading {sectors} sector(s), round {attempt}/{}   ", rip::MAX_REREADS)
                }
            }
            std::io::stdout().flush().ok();
        })
        .with_context(|| format!("cannot read track {}", track.number))?;
        let audio = read.audio;

        let file_name = format!("{:02} - {}.{}", track.number, sanitize(&title), args.format.extension());
        let path = album_dir.join(&file_name);
        match args.format {
            Format::Flac => {
                print!("\r{label}  encoding\x1b[K");
                std::io::stdout().flush().ok();
                let mut tags = tags::vorbis_comments(release.as_ref(), track.number as u32, audio_count as u32, &disc_id);
                if let Some(isrc) = &isrc {
                    tags.push(("ISRC", isrc.clone()));
                }
                let file = flac::encode(&audio, &tags, cover.as_ref())
                    .with_context(|| format!("cannot encode track {}", track.number))?;
                fs::write(&path, file).with_context(|| format!("cannot write {}", path.display()))?;
            }
            Format::Wav => {
                let mut file =
                    BufWriter::new(File::create(&path).with_context(|| format!("cannot create {}", path.display()))?);
                wav::write(&mut file, &audio)?;
                file.flush()?;
            }
        }

        let checksums = accuraterip::checksums(&audio, position + 1, audio_count);
        let verdict = accuraterip::verify(pressings.as_deref(), position, checksums);
        println!("\r{label}  {}  [{}]\x1b[K", log::verdict_text(verdict), log::read_summary(&read.report));

        cue_tracks.push(cue::TrackFile {
            index,
            file_name: file_name.clone(),
            pregap: subcode.pregaps[index],
            isrc: isrc.clone(),
        });
        track_logs.push(log::TrackLog {
            number: track.number,
            start: track.start,
            file_name,
            pregap: subcode.pregaps[index],
            isrc,
            peak: log::peak_level(&audio),
            crc: log::copy_crc(&audio),
            checksums,
            verdict,
            read: read.report,
        });
    }

    let base_name = album_dir.file_name().unwrap().to_string_lossy().into_owned();
    let rip_log = log::RipLog {
        date: SystemTime::now(),
        drive: &info,
        offset,
        read_mode: &read_mode,
        format: args.format.extension(),
        toc: &toc,
        release: release.as_ref(),
        mcn: subcode.mcn.as_deref(),
        accuraterip_sets: pressings.as_ref().map(Vec::len),
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
    fs::write(&log_path, rip_log.render()).with_context(|| format!("cannot write {}", log_path.display()))?;
    // A cue sheet of part of the disc would describe a disc that does not exist.
    if cue_tracks.len() == audio_count {
        let sheet = cue::cue_sheet(&toc, release.as_ref(), subcode.mcn.as_deref(), &cue_tracks);
        let cue_path = album_dir.join(format!("{base_name}.cue"));
        fs::write(&cue_path, sheet).with_context(|| format!("cannot write {}", cue_path.display()))?;
    }

    let accurate = track_logs.iter().filter(|t| matches!(t.verdict, Verdict::Accurate { .. })).count();
    println!();
    println!(
        "{accurate}/{} track(s) accurately ripped in {:.0} s",
        track_logs.len(),
        started.elapsed().as_secs_f32()
    );
    Ok(())
}

/// What the sub-channel tells about the disc, beyond the TOC.
struct Subcode {
    mcn: Option<String>,
    /// Per entry of `toc.tracks`.
    isrcs: Vec<Option<String>>,
    pregaps: Vec<u32>,
}

/// Some drives do not support these commands: the rip goes on without them.
fn read_subcode(drive: &mut Drive, toc: &Toc) -> Subcode {
    let warn = |what: &str, e: &dyn std::fmt::Display| println!("\rWarning  cannot read {what}: {e}");
    let mcn = drive.read_mcn().unwrap_or_else(|e| {
        warn("the MCN", &e);
        None
    });
    let isrcs = toc
        .tracks
        .iter()
        .map(|t| if t.is_audio { drive.read_isrc(t.number).ok().flatten() } else { None })
        .collect();
    let pregaps = gaps::detect_pregaps(toc, &mut |lba| drive.read_q(lba)).unwrap_or_else(|e| {
        warn("pregaps", &e);
        vec![0; toc.tracks.len()]
    });
    Subcode { mcn, isrcs, pregaps }
}

/// Downloads the front cover and saves it next to the tracks. A missing cover
/// is not worth failing the rip for.
fn fetch_cover(release: &Release, album_dir: &Path) -> Option<CoverArt> {
    match coverart::fetch_front(release) {
        Ok(Some(cover)) => {
            let path = album_dir.join(format!("cover.{}", cover.extension()));
            if let Err(e) = fs::write(&path, &cover.data) {
                println!("Cover    cannot save {}: {e}", path.display());
            }
            println!("Cover    {}×{} {}", cover.width, cover.height, cover.mime_type);
            Some(cover)
        }
        Ok(None) => {
            println!("Cover    none on the Cover Art Archive");
            None
        }
        Err(e) => {
            println!("Cover    download failed: {e}");
            None
        }
    }
}

/// Replaces characters that are not allowed in file names on Windows, macOS or Linux.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect::<String>()
        .trim_end_matches(['.', ' '])
        .to_string()
}
