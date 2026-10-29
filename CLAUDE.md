# Galette

Secure audio CD ripper, free software (GPL-3.0-or-later): EAC-quality ripping with an interface as simple as dBpoweramp CD Ripper. Recognizes the disc on MusicBrainz, rips securely, verifies with AccurateRip, writes tagged FLAC + cover + cue + log.

## Working with the user

- The user speaks French: answer in French. The UI, README and release notes are in French; code, comments and commit messages are in English.
- The user is new to Rust and is learning it through this project: explain choices briefly when they involve Rust concepts.
- **Always ask before pushing, tagging or publishing anything** (git push, GitHub releases, issues).
- Git identity is anonymous: `cyderbot97 <49147296+cyderbot97@users.noreply.github.com>`. Never send the user's personal email to any service (User-Agent, headers, etc.).
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Versioning: SemVer. 0.1.0 is published (Linux, pre-release). Patch = fixes, minor = new features (macOS, Windows, CTDB…). The version appears in `Cargo.toml` (workspace), `crates/galette-gui/tauri.conf.json` and the HTTP User-Agent in `galette-core/src/http.rs`.

## Layout

Cargo workspace, edition 2024:

- `crates/galette-core` — the engine (no UI).
  - `drive/` — SCSI/MMC over a `Transport` trait per OS. `mmc.rs` builds CDBs and parses replies (OS-independent); `linux.rs` is the SG_IO transport. **macOS (IOKit) and Windows (IOCTL_SCSI_PASS_THROUGH_DIRECT) transports are not written yet**: on other OSes `Drive::open` returns `Error::Unsupported`, everything else compiles.
  - `toc.rs`, `ids.rs` — TOC, MusicBrainz / FreeDB / AccurateRip disc IDs.
  - `metadata/` — MusicBrainz lookup, Cover Art Archive.
  - `accuraterip.rs` — CRC v1/v2, database fetch, drive offsets database.
  - `rip.rs` — burst and secure reading (C2, comparison passes, re-reads).
  - `gaps.rs` — pregap detection from the Q sub-channel (binary search).
  - `flac.rs`, `tags.rs`, `wav.rs`, `cue.rs`, `log.rs`, `naming.rs`.
  - `session.rs` — the whole rip of a disc, emitting `Event`s; used by both CLI and GUI.
- `crates/galette-cli` — binary `galette` (`galette info`, `galette rip`).
- `crates/galette-gui` — Tauri v2, binary `galette-gui`. Frontend is plain HTML/CSS/JS in `ui/` (no Node.js, `withGlobalTauri`). Settings are stored in the app config dir (`settings.json`). A thread polls the drive (TEST UNIT READY every 1.5 s) to detect disc insertion.

## Commands

```sh
cargo test
cargo clippy --all-targets
cargo run --release -p galette-gui
cargo run --release -p galette-cli -- info
cargo tauri build            # from crates/galette-gui: .deb + AppImage on Linux
```

## Hard-won knowledge (do not undo)

- **Open the drive read-write when possible** (fallback read-only): Linux refuses non-read commands such as eject on a read-only descriptor.
- **C2 pointers use the 296-byte format** (C2 + block error bits, READ CD byte 9 = 0x14). The 294-byte format hung an Initio USB bridge (13fd:0842, Samsung SE-S084C): multi-sector transfers came back truncated, then the bridge stopped answering until replugged.
- **Drive failure detection** (`linux.rs`): `host_status != 0` or a driver status other than 0 / DRIVER_SENSE (0x08) means `Error::DriveNotResponding` → abort the rip immediately (the GUI tells the user to unplug and replug). DRIVER_SENSE alone just means sense data is present (e.g. no disc) — treating it as a failure was a past bug.
- **flacenc writes a wrong STREAMINFO min block size**: `flac.rs` sets min = max block size. Without it libFLAC warns on `flac -t`.
- AccurateRip skips 5×588−1 samples at the disc edges; Enhanced CDs use data start − 11400 as the audio lead-out.
- Secure mode: if the burst pass is C2-clean and matches AccurateRip, stop; otherwise flush the cache, compare a second pass, re-read differing/C2 sectors (max 20 rounds, 3 identical clean reads to confirm). If the final audio matches AccurateRip, cleared positions are not reported as suspicious.
- Partial rips (some tracks only) write their log under a distinct name and no cue sheet, so they never overwrite a full rip's files.
- Tauri bundle category must be a valid Tauri value (`Music`); icon file names must be plain sizes (`256x256.png`, not `@2x`).

## Tested hardware

ASUS SDRW-08U9M-U and Samsung (TSSTcorp) SE-S084C over USB, both read offset +6. Debian KDE. Verified discs include rips matching AccurateRip 10/10 and 12/12, a damaged disc (suspicious sectors correctly reported), and discs absent from AccurateRip.

## Roadmap

- macOS transport (IOKit, SCSITaskDeviceInterface / exclusive access, which requires unmounting the disc via DiskArbitration) — next.
- Windows transport.
- CTDB verification and repair, HTOA, CD-Text, pre-emphasis.
