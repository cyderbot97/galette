use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};

use galette_core::drive::{self, Drive};
use galette_core::ids::{self, AccurateRipId};
use galette_core::metadata::coverart::{self, CoverArt};
use galette_core::metadata::{Release, musicbrainz};
use galette_core::session::{self, Disc, Event};
use galette_core::toc::{self, Toc};
use galette_core::{log, rip};

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
    let mut disc = Disc::read(&mut drive).with_context(|| format!("cannot read the disc in {device}"))?;
    let mode = match args.mode {
        ReadMode::Burst => rip::Mode::Burst,
        ReadMode::Secure => rip::Mode::Secure,
    };
    let format = match args.format {
        Format::Flac => session::Format::Flac,
        Format::Wav => session::Format::Wav,
    };
    println!("Drive    {} {} ({})", disc.drive.vendor, disc.drive.product, disc.drive.revision);
    println!("Mode     {}", session::read_mode_text(mode, disc.c2()));

    disc.lookup().context("online lookup failed")?;
    let offset = match (args.offset, disc.offset) {
        (Some(offset), _) | (None, Some(offset)) => offset,
        (None, None) => bail!("this drive is not in the AccurateRip offset database; give its offset with --offset"),
    };
    println!("Offset   {offset:+} samples");

    let release = match disc.releases.len() {
        0 => {
            println!("Album    not found on MusicBrainz, tracks will be numbered only");
            None
        }
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
    let cover = match (&release, format) {
        (Some(r), session::Format::Flac) if !args.no_cover => fetch_cover(r),
        _ => None,
    };
    println!("Output   {}", args.output.join(session::album_name(release.as_ref(), &disc.toc)).display());

    let settings = session::Settings {
        output: args.output.clone(),
        format,
        mode,
        offset,
        release,
        cover,
        tracks: args.tracks.clone(),
    };
    let started = Instant::now();
    let mut label = String::new();
    let outcome = session::rip(&mut drive, &disc, &settings, &mut |event| {
        match event {
            Event::Subcode { pregaps, isrcs, mcn } => {
                println!("Subcode  {pregaps} pregap(s), {isrcs} ISRC(s), MCN {}", mcn.unwrap_or("none"));
                println!();
            }
            Event::Warning(message) => println!("\rWarning  {message}\x1b[K"),
            Event::TrackStarted { number, title } => label = format!("{number:>2}. {title}"),
            Event::TrackProgress { progress, .. } => match progress {
                rip::Progress::Reading(f) => print!("\r{label}  reading {:>3}%\x1b[K", (f * 100.0) as u32),
                rip::Progress::Verifying(f) => print!("\r{label}  verifying {:>3}%\x1b[K", (f * 100.0) as u32),
                rip::Progress::Rereading { attempt, sectors } => print!(
                    "\r{label}  re-reading {sectors} sector(s), round {attempt}/{}\x1b[K",
                    rip::MAX_REREADS
                ),
            },
            Event::Encoding { .. } => print!("\r{label}  encoding\x1b[K"),
            Event::TrackDone(track) => {
                println!("\r{label}  {}  [{}]\x1b[K", log::verdict_text(track.verdict), log::read_summary(&track.read))
            }
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
