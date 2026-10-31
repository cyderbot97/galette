//! Galette's ripping engine.
//!
//! This crate holds everything that is independent of the user interface:
//! talking to the CD drive, computing disc identifiers, fetching metadata,
//! extraction, AccurateRip verification and encoding.
//! The CLI and the future desktop GUI are thin layers on top of it.

#[cfg(target_os = "macos")]
pub mod aac;
pub mod accuraterip;
pub mod aiff;
pub mod alac;
pub mod cache;
pub mod cdtext;
pub mod ctdb;
pub mod cue;
pub mod drive;
pub mod error;
pub mod flac;
pub mod gaps;
mod http;
pub mod ids;
pub mod log;
mod m4a;
pub mod metadata;
pub mod mp3;
pub mod naming;
pub mod offset;
pub mod parity;
pub mod replaygain;
pub mod rip;
pub mod session;
pub mod tags;
pub mod toc;
pub mod wav;

pub use error::{Error, Result};
pub use toc::{Toc, Track};
