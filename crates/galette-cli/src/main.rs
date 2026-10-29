use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};

use galette_core::accuraterip::{self, Verdict};
use galette_core::drive::{self, Drive};
use galette_core::ids::{self, AccurateRipId};
use galette_core::metadata::coverart::{self, CoverArt};
use galette_core::metadata::{Release, musicbrainz};
use galette_core::{flac, rip, tags};
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
    println!();

    let disc_id = ids::musicbrainz(&toc);

    let audio_count = toc.audio_tracks().count();
    let mut accurate = 0;
    let mut ripped = 0;
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

        let audio = rip::read_track(&mut drive, &toc, index, offset, &mut |done| {
            print!("\r{label}  {:>3}%", (done * 100.0) as u32);
            std::io::stdout().flush().ok();
        })
        .with_context(|| format!("cannot read track {}", track.number))?;

        let name = format!("{:02} - {}", track.number, sanitize(&title));
        match args.format {
            Format::Flac => {
                print!("\r{label}  encoding");
                std::io::stdout().flush().ok();
                let tags = tags::vorbis_comments(release.as_ref(), track.number as u32, audio_count as u32, &disc_id);
                let file = flac::encode(&audio, &tags, cover.as_ref())
                    .with_context(|| format!("cannot encode track {}", track.number))?;
                let path = album_dir.join(format!("{name}.flac"));
                fs::write(&path, file).with_context(|| format!("cannot write {}", path.display()))?;
            }
            Format::Wav => {
                let path = album_dir.join(format!("{name}.wav"));
                let mut file =
                    BufWriter::new(File::create(&path).with_context(|| format!("cannot create {}", path.display()))?);
                wav::write(&mut file, &audio)?;
                file.flush()?;
            }
        }

        let sums = accuraterip::checksums(&audio, position + 1, audio_count);
        let verdict = accuraterip::verify(pressings.as_deref(), position, sums);
        let verdict = match verdict {
            Verdict::Accurate { version, confidence } => {
                accurate += 1;
                format!("accurate (v{version}, confidence {confidence})")
            }
            Verdict::Mismatch => "NOT ACCURATE".to_string(),
            Verdict::Unknown => "not verified".to_string(),
        };
        ripped += 1;
        println!("\r{label}  [{:08X} / {:08X}]  {verdict}", sums.v1, sums.v2);
    }

    println!();
    println!("{accurate}/{ripped} track(s) accurately ripped in {:.0} s", started.elapsed().as_secs_f32());
    Ok(())
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
