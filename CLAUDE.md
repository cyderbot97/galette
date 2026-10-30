# Galette

Secure audio CD ripper, free software (GPL-3.0-or-later): EAC-quality ripping with an interface as simple as dBpoweramp CD Ripper. Recognizes the disc on MusicBrainz, rips securely, verifies with AccurateRip, writes tagged FLAC + cover + cue + log.

## Working with the user

- The user speaks French: answer in French. The UI, README and release notes are in French; code, comments and commit messages are in English.
- The user is new to Rust and is learning it through this project: explain choices briefly when they involve Rust concepts.
- **Always ask before pushing, tagging or publishing anything** (git push, GitHub releases, issues).
- Git identity is anonymous: `cyderbot97 <49147296+cyderbot97@users.noreply.github.com>`. Never send the user's personal email to any service (User-Agent, headers, etc.).
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Versioning: SemVer. 0.1.0 is published (Linux, pre-release); 0.2.0 adds macOS; 0.4.0 adds Windows (Windows-only release: Linux and macOS packages stay at 0.3.0, the code is otherwise identical). Patch = fixes, minor = new features (macOS, Windows, CTDB…). The version appears in `Cargo.toml` (workspace), `crates/galette-gui/tauri.conf.json` and the HTTP User-Agent in `galette-core/src/http.rs`.

## Layout

Cargo workspace, edition 2024:

- `crates/galette-core` — the engine (no UI).
  - `drive/` — SCSI/MMC over a `Transport` trait per OS. `mmc.rs` builds CDBs and parses replies (OS-independent); `linux.rs` is the SG_IO transport; `macos.rs` is the IOKit SCSITask transport (hand-written FFI, no extra crate); `windows.rs` is the IOCTL_SCSI_PASS_THROUGH_DIRECT transport (`windows-sys`).
  - `toc.rs`, `ids.rs` — TOC, MusicBrainz / FreeDB / AccurateRip disc IDs.
  - `metadata/` — MusicBrainz lookup, Cover Art Archive. `http.rs` retries 503/429 answers (Retry-After, else 1-2-4-8 s).
  - `accuraterip.rs` — CRC v1/v2, database fetch, drive offsets database.
  - `rip.rs` — burst and secure reading (C2, comparison passes, re-reads).
  - `gaps.rs` — pregap detection from the Q sub-channel (binary search).
  - Output formats: `flac.rs`, `alac.rs` (alac-encoder, verified by decoding with the `alac` crate; `m4a.rs` is our own minimal MP4 muxer, shared with AAC; tags added by mp4ameta), `aac.rs` (macOS only, `#[cfg]`: Apple's AudioToolbox AudioConverter via hand-written FFI, 256 kbit/s constrained VBR; priming frames declared by an `elst` edit list and the `iTunSMPB` tag for gapless playback; `Format::ALL` tells the GUI which formats exist so AAC is not offered elsewhere), `mp3.rs` (LAME, bundled C source via mp3lame-sys; the LAME tag is written over the first frame for gapless playback), `aiff.rs`, `wav.rs`. `session::Format` lists them; `session::encode` dispatches. `tags.rs` builds Vorbis comments (Picard names) and converts them to ID3v2.4 for MP3/AIFF and to iTunes atoms for .m4a.
  - `cue.rs`, `log.rs`, `naming.rs`.
  - `session.rs` — the whole rip of a disc, emitting `Event`s; used by both CLI and GUI.
- `crates/galette-cli` — binary `galette` (`galette info`, `galette rip`).
- `crates/galette-gui` — Tauri v2, binary `galette-gui`. Frontend is plain HTML/CSS/JS in `ui/` (no Node.js, `withGlobalTauri`). Settings are stored in the app config dir (`settings.json`). Optional themes (`Prefs.theme`, chosen in the settings) live in `ui/themes/<name>/` as CSS + SVG over the same layout: they only change colors, fonts, background and logos, never the organization. Only `cathedrale` is kept; rejected themes are archived in the git-ignored `attic/`. **Languages** (French, English, or the system's): every text of the window is in `ui/i18n.js`; elements carry `data-i18n` keys so that a change of language rewrites them all. The Rust side never sends sentences to the window, only codes (`Message { code, detail }`, progress phases, read counts) that the window translates; engine errors and `session::Warning` map to codes in `src/main.rs`. Logs and the CLI stay in English. A thread polls the drive (TEST UNIT READY every 1.5 s) to detect disc insertion.

## Commands

```sh
cargo test
cargo clippy --all-targets
cargo run --release -p galette-gui
cargo run --release -p galette-cli -- info
cargo tauri build            # from crates/galette-gui: .deb + AppImage on Linux, .app + .dmg on macOS, NSIS .exe on Windows
```

On macOS, build outside `~/Documents` (see below): `export CARGO_TARGET_DIR=~/.cache/galette-target`.

## Hard-won knowledge (do not undo)

- **Open the drive read-write when possible** (fallback read-only): Linux refuses non-read commands such as eject on a read-only descriptor.
- **C2 pointers use the 296-byte format** (C2 + block error bits, READ CD byte 9 = 0x14). The 294-byte format hung an Initio USB bridge (13fd:0842, Samsung SE-S084C): multi-sector transfers came back truncated, then the bridge stopped answering until replugged.
- **Drive failure detection** (`linux.rs`): `host_status != 0` or a driver status other than 0 / DRIVER_SENSE (0x08) means `Error::DriveNotResponding` → abort the rip immediately (the GUI tells the user to unplug and replug). DRIVER_SENSE alone just means sense data is present (e.g. no disc) — treating it as a failure was a past bug.
- **flacenc writes a wrong STREAMINFO min block size**: `flac.rs` sets min = max block size. Without it libFLAC warns on `flac -t`.
- AccurateRip skips 5×588−1 samples at the disc edges; Enhanced CDs use data start − 11400 as the audio lead-out.
- Secure mode: if the burst pass is C2-clean and matches AccurateRip, stop; otherwise flush the cache, compare a second pass, re-read differing/C2 sectors (max 20 rounds, 3 identical clean reads to confirm). If the final audio matches AccurateRip, cleared positions are not reported as suspicious.
- Partial rips (some tracks only) write their log under a distinct name and no cue sheet, so they never overwrite a full rip's files.
- A rip never replaces a rip in another format: if the album folder holds a log whose "Output format" names another format, the rip goes to "<folder> (FORMAT)" (`session::album_dir`). ALAC and AAC files share the .m4a extension, and every format's log and cue sheet share their names.
- **macOS: exclusive access is lazy.** Raw SCSI commands need `ObtainExclusiveAccess`, refused while the disc is mounted (cddafs), so `macos.rs` unmounts it via Disk Arbitration first. TEST UNIT READY goes through the shared `MMCDeviceInterface` instead, so the GUI's 1.5 s polling never unmounts the disc. Dropping the transport releases the drive and macOS remounts the disc by itself.
- **macOS: a binary under `~/Documents` cannot open the drive** (`IOCreatePlugInInterfaceForService` → 0xE00002BE kIOReturnNoResources): opening the MMC user client makes `authd` check the binary, and TCC forbids authd from reading `~/Documents`. Hence `CARGO_TARGET_DIR` elsewhere. An app in /Applications is not affected.
- **macOS: IOKit vtables must match `SCSITaskLib.h` slot by slot.** A first version missed `GetCommandDescriptorBlockSize` and `GetTimeoutDuration` (not returning IOReturn, so a grep skipped them): every call landed on the wrong function, silently. Check offsets with a C `offsetof` program when touching them.
- **macOS: `GetSCSIServiceResponse` must be checked** after `ExecuteTaskSync`: not TASK_COMPLETE → `DriveNotResponding`; a rejected command completes with CHECK CONDITION status and sense data.
- macOS bundle settings live in `tauri.macos.conf.json` (merged over `tauri.conf.json`): targets app + dmg, `icon.icns`, and `signingIdentity: "-"` so the whole bundle is ad-hoc signed (otherwise only the binary is, and a downloaded app is reported as damaged).
- Tauri bundle category must be a valid Tauri value (`Music`); icon file names must be plain sizes (`256x256.png`, not `@2x`).

## Windows

- **Build setup**: Rust with the MSVC toolchain (rustup default), Visual Studio Build Tools with "Desktop development with C++" (the C compiler builds LAME via mp3lame-sys, and ring), `cargo install tauri-cli`. WebView2 ships with Windows 10/11. `CARGO_TARGET_DIR` is a macOS-only workaround, not needed here.
- **Smart App Control blocks the build**: with it on, Windows refuses to run unsigned, unknown executables, and randomly kills Cargo build scripts (os error 4551, "Une stratégie de contrôle d'application a bloqué ce fichier"). It must be turned off to build. It will also block the unsigned installer on users' machines; SmartScreen merely warns (README says so).
- `drive/windows.rs`: `Drive::open(None)` takes the first `DRIVE_CDROM` letter; `--device` accepts `D`, `D:`, `\\.\D:`. Opens `\\.\D:` read-write (fallback read-only). The data buffer honors the adapter's `AlignmentMask` through a bounce buffer. A failing `DeviceIoControl` → `DriveNotResponding`; non-zero `ScsiStatus` → sense data.
- Verified: no admin rights needed; pass-through works while Windows has the disc mounted (no unmount); the GUI's 1.5 s polling (one handle opened per poll) is harmless; a full secure rip matched AccurateRip 12/12.
- `CreateFileW` needs the `Win32_Security` feature of `windows-sys` (its SECURITY_ATTRIBUTES parameter).
- Ready from the Mac side: `tauri.windows.conf.json` (NSIS installer, `icons/icon.ico`); file names sanitized for Windows (`session::sanitize`); AAC hidden; output folder defaults to the user's Music folder.

## Tested hardware

ASUS SDRW-08U9M-U and Samsung (TSSTcorp) SE-S084C over USB, both read offset +6. Debian KDE; macOS 27 on Apple Silicon (ASUS, accurate rip with C2, ISRC, MCN); Windows 11 (ASUS, accurate rip 12/12 in secure mode, ISRC, MCN, pregaps). Verified discs include rips matching AccurateRip 10/10 and 12/12, a damaged disc (suspicious sectors correctly reported), and discs absent from AccurateRip.

## Roadmap

`TODO.md` (French, for the user) lists postponed work and things the user has not tested yet: keep it up to date.

- Grow the app on Linux + macOS (Apple Silicon) first. No Intel Mac build planned for now (user's decision).
- Windows: works, see "Windows" above; some GUI features still to check (TODO.md).
- Output formats are complete for now. Opus and WavPack: user decided against them (WavPack's reference library was too much hassle to bundle). AAC is macOS only, option hidden on other OSes (user's requirement).
- CTDB verification and repair (protocol already studied: lookup2.php, RS over GF(2^16) poly 0x1100B, stride 5880 samples; user postponed it as too complex for now), HTOA, CD-Text, pre-emphasis.
