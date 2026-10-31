use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};

use galette_core::drive::{self, Drive};
use galette_core::ids::{self, AccurateRipId};
use galette_core::metadata::coverart::{self, CoverArt};
use galette_core::metadata::{Release, musicbrainz};
use galette_core::naming::Naming;
use galette_core::session::{self, Disc, Event};
use galette_core::toc::{self, Toc};
use galette_core::{cache, log, mp3, offset, rip};

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

    /// Find the drive's read offset, with a disc known to AccurateRip
    /// (for drives missing from its offset database).
    Offset {
        /// CD drive to use.
        #[arg(short, long, default_value = drive::default_device())]
        device: String,

        /// Offsets other discs matched, with their confidence, as this
        /// command prints them (e.g. --among=+6:22,+673:200).
        #[arg(long, value_delimiter = ',', allow_hyphen_values = true, value_parser = parse_match)]
        among: Vec<offset::Match>,
    },

    /// Measure the drive's cache, to make sure that secure mode's re-reads
    /// really read the disc again. Any audio CD will do.
    Cache {
        /// CD drive to use.
        #[arg(short, long, default_value = drive::default_device())]
        device: String,
    },
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

    /// Quality of MP3 files.
    #[arg(long, value_enum, default_value_t = Mp3Quality::V0)]
    mp3_quality: Mp3Quality,

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

    /// Do not add ReplayGain tags (loudness, for players to level tracks).
    #[arg(long)]
    no_replaygain: bool,

    /// Do not ask the drive for C2 pointers (for drives that report them badly).
    #[arg(long)]
    no_c2: bool,

    /// How much to read elsewhere on the disc to empty the drive's cache
    /// before re-reading, in KiB, as `galette cache` measured it.
    #[arg(long)]
    cache_kib: Option<u32>,

    /// Album folder template; may contain `/` for subfolders.
    /// Fields: {albumartist} {album} {year} {disc} {disccount}.
    #[arg(long, default_value_t = Naming::default().folder)]
    folder: String,

    /// Track file template, without extension.
    /// Fields: those of --folder, plus {track} {title} {artist}.
    #[arg(long, default_value_t = Naming::default().file)]
    name: String,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// Lossless and compressed, with tags and cover art.
    Flac,
    /// Apple Lossless (.m4a), with tags and cover art.
    Alac,
    /// Uncompressed, without any metadata.
    Wav,
    /// Uncompressed (Apple), with tags and cover art.
    Aiff,
    /// Lossy, with tags and cover art.
    Mp3,
    /// Lossy (.m4a, Apple's encoder, 256 kbit/s), with tags and cover art.
    #[cfg(target_os = "macos")]
    Aac,
}

#[derive(Clone, Copy, ValueEnum)]
enum Mp3Quality {
    /// Variable bitrate, best quality (about 245 kbit/s).
    V0,
    /// Variable bitrate (about 190 kbit/s).
    V2,
    /// Constant 320 kbit/s.
    #[value(name = "320")]
    Cbr320,
}

#[derive(Clone, Copy, ValueEnum)]
enum ReadMode {
    Burst,
    Secure,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { device, toc, offline } => info(&device, toc.as_deref(), offline),
        Command::Rip(args) => rip(&args),
        Command::Offset { device, among } => find_offset(&device, &among),
        Command::Cache { device } => test_cache(&device),
    }
}

/// `+6:22`: an offset and its confidence.
fn parse_match(text: &str) -> std::result::Result<offset::Match, String> {
    let (offset, confidence) = text.split_once(':').ok_or("expected offset:confidence, e.g. +6:22")?;
    let offset = offset.trim_start_matches('+').parse().map_err(|_| format!("not an offset: {offset}"))?;
    let confidence = confidence.parse().map_err(|_| format!("not a confidence: {confidence}"))?;
    Ok(offset::Match { offset, confidence })
}

