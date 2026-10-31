//! Galette's desktop application: a Tauri window whose page (in `ui/`) calls
//! the commands below. Everything slow runs in a background thread, so the
//! window stays responsive; rips report their progress as `rip` events, and
//! a watcher thread reports discs being inserted or removed as `drive` events.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use galette_core::accuraterip::Verdict;
use galette_core::drive::{Drive, MediaState};
use galette_core::metadata::Release;
use galette_core::metadata::coverart::{self, CoverArt};
use galette_core::naming::Naming;
use galette_core::rip::{self, Progress};
use galette_core::session::{self, Disc, Event, Format};
use galette_core::{cache, ctdb, ids, log, mp3, offset, toc};

/// Least time between two reading progress events sent to the window.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default)]
struct Shared {
    disc: Mutex<Option<Disc>>,
    /// Covers already downloaded, by release id.
    covers: Mutex<HashMap<String, Option<CoverArt>>>,
    prefs: Mutex<Prefs>,
    prefs_path: Mutex<Option<PathBuf>>,
    /// Ripping, or testing the drive.
    ripping: AtomicBool,
    scanning: AtomicBool,
    /// Offsets a disc matched when looking for the drive's read offset,
    /// when there were several: the next disc picks among them.
    offset_candidates: Mutex<Vec<i32>>,
    cancel: Arc<AtomicBool>,
}

/// The user's settings, saved as JSON in the application's config folder.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Prefs {
    output: PathBuf,
    format: Format,
    mp3_quality: mp3::Quality,
    secure: bool,
    naming: Naming,
    /// Read offset given by hand; `None` to take it from AccurateRip.
    offset: Option<i32>,
    c2: bool,
    /// How much to read elsewhere to empty the drive's cache, in KiB, as
    /// measured by the cache test; `None` to go by what the drive reports.
    cache_kib: Option<u32>,
    embed_cover: bool,
    save_cover: bool,
    eject_after: bool,
    theme: Theme,
    language: Language,
}

/// Language of the window; `Auto` follows the system's.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Language {
    Fr,
    En,
    #[default]
    #[serde(other)]
    Auto,
}

/// Look of the window; each one but the classic lives in ui/themes/<name>/.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Theme {
    Cathedrale,
    // A theme removed since it was saved falls back to the classic look
    // (serde wants this catch-all on the last variant).
    #[default]
    #[serde(other)]
    Classic,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            output: PathBuf::new(),
            format: Format::Flac,
            mp3_quality: mp3::Quality::default(),
            secure: true,
            naming: Naming::default(),
            offset: None,
            c2: true,
            cache_kib: None,
            embed_cover: true,
            save_cover: true,
            eject_after: false,
            theme: Theme::Classic,
            language: Language::Auto,
        }
    }
}

#[tauri::command]
fn get_prefs(state: State<'_, AppState>) -> Prefs {
    state.prefs.lock().unwrap().clone()
}

/// The formats this build can write, so that the window offers only those
/// (AAC exists on macOS only).
#[tauri::command]
fn formats() -> &'static [Format] {
    Format::ALL
}

#[tauri::command]
fn save_prefs(state: State<'_, AppState>, prefs: Prefs) -> CommandResult<()> {
    if let Some(path) = state.prefs_path.lock().unwrap().as_ref() {
        let json = serde_json::to_string_pretty(&prefs).map_err(|e| Message::with("saveSettings", e))?;
        fs::create_dir_all(path.parent().unwrap())
            .and_then(|_| fs::write(path, json))
            .map_err(|e| Message::with("saveSettings", e))?;
    }
    *state.prefs.lock().unwrap() = prefs;
    Ok(())
}

