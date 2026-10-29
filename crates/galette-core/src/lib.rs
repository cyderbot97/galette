//! Galette's ripping engine.
//!
//! This crate holds everything that is independent of the user interface:
//! talking to the CD drive, computing disc identifiers, fetching metadata and
//! (soon) secure extraction, AccurateRip verification and encoding.
//! The CLI and the future desktop GUI are thin layers on top of it.

pub mod accuraterip;
pub mod drive;
pub mod error;
mod http;
pub mod ids;
pub mod metadata;
pub mod rip;
pub mod toc;
pub mod wav;

pub use error::{Error, Result};
pub use toc::{Toc, Track};