fn find_offset(device: &str, among: &[offset::Match]) -> Result<()> {
    let mut drive = Drive::open(Some(device))?;
    let mut disc = Disc::read(&mut drive).with_context(|| format!("cannot read the disc in {device}"))?;
    println!("Drive    {} {} ({})", disc.drive.vendor, disc.drive.product, disc.drive.revision);
    disc.lookup().context("online lookup failed")?;
    match disc.offset {
        Some(o) => println!("Offset   {o:+} samples in the AccurateRip database"),
        None => println!("Offset   this drive is not in the AccurateRip database"),
    }
    let Some(pressings) = &disc.accuraterip else {
        bail!("this disc is not in AccurateRip: try another one");
    };
    let known = offset::known_offsets();
    let detection = offset::detect(&mut drive, &disc.toc, pressings, &known, None, &mut |track, f| {
        print!("\rReading  track {track}, {:>3}%\x1b[K", (f * 100.0) as u32);
        std::io::stdout().flush().ok();
    })?;
    print!("\r\x1b[K");
    match detection {
        offset::Detection::Found(matches) => {
            let all = matches.len();
            let matches = if among.is_empty() { matches } else { offset::among(&matches, among) };
            if matches.is_empty() {
                println!("None of the {all} offsets this disc matches is among those given: start again without --among");
            } else if let Some(found) = offset::decide(&matches, !among.is_empty()) {
                println!("Found    read offset {:+} samples (AccurateRip confidence {})", found.offset, found.confidence);
                println!("Rip with --offset {:+}", found.offset);
            } else {
                println!("Several offsets match: this disc exists in pressings shifted from one another.");
                for m in &matches {
                    println!("         {:+} (AccurateRip confidence {})", m.offset, m.confidence);
                }
                let list: Vec<String> = matches.iter().map(|m| format!("{:+}:{}", m.offset, m.confidence)).collect();
                println!("Insert another disc and run: galette offset --among={}", list.join(","));
            }
        }
        offset::Detection::NoMatch => {
            println!("No offset within ±{} samples matches: the disc may be damaged; try another one", offset::MAX_OFFSET)
        }
        offset::Detection::UnsuitableDisc => {
            println!("This disc has no middle track known to AccurateRip: try a disc with at least 3 tracks")
        }
    }
    Ok(())
}

fn test_cache(device: &str) -> Result<()> {
    let mut drive = Drive::open(Some(device))?;
    let disc = Disc::read(&mut drive).with_context(|| format!("cannot read the disc in {device}"))?;
    println!("Drive    {} {} ({})", disc.drive.vendor, disc.drive.product, disc.drive.revision);
    let reported = disc.capabilities.map_or(0, |c| u32::from(c.buffer_kib));
    println!("Cache    {reported} KiB according to the drive");
    let result = cache::test(&mut drive, &disc.toc, reported, &mut |kib| {
        print!("\rTesting  reading {kib} KiB elsewhere to empty the cache…\x1b[K");
        std::io::stdout().flush().ok();
    })?;
    print!("\r\x1b[K");
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    println!("Timing   {:.1} ms from the disc, {:.1} ms when read again at once", ms(result.disc_read), ms(result.reread));
    let default = reported.max(rip::MIN_CACHE_KIB);
    match result.flush_kib {
        _ if !result.caches => println!("Result   the drive does not cache audio: re-reads always read the disc"),
        Some(kib) if kib <= default => println!("Result   the drive caches audio; reading {kib} KiB elsewhere empties it, as Galette does"),
        Some(kib) => println!("Result   the drive caches audio; reading {kib} KiB elsewhere empties it: rip with --cache-kib {kib}"),
        None => println!(
            "Result   even reading {} KiB elsewhere does not empty the cache: secure re-reads may come from it (C2 pointers and AccurateRip still hold)",
            cache::MAX_FLUSH_KIB
        ),
    }
    Ok(())
}

fn read_toc(drive: &mut Drive, device: &str) -> Result<Toc> {
    drive.read_toc().with_context(|| format!("cannot read the TOC from {device}"))
}

