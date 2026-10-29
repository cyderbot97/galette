//! Galette's ripping engine.
//!
//! This crate holds everything that is independent of the user interface:
//! talking to the CD drive, computing disc identifiers, fetching metadata,
//! extraction, AccurateRip verification and encoding.
//! The CLI and the future desktop GUI are thin layers on top of it.

pub mod accuraterip;
pub mod drive;
pub mod error;
pub mod flac;
mod http;
pub mod ids;
pub mod metadata;
pub mod rip;
pub mod tags;
pub mod toc;
pub mod wav;

pub use error::{Error, Result};
pub use toc::{Toc, Track};
