//! Galette's desktop application: a Tauri window whose page (in `ui/`) calls
//! the commands below. Everything slow runs in a background thread, so the
//! window stays responsive; rips report their progress as `rip` events.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use galette_core::accuraterip::Verdict;
use galette_core::drive::Drive;
use galette_core::metadata::Release;
use galette_core::metadata::coverart::{self, CoverArt};
use galette_core::rip::{self, Progress};
use galette_core::session::{self, Disc, Event, Format};
use galette_core::{ids, log, toc};

#[derive(Default)]
struct Shared {
    disc: Mutex<Option<Disc>>,
    /// Covers already downloaded, by release id.
    covers: Mutex<HashMap<String, Option<CoverArt>>>,
    ripping: AtomicBool,
    cancel: Arc<AtomicBool>,
}

type AppState = Arc<Shared>;

/// Errors are shown to the user as they are.
type CommandResult<T> = Result<T, String>;

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
    warning: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackView {
    number: u8,
    length: String,
    is_audio: bool,
}

/// Reads the disc in the drive and looks it up online.
#[tauri::command]
async fn scan(state: State<'_, AppState>) -> CommandResult<DiscView> {
    let state = Arc::clone(&state);
    blocking(move || {
        let mut drive = Drive::open(None).map_err(|e| e.to_string())?;
        let mut disc = Disc::read(&mut drive).map_err(|e| e.to_string())?;
        let warning = disc.lookup().err().map(|e| format!("Recherche en ligne impossible : {e}"));
        let view = DiscView {
            drive: format!("{} {}", disc.drive.vendor, disc.drive.product),
            c2: disc.c2(),
            offset: disc.offset,
            disc_id: ids::musicbrainz(&disc.toc),
            accuraterip: disc.accuraterip.as_ref().map(Vec::len),
            tracks: disc
                .toc
                .tracks
                .iter()
                .enumerate()
                .map(|(i, t)| TrackView {
                    number: t.number,
                    length: duration(disc.toc.track_sectors(i)),
                    is_audio: t.is_audio,
                })
                .collect(),
            releases: disc.releases.clone(),
            warning,
        };
        *state.disc.lock().unwrap() = Some(disc);
        Ok(view)
    })
    .await
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
    output: PathBuf,
    format: Format,
    secure: bool,
    offset: Option<i32>,
}