fn info(device: &str, toc: Option<&str>, offline: bool) -> Result<()> {
    let (toc, cd_text) = match toc {
        Some(text) => (Toc::parse_musicbrainz(text)?, None),
        None => {
            let mut drive = Drive::open(Some(device))?;
            let toc = read_toc(&mut drive, device)?;
            // Optional: many drives and discs have none.
            (toc, drive.read_cd_text().ok().flatten())
        }
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
    if let Some(text) = &cd_text {
        println!();
        println!("CD-Text: {} – {}", text.performer(0).unwrap_or("?"), text.title(0).unwrap_or("?"));
        for track in toc.audio_tracks() {
            let n = usize::from(track.number);
            let performer = text.performer(n).map(|p| format!("  ({p})")).unwrap_or_default();
            println!("    {:>2}. {}{performer}", track.number, text.title(n).unwrap_or("?"));
        }
    }

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
    let mut disc = Disc::read(&mut drive).with_context(|| format!("cannot read the disc in {device}"))?;
    let mode = match args.mode {
        ReadMode::Burst => rip::Mode::Burst,
        ReadMode::Secure => rip::Mode::Secure,
    };
    let format = match args.format {
        Format::Flac => session::Format::Flac,
        Format::Alac => session::Format::Alac,
        Format::Wav => session::Format::Wav,
        Format::Aiff => session::Format::Aiff,
        Format::Mp3 => session::Format::Mp3,
        #[cfg(target_os = "macos")]
        Format::Aac => session::Format::Aac,
    };
    let mp3_quality = match args.mp3_quality {
        Mp3Quality::V0 => mp3::Quality::V0,
        Mp3Quality::V2 => mp3::Quality::V2,
        Mp3Quality::Cbr320 => mp3::Quality::Cbr320,
    };
    println!("Drive    {} {} ({})", disc.drive.vendor, disc.drive.product, disc.drive.revision);
    println!("Mode     {}", session::read_mode_text(mode, !args.no_c2 && disc.c2()));

    disc.lookup().context("online lookup failed")?;
    let offset = match (args.offset, disc.offset) {
        (Some(offset), _) | (None, Some(offset)) => offset,
        (None, None) => bail!("this drive is not in the AccurateRip offset database; give its offset with --offset"),
    };
    println!("Offset   {offset:+} samples");

    let release = match disc.releases.len() {
        0 => match disc.cd_text_release() {
            Some(release) => {
                println!("Album    not found on MusicBrainz, titles taken from the disc's CD-Text");
                Some(release)
            }
            None => {
                println!("Album    not found on MusicBrainz, tracks will be numbered only");
                None
            }
        },
        n if args.release == 0 || args.release > n => bail!("--release must be between 1 and {n}"),
        _ => Some(disc.releases[args.release - 1].clone()),
    };
    if let Some(r) = &release {
        println!("Album    {} – {}", r.artist, r.title);
    }
    match &disc.accuraterip {
        Some(p) => println!("AccurateRip  {} set(s) of checksums for this disc", p.len()),
        None => println!("AccurateRip  disc not in the database, rips cannot be verified"),
    }
    let cover = match &release {
        Some(r) if format.has_tags() && !args.no_cover => fetch_cover(r),
        _ => None,
    };
    let naming = Naming { folder: args.folder.clone(), file: args.name.clone() };
    println!("Output   {}", args.output.join(naming.folder(release.as_ref(), &disc.toc)).display());

    let settings = session::Settings {
        output: args.output.clone(),
        format,
        mp3_quality,
        mode,
        offset,
        c2: !args.no_c2,
        cache_kib: args.cache_kib,
        naming,
        release,
        cover,
        embed_cover: true,
        save_cover: true,
        replaygain: !args.no_replaygain,
        tracks: args.tracks.clone(),
        cancel: None,
    };
    let started = Instant::now();
    let mut label = String::new();
    // Damaged tracks are finished after the others, once repaired.
    let mut labels = std::collections::HashMap::new();
    let outcome = session::rip(&mut drive, &disc, &settings, &mut |event| {
        match event {
            Event::ReadingSubcode => print!("Subcode  reading pregaps, ISRCs and MCN…"),
            Event::Subcode { pregaps, isrcs, mcn } => {
                println!("\rSubcode  {pregaps} pregap(s), {isrcs} ISRC(s), MCN {}\x1b[K", mcn.unwrap_or("none"));
                println!();
            }
            Event::Warning(message) => println!("\rWarning  {message}\x1b[K"),
            Event::TrackStarted { number, title } => {
                label = format!("{number:>2}. {title}");
                labels.insert(number, label.clone());
            }
            Event::TrackProgress { progress, .. } => match progress {
                rip::Progress::Reading(f) => print!("\r{label}  reading {:>3}%\x1b[K", (f * 100.0) as u32),
                rip::Progress::Verifying(f) => print!("\r{label}  verifying {:>3}%\x1b[K", (f * 100.0) as u32),
                rip::Progress::Rereading { attempt, sectors } => print!(
                    "\r{label}  re-reading {sectors} sector(s), round {attempt}/{}\x1b[K",
                    rip::MAX_REREADS
                ),
            },
            Event::HiddenTrackSilent => println!("\r{label}  silent: no file\x1b[K"),
            Event::Encoding { number } => print!("\r{}  encoding\x1b[K", labels[&number]),
            Event::TrackHeld { .. } => println!("\r{label}  damaged: waiting to be repaired from the CTDB\x1b[K"),
            Event::Repairing => print!("\rCTDB     downloading parity data and repairing…\x1b[K"),
            Event::TrackDone(track) => println!(
                "\r{}  {}  [{}]\x1b[K",
                labels[&track.number],
                log::verdict_text(track.verdict),
                log::read_summary(&track.read)
            ),
        }
        std::io::stdout().flush().ok();
    })?;

    println!();
    let seconds = started.elapsed().as_secs_f32();
    if disc.accuraterip.is_some() {
        println!("{}/{} track(s) accurately ripped in {seconds:.0} s", outcome.accurate(), outcome.tracks.len());
    } else {
        // Without AccurateRip, identical secure reads are the best proof there is.
        let clean = outcome.tracks.iter().filter(|t| t.read.is_clean()).count();
        println!(
            "{clean}/{} track(s) ripped without read errors in {seconds:.0} s (disc not in AccurateRip)",
            outcome.tracks.len()
        );
    }
    println!("{}", log::ctdb_summary(&outcome.ctdb));
    println!("Log      {}", outcome.log_path.display());
    Ok(())
}

/// A missing cover is not worth failing the rip for.
fn fetch_cover(release: &Release) -> Option<CoverArt> {
    match coverart::fetch_front(release) {
        Ok(Some(cover)) => {
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