/// Where the first track would be written with these templates.
#[tauri::command]
fn preview_naming(state: State<'_, AppState>, naming: Naming, release: Option<Release>) -> String {
    let disc = state.disc.lock().unwrap();
    let Some(disc) = disc.as_ref() else {
        return String::new();
    };
    let format = state.prefs.lock().unwrap().format;
    let first = disc.toc.audio_tracks().next().map_or(1, |t| t.number);
    let folder = naming.folder(release.as_ref(), &disc.toc);
    let file = naming.file(release.as_ref(), &disc.toc, first);
    folder.join(format!("{file}.{}", format.extension())).display().to_string()
}

type AppState = Arc<Shared>;

type CommandResult<T> = Result<T, Message>;

/// An error or a warning for the window. The window shows its own text for
/// `code`, in the user's language; `detail` is the engine's technical
/// explanation (in English), added to it when there is one.
#[derive(Clone, Debug, Serialize)]
struct Message {
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl Message {
    fn new(code: &'static str) -> Self {
        Self { code, detail: None }
    }

    fn with(code: &'static str, detail: impl ToString) -> Self {
        Self { code, detail: Some(detail.to_string()) }
    }
}

/// Lets `?` turn an engine error into a message.
impl From<galette_core::Error> for Message {
    fn from(e: galette_core::Error) -> Self {
        use galette_core::Error;
        match e {
            Error::NoDrive => Message::new("noDrive"),
            Error::Scsi(s) if s.is_no_medium() => Message::new("noDisc"),
            Error::Scsi(s) => Message::with("scsi", s),
            Error::DriveNotResponding(d) => Message::with("driveNotResponding", d),
            Error::Io(e) => Message::with("io", e),
            Error::InvalidToc(d) => Message::with("invalidToc", d),
            Error::Encode(d) => Message::with("encode", d),
            Error::Network(d) => Message::with("network", d),
            Error::Cancelled => Message::new("cancelled"),
            Error::Unsupported => Message::new("unsupported"),
        }
    }
}

impl From<&session::Warning> for Message {
    fn from(warning: &session::Warning) -> Self {
        use session::Warning;
        match warning {
            Warning::CoverNotSaved { path, error } => Message::with("coverNotSaved", format!("{}: {error}", path.display())),
            Warning::HiddenAudio(length) => Message::with("hiddenAudio", length),
            Warning::PreEmphasis(tracks) => {
                Message::with("preEmphasis", tracks.iter().map(u8::to_string).collect::<Vec<_>>().join(", "))
            }
            Warning::McnUnreadable(e) => Message::with("mcnUnreadable", e),
            Warning::PregapsUnreadable(e) => Message::with("pregapsUnreadable", e),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscView {
    drive: String,
    c2: bool,
    offset: Option<i32>,
    disc_id: String,
    /// Checksum sets in AccurateRip, `None` if the disc is not in it.
    accuraterip: Option<usize>,
    tracks: Vec<TrackView>,
    releases: Vec<Release>,
    /// The disc's own CD-Text, as a release without MusicBrainz identifiers.
    cd_text: Option<Release>,
    /// The online lookup failed.
    warning: Option<Message>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackView {
    /// 0 for the hidden track before track 1.
    number: u8,
    length: String,
    is_audio: bool,
}

/// Reads the disc in the drive and looks it up online.
#[tauri::command]
async fn scan(state: State<'_, AppState>) -> CommandResult<DiscView> {
    let state = Arc::clone(&state);
    blocking(move || {
        state.scanning.store(true, Ordering::SeqCst);
        let result = read_disc(&state);
        state.scanning.store(false, Ordering::SeqCst);
        result
    })
    .await
}

fn read_disc(state: &Shared) -> CommandResult<DiscView> {
    let mut drive = Drive::open(None)?;
    let mut disc = Disc::read(&mut drive)?;
    let warning = disc.lookup().err().map(|e| Message::with("lookup", e));
    // Audio before track 1 is offered as track 0.
    let first = &disc.toc.tracks[0];
    let hidden = (first.start > 0 && first.is_audio)
        .then(|| TrackView { number: 0, length: duration(first.start), is_audio: true });
    let view = DiscView {
        drive: format!("{} {}", disc.drive.vendor, disc.drive.product),
        c2: disc.c2(),
        offset: disc.offset,
        disc_id: ids::musicbrainz(&disc.toc),
        accuraterip: disc.accuraterip.as_ref().map(Vec::len),
        tracks: hidden
            .into_iter()
            .chain(disc.toc.tracks.iter().enumerate().map(|(i, t)| TrackView {
                number: t.number,
                length: duration(disc.toc.track_sectors(i)),
                is_audio: t.is_audio,
            }))
            .collect(),
        releases: disc.releases.clone(),
        cd_text: disc.cd_text_release(),
        warning,
    };
    *state.disc.lock().unwrap() = Some(disc);
    Ok(view)
}

/// The front cover of a release, as a `data:` URL for the page.
#[tauri::command]
async fn cover(state: State<'_, AppState>, release: Release) -> CommandResult<Option<String>> {
    let state = Arc::clone(&state);
    blocking(move || {
        let cached = state.covers.lock().unwrap().get(&release.id).cloned();
        let cover = match cached {
            Some(cover) => cover,
            None => {
                let cover = coverart::fetch_front(&release).ok().flatten();
                state.covers.lock().unwrap().insert(release.id.clone(), cover.clone());
                cover
            }
        };
        Ok(cover.map(|c| {
            let data = base64::engine::general_purpose::STANDARD.encode(&c.data);
            format!("data:{};base64,{data}", c.mime_type)
        }))
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RipRequest {
    /// The release as shown, with the user's edits.
    release: Option<Release>,
    tracks: Vec<u8>,
}

/// What the page receives while ripping.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum RipEvent {
    /// Reading pregaps, ISRCs and MCN before the first track.
    Analyzing,
    Analyzed,
    Warning { warning: Message },
    /// `phase`: "reading", "verifying", "rereading" (round `attempt` of
    /// `max_attempts`, on `sectors` sectors), "held" (damaged, waiting for
    /// the CTDB repair) or "encoding". Number 0 is the hidden track.
    Progress {
        number: u8,
        phase: &'static str,
        fraction: f32,
        #[serde(skip_serializing_if = "Option::is_none")]
        reread: Option<Reread>,
    },
    /// `status`: "accurate" (with `confidence`), "mismatch", "clean" or "errors".
    /// Downloading the CTDB's parity data and repairing the held tracks.
    Repairing,
    /// The hidden track is only silence: no file was written.
    HiddenSilent,
    TrackDone { number: u8, status: &'static str, confidence: Option<u32>, read: ReadView },
    Finished { album_dir: PathBuf, accurate: usize, clean: usize, total: usize, verified: bool, ctdb: CtdbView },
    Failed { error: Message },
    Cancelled,
}

/// The CUETools Database's verdict. `status`: "unavailable", "notFound",
/// "partialRip" or "verified"; then the confirmations matching the whole
/// disc, of all `total`, and for each track (by number).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CtdbView {
    status: &'static str,
    disc: u32,
    total: u32,
    tracks: Vec<(u8, u32)>,
}

impl CtdbView {
    fn new(status: &ctdb::Status, tracks: &[log::TrackLog]) -> Self {
        let view = |status| CtdbView { status, disc: 0, total: 0, tracks: Vec::new() };
        match status {
            ctdb::Status::Unavailable => view("unavailable"),
            ctdb::Status::NotFound => view("notFound"),
            ctdb::Status::PartialRip => view("partialRip"),
            ctdb::Status::Verified(v) => CtdbView {
                status: "verified",
                disc: v.disc,
                total: v.total,
                tracks: tracks.iter().map(|t| t.number).zip(v.tracks.iter().copied()).collect(),
            },
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Reread {
    attempt: u32,
    max_attempts: u32,
    sectors: usize,
}

/// How a track was read, in numbers the window puts into words.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadView {
    passes: u32,
    c2_sectors: u32,
    mismatches: u32,
    rereads: u32,
    suspicious: usize,
    unreadable: usize,
    cleared: usize,
    repaired: usize,
}

/// Starts ripping in the background; progress comes as `rip` events.
#[tauri::command]
fn start_rip(app: AppHandle, state: State<'_, AppState>, request: RipRequest) -> CommandResult<()> {
    let disc = state.disc.lock().unwrap().clone().ok_or(Message::new("noScan"))?;
    let prefs = state.prefs.lock().unwrap().clone();
    let offset = prefs.offset.or(disc.offset).ok_or(Message::new("unknownOffset"))?;
    if state.ripping.swap(true, Ordering::SeqCst) {
        return Err(Message::new("alreadyRipping"));
    }
    state.cancel.store(false, Ordering::SeqCst);
    let cover = request.release.as_ref().and_then(|r| state.covers.lock().unwrap().get(&r.id).cloned().flatten());
    let settings = session::Settings {
        output: prefs.output,
        format: prefs.format,
        mp3_quality: prefs.mp3_quality,
        mode: if prefs.secure { rip::Mode::Secure } else { rip::Mode::Burst },
        offset,
        c2: prefs.c2,
        cache_kib: prefs.cache_kib,
        naming: prefs.naming,
        release: request.release,
        cover,
        embed_cover: prefs.embed_cover && prefs.format.has_tags(),
        save_cover: prefs.save_cover,
        tracks: request.tracks,
        cancel: Some(Arc::clone(&state.cancel)),
    };

    let state = Arc::clone(&state);
    std::thread::spawn(move || {
        let emit = |event: RipEvent| {
            let _ = app.emit("rip", event);
        };
        // Reading progress comes after every few sectors, dozens of times a
        // second: that many events kept WebKitGTK so busy that the window no
        // longer answered clicks (the stop button). Ten a second is enough.
        let mut last_progress = Instant::now() - PROGRESS_INTERVAL;
        let result = Drive::open(None).and_then(|mut drive| {
            session::rip(&mut drive, &disc, &settings, &mut |event| {
                if let Event::TrackProgress { progress: Progress::Reading(_) | Progress::Verifying(_), .. } = event {
                    if last_progress.elapsed() < PROGRESS_INTERVAL {
                        return;
                    }
                    last_progress = Instant::now();
                }
                if let Some(event) = rip_event(event) {
                    emit(event);
                }
            })
        });
        if result.is_ok() && prefs.eject_after {
            let _ = Drive::open(None).and_then(|mut d| d.eject());
        }
        state.ripping.store(false, Ordering::SeqCst);
        emit(match result {
            Ok(outcome) => RipEvent::Finished {
                accurate: outcome.accurate(),
                clean: outcome.tracks.iter().filter(|t| t.read.is_clean()).count(),
                total: outcome.tracks.len(),
                verified: disc.accuraterip.is_some(),
                ctdb: CtdbView::new(&outcome.ctdb, &outcome.tracks),
                album_dir: outcome.album_dir,
            },
            Err(galette_core::Error::Cancelled) => RipEvent::Cancelled,
            Err(e) => RipEvent::Failed { error: e.into() },
        });
    });
    Ok(())
}

fn rip_event(event: Event) -> Option<RipEvent> {
    let progress = |number, phase, fraction| RipEvent::Progress { number, phase, fraction, reread: None };
    Some(match event {
        Event::ReadingSubcode => RipEvent::Analyzing,
        Event::Subcode { .. } => RipEvent::Analyzed,
        Event::Warning(warning) => RipEvent::Warning { warning: (&warning).into() },
        Event::TrackStarted { number, .. } => progress(number, "reading", 0.0),
        Event::TrackProgress { number, progress: p } => match p {
            Progress::Reading(fraction) => progress(number, "reading", fraction),
            Progress::Verifying(fraction) => progress(number, "verifying", fraction),
            Progress::Rereading { attempt, sectors } => RipEvent::Progress {
                number,
                phase: "rereading",
                fraction: attempt as f32 / rip::MAX_REREADS as f32,
                reread: Some(Reread { attempt, max_attempts: rip::MAX_REREADS, sectors }),
            },
        },
        Event::Encoding { number } => progress(number, "encoding", 1.0),
        Event::TrackHeld { number } => progress(number, "held", 1.0),
        Event::Repairing => RipEvent::Repairing,
        Event::HiddenTrackSilent => RipEvent::HiddenSilent,
        Event::TrackDone(track) => {
            let (status, confidence) = match (track.verdict, track.read.is_clean()) {
                (Verdict::Accurate { confidence, .. }, _) => ("accurate", Some(confidence)),
                (Verdict::Mismatch, _) => ("mismatch", None),
                (Verdict::Unknown, true) => ("clean", None),
                (Verdict::Unknown, false) => ("errors", None),
            };
            let read = &track.read;
            let read = ReadView {
                passes: read.passes,
                c2_sectors: read.c2_sectors,
                mismatches: read.mismatches,
                rereads: read.rereads,
                suspicious: read.suspicious.len(),
                unreadable: read.unreadable.len(),
                cleared: read.cleared_by_accuraterip,
                repaired: read.repaired_samples,
            };
            RipEvent::TrackDone { number: track.number, status, confidence, read }
        }
    })
}

/// Result of the read offset detection. `status`: "found" (`offsets` holds
/// it), "ambiguous" (another disc must choose among `offsets`), "noMatch",
/// "unsuitable" (no middle track in AccurateRip) or "notInAccurateRip".
#[derive(Serialize)]
struct OffsetView {
    status: &'static str,
    offsets: Vec<i32>,
}

/// Looks for the drive's read offset with the disc in the drive; progress
/// comes as `tool` events (track number and fraction read).
#[tauri::command]
async fn detect_offset(app: AppHandle, state: State<'_, AppState>) -> CommandResult<OffsetView> {
    let disc = state.disc.lock().unwrap().clone().ok_or(Message::new("noScan"))?;
    let state = Arc::clone(&state);
    blocking(move || {
        let view = |status, offsets| Ok(OffsetView { status, offsets });
        let Some(pressings) = &disc.accuraterip else { return view("notInAccurateRip", Vec::new()) };
        let detection = with_drive(&state, |drive| {
            let known = offset::known_offsets();
            let mut last_progress = Instant::now() - PROGRESS_INTERVAL;
            offset::detect(drive, &disc.toc, pressings, &known, None, &mut |track, fraction| {
                if last_progress.elapsed() >= PROGRESS_INTERVAL {
                    last_progress = Instant::now();
                    let _ = app.emit("tool", (track, fraction));
                }
            })
        })?;
        let matches = match detection {
            offset::Detection::Found(matches) => matches,
            offset::Detection::NoMatch => return view("noMatch", Vec::new()),
            offset::Detection::UnsuitableDisc => return view("unsuitable", Vec::new()),
        };
        // Another disc left candidates: the drive's offset is among them,
        // unless that disc was misread.
        let mut candidates = state.offset_candidates.lock().unwrap();
        let narrowed = offset::among(&matches, &candidates);
        let kept = if narrowed.is_empty() { matches } else { narrowed };
        let offsets: Vec<i32> = kept.iter().map(|m| m.offset).collect();
        if let [_] = offsets[..] {
            candidates.clear();
            view("found", offsets)
        } else {
            *candidates = offsets.clone();
            view("ambiguous", offsets)
        }
    })
    .await
}

/// What the cache test found, see [`cache::CacheTest`].
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheView {
    caches: bool,
    disc_ms: f64,
    reread_ms: f64,
    flush_kib: Option<u32>,
    /// What the rip reads without the test's measure.
    default_kib: u32,
}

/// Measures the drive's cache with the disc in the drive.
#[tauri::command]
async fn test_cache(state: State<'_, AppState>) -> CommandResult<CacheView> {
    let disc = state.disc.lock().unwrap().clone().ok_or(Message::new("noScan"))?;
    let state = Arc::clone(&state);
    blocking(move || {
        let reported = disc.capabilities.map_or(0, |c| u32::from(c.buffer_kib));
        let result = with_drive(&state, |drive| cache::test(drive, &disc.toc, reported, &mut |_| {}))?;
        Ok(CacheView {
            caches: result.caches,
            disc_ms: result.disc_read.as_secs_f64() * 1000.0,
            reread_ms: result.reread.as_secs_f64() * 1000.0,
            flush_kib: result.flush_kib,
            default_kib: reported.max(rip::MIN_CACHE_KIB),
        })
    })
    .await
}

/// Runs `work` on the drive, keeping rips and the drive watcher away meanwhile.
fn with_drive<T>(state: &Shared, work: impl FnOnce(&mut Drive) -> galette_core::Result<T>) -> CommandResult<T> {
    if state.ripping.swap(true, Ordering::SeqCst) {
        return Err(Message::new("alreadyRipping"));
    }
    let result = Drive::open(None).and_then(|mut drive| work(&mut drive));
    state.ripping.store(false, Ordering::SeqCst);
    Ok(result?)
}

#[tauri::command]
fn cancel_rip(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
async fn eject(state: State<'_, AppState>) -> CommandResult<()> {
    if state.ripping.load(Ordering::SeqCst) {
        return Err(Message::new("ejectWhileRipping"));
    }
    *state.disc.lock().unwrap() = None;
    blocking(|| Ok(Drive::open(None).and_then(|mut d| d.eject())?)).await
}

#[tauri::command]
fn open_folder(app: AppHandle, path: String) -> CommandResult<()> {
    app.opener().open_path(path, None::<&str>).map_err(|e| Message::with("openFolder", e))
}

/// Polls the drive and sends a `drive` event ("ready", "loading", "empty" or
/// "nodrive") whenever its state changes, so that inserted discs are read
/// without the user asking. Leaves the drive alone while it is in use.
fn watch_drive(app: AppHandle, state: AppState) {
    let mut last = None;
    loop {
        std::thread::sleep(Duration::from_millis(1500));
        if state.ripping.load(Ordering::SeqCst) || state.scanning.load(Ordering::SeqCst) {
            continue;
        }
        let now = match Drive::open(None).and_then(|mut d| d.media_state()) {
            Ok(MediaState::Ready) => "ready",
            Ok(MediaState::Loading) => "loading",
            Ok(MediaState::NoDisc) => "empty",
            Err(_) => "nodrive",
        };
        if last.is_some_and(|last| last != now) {
            if now != "ready" {
                *state.disc.lock().unwrap() = None;
            }
            let _ = app.emit("drive", now);
        }
        last = Some(now);
    }
}

/// Loads the settings, or makes them up on first launch.
fn load_prefs(app: &AppHandle, state: &Shared) {
    let path = app.path().app_config_dir().ok().map(|dir| dir.join("settings.json"));
    let mut prefs: Prefs = path
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    if prefs.output.as_os_str().is_empty() {
        let dirs = app.path();
        prefs.output = dirs.audio_dir().or_else(|_| dirs.home_dir()).unwrap_or_default();
    }
    *state.prefs.lock().unwrap() = prefs;
    *state.prefs_path.lock().unwrap() = path;
}

/// Runs blocking work (drive, network) off the window's threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> CommandResult<T> + Send + 'static) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| Message::with("internal", e))?
}

/// `m:ss`
fn duration(sectors: u32) -> String {
    let seconds = sectors / toc::SECTORS_PER_SECOND;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            let state = Arc::clone(&app.state::<AppState>());
            load_prefs(app.handle(), &state);
            let handle = app.handle().clone();
            std::thread::spawn(move || watch_drive(handle, state));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            formats,
            scan,
            cover,
            start_rip,
            cancel_rip,
            eject,
            open_folder,
            get_prefs,
            save_prefs,
            preview_naming,
            detect_offset,
            test_cache
        ])
        .run(tauri::generate_context!())
        .expect("cannot start Galette");
}