/// What the page receives while ripping.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum RipEvent {
    Status { message: String },
    Warning { message: String },
    Progress { number: u8, phase: &'static str, fraction: f32, detail: String },
    TrackDone { number: u8, status: &'static str, text: String, read: String },
    Finished { album_dir: PathBuf, accurate: usize, clean: usize, total: usize, verified: bool },
    Failed { message: String },
    Cancelled,
}

/// Starts ripping in the background; progress comes as `rip` events.
#[tauri::command]
fn start_rip(app: AppHandle, state: State<'_, AppState>, request: RipRequest) -> CommandResult<()> {
    let disc = state.disc.lock().unwrap().clone().ok_or("Aucun disque analysé")?;
    let offset = request.offset.or(disc.offset).ok_or(
        "Ce lecteur n'est pas dans la base d'AccurateRip : indiquez son décalage de lecture.",
    )?;
    if state.ripping.swap(true, Ordering::SeqCst) {
        return Err("Un rip est déjà en cours".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let cover = match (&request.release, request.format) {
        (Some(r), Format::Flac) => state.covers.lock().unwrap().get(&r.id).cloned().flatten(),
        _ => None,
    };
    let settings = session::Settings {
        output: request.output,
        format: request.format,
        mode: if request.secure { rip::Mode::Secure } else { rip::Mode::Burst },
        offset,
        release: request.release,
        cover,
        tracks: request.tracks,
        cancel: Some(Arc::clone(&state.cancel)),
    };

    let state = Arc::clone(&state);
    std::thread::spawn(move || {
        let emit = |event: RipEvent| {
            let _ = app.emit("rip", event);
        };
        let result = Drive::open(None).and_then(|mut drive| {
            session::rip(&mut drive, &disc, &settings, &mut |event| {
                if let Some(event) = rip_event(event) {
                    emit(event);
                }
            })
        });
        state.ripping.store(false, Ordering::SeqCst);
        emit(match result {
            Ok(outcome) => RipEvent::Finished {
                accurate: outcome.accurate(),
                clean: outcome.tracks.iter().filter(|t| t.read.is_clean()).count(),
                total: outcome.tracks.len(),
                verified: disc.accuraterip.is_some(),
                album_dir: outcome.album_dir,
            },
            Err(galette_core::Error::Cancelled) => RipEvent::Cancelled,
            Err(galette_core::Error::DriveNotResponding(_)) => RipEvent::Failed {
                message: "le lecteur ne répond plus. Débranchez-le, rebranchez-le, puis cliquez sur Actualiser.".into(),
            },
            Err(e) => RipEvent::Failed { message: e.to_string() },
        });
    });
    Ok(())
}

fn rip_event(event: Event) -> Option<RipEvent> {
    Some(match event {
        Event::ReadingSubcode => RipEvent::Status { message: "Analyse du disque (pré-gaps, ISRC)…".into() },
        Event::Warning(message) => RipEvent::Warning { message },
        Event::TrackStarted { number, .. } => {
            RipEvent::Progress { number, phase: "reading", fraction: 0.0, detail: "Lecture".into() }
        }
        Event::TrackProgress { number, progress } => match progress {
            Progress::Reading(fraction) => {
                RipEvent::Progress { number, phase: "reading", fraction, detail: "Lecture".into() }
            }
            Progress::Verifying(fraction) => {
                RipEvent::Progress { number, phase: "verifying", fraction, detail: "Seconde lecture".into() }
            }
            Progress::Rereading { attempt, sectors } => RipEvent::Progress {
                number,
                phase: "rereading",
                fraction: attempt as f32 / rip::MAX_REREADS as f32,
                detail: format!("Relecture de {sectors} secteur(s), {attempt}/{}", rip::MAX_REREADS),
            },
        },
        Event::Encoding { number } => {
            RipEvent::Progress { number, phase: "encoding", fraction: 1.0, detail: "Encodage".into() }
        }
        Event::TrackDone(track) => {
            let (status, text) = match (track.verdict, track.read.is_clean()) {
                (Verdict::Accurate { confidence, .. }, _) => ("accurate", format!("AccurateRip ✓ ({confidence})")),
                (Verdict::Mismatch, _) => ("bad", "Non conforme à AccurateRip".to_string()),
                (Verdict::Unknown, true) => ("clean", "Lectures identiques ✓".to_string()),
                (Verdict::Unknown, false) => ("bad", "Erreurs de lecture".to_string()),
            };
            RipEvent::TrackDone { number: track.number, status, text, read: log::read_summary(&track.read) }
        }
        Event::Subcode { .. } => RipEvent::Status { message: String::new() },
    })
}

#[tauri::command]
fn cancel_rip(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[tauri::command]
async fn eject(state: State<'_, AppState>) -> CommandResult<()> {
    if state.ripping.load(Ordering::SeqCst) {
        return Err("Impossible d'éjecter pendant un rip".into());
    }
    *state.disc.lock().unwrap() = None;
    blocking(|| Drive::open(None).and_then(|mut d| d.eject()).map_err(|e| e.to_string())).await
}

#[tauri::command]
fn default_output(app: AppHandle) -> PathBuf {
    let path = app.path();
    path.audio_dir().or_else(|_| path.home_dir()).unwrap_or_default()
}

#[tauri::command]
fn open_folder(app: AppHandle, path: String) -> CommandResult<()> {
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

/// Runs blocking work (drive, network) off the window's threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> CommandResult<T> + Send + 'static) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
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
        .invoke_handler(tauri::generate_handler![
            scan,
            cover,
            start_rip,
            cancel_rip,
            eject,
            default_output,
            open_folder
        ])
        .run(tauri::generate_context!())
        .expect("cannot start Galette");
}
