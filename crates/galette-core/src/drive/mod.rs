//! Access to the CD drive.
//!
//! Secure ripping needs raw access to the drive, below the operating system's
//! CD filesystem: we send MMC commands (the SCSI command set of optical drives)
//! ourselves. The commands are built and decoded once, in [`mmc`]; each
//! operating system only provides a [`Transport`] that delivers them.

pub mod mmc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

use std::time::Duration;

use crate::error::ScsiError;
use crate::toc::{SECTOR_BYTES, Toc};
use crate::{Error, Result};

/// Sends one SCSI command to a device and reads its answer into `buf`.
/// `Send` so that a rip can run in a background thread.
pub trait Transport: Send {
    /// Returns the number of bytes actually transferred. An empty `buf`
    /// sends a command that transfers no data.
    fn read(&mut self, cdb: &[u8], buf: &mut [u8], timeout: Duration) -> Result<usize>;
}

/// Whether there is a disc to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaState {
    Ready,
    /// A disc is being loaded, or was just changed.
    Loading,
    NoDisc,
}

/// Identity of a drive, as reported by INQUIRY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveInfo {
    pub vendor: String,
    pub product: String,
    pub revision: String,
}

pub struct Drive {
    transport: Box<dyn Transport>,
}

impl Drive {
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self { transport }
    }

    /// Opens the drive at `device`, or the platform's default drive.
    /// On macOS, `device` names the disc ("disk4") and may be empty.
    pub fn open(device: Option<&str>) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let transport = linux::SgTransport::open(device.unwrap_or(default_device()))?;
            Ok(Self::new(Box::new(transport)))
        }
        #[cfg(target_os = "macos")]
        {
            let transport = macos::IoKitTransport::open(device.filter(|d| !d.is_empty()))?;
            Ok(Self::new(Box::new(transport)))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = device;
            Err(Error::Unsupported)
        }
    }

    pub fn info(&mut self) -> Result<DriveInfo> {
        let mut buf = [0u8; mmc::INQUIRY_LEN];
        let len = self.transport.read(&mmc::inquiry_cdb(), &mut buf, Duration::from_secs(5))?;
        mmc::parse_inquiry(&buf[..len])
    }

    /// `None` if the drive does not describe its capabilities.
    pub fn capabilities(&mut self) -> Result<Option<mmc::Capabilities>> {
        let mut buf = [0u8; mmc::CAPABILITIES_LEN];
        let len = self.transport.read(&mmc::mode_sense_capabilities_cdb(), &mut buf, Duration::from_secs(5))?;
        Ok(mmc::parse_capabilities(&buf[..len]))
    }

    /// Asks the drive whether a disc is ready, with TEST UNIT READY: a
    /// command quick enough to be sent every second or two.
    pub fn media_state(&mut self) -> Result<MediaState> {
        match self.transport.read(&mmc::test_unit_ready_cdb(), &mut [], Duration::from_secs(5)) {
            Ok(_) => Ok(MediaState::Ready),
            Err(Error::Scsi(e)) if e.is_no_medium() => Ok(MediaState::NoDisc),
            // "Becoming ready", "medium may have changed"…
            Err(Error::Scsi(_)) => Ok(MediaState::Loading),
            Err(e) => Err(e),
        }
    }

    /// Unlocks the tray and opens it.
    pub fn eject(&mut self) -> Result<()> {
        self.transport.read(&mmc::allow_removal_cdb(), &mut [], Duration::from_secs(5))?;
        self.transport.read(&mmc::eject_cdb(), &mut [], Duration::from_secs(10))?;
        Ok(())
    }

    pub fn read_toc(&mut self) -> Result<Toc> {
        let mut buf = vec![0u8; mmc::TOC_BUFFER_LEN];
        let len = self.transport.read(&mmc::read_toc_cdb(buf.len()), &mut buf, Duration::from_secs(10))?;
        mmc::parse_toc(&buf[..len])
    }

    /// Position of the laser over `lba` according to the Q sub-channel, or
    /// `None` if this sector's Q frame does not carry a position.
    pub fn read_q(&mut self, lba: u32) -> Result<Option<mmc::QPosition>> {
        let mut buf = vec![0u8; SECTOR_BYTES + mmc::Q_LEN];
        let len = self.transport.read(&mmc::read_cd_q_cdb(lba), &mut buf, Duration::from_secs(10))?;
        Ok(buf.get(SECTOR_BYTES..len).and_then(mmc::parse_q))
    }

    /// Media Catalog Number of the disc, if it has one.
    pub fn read_mcn(&mut self) -> Result<Option<String>> {
        let mut buf = [0u8; mmc::SUB_CHANNEL_LEN];
        let len = self.transport.read(&mmc::read_mcn_cdb(), &mut buf, Duration::from_secs(10))?;
        Ok(mmc::parse_mcn(&buf[..len]))
    }

    /// ISRC of a track, if it has one.
    pub fn read_isrc(&mut self, track: u8) -> Result<Option<String>> {
        let mut buf = [0u8; mmc::SUB_CHANNEL_LEN];
        let len = self.transport.read(&mmc::read_isrc_cdb(track), &mut buf, Duration::from_secs(10))?;
        Ok(mmc::parse_isrc(&buf[..len]))
    }

    /// Reads `count` audio sectors starting at `lba` into `buf`
    /// (which must hold `count * SECTOR_BYTES` bytes).
    pub fn read_audio(&mut self, lba: u32, count: u32, buf: &mut [u8]) -> Result<()> {
        self.read_sectors(&mmc::read_cd_cdb(lba, count), count as usize * SECTOR_BYTES, buf)
    }

    /// Like [`Drive::read_audio`], each sector being followed by its
    /// [`mmc::C2_LEN`] bytes of C2 error pointers.
    pub fn read_audio_c2(&mut self, lba: u32, count: u32, buf: &mut [u8]) -> Result<()> {
        self.read_sectors(&mmc::read_cd_c2_cdb(lba, count), count as usize * (SECTOR_BYTES + mmc::C2_LEN), buf)
    }

    fn read_sectors(&mut self, cdb: &[u8], expected: usize, buf: &mut [u8]) -> Result<()> {
        assert_eq!(buf.len(), expected, "buffer size must match sector count");
        let len = self.transport.read(cdb, buf, Duration::from_secs(30))?;
        if len != expected {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("drive returned {len} bytes instead of {expected}"),
            )));
        }
        Ok(())
    }
}

pub fn default_device() -> &'static str {
    if cfg!(target_os = "linux") { "/dev/sr0" } else { "" }
}

/// Decodes fixed (0x70/0x71) or descriptor (0x72/0x73) format sense data.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
fn sense_error(opcode: u8, sense: &[u8]) -> Error {
    let byte = |i: usize| sense.get(i).copied().unwrap_or(0);
    let (sense_key, asc, ascq) = match byte(0) & 0x7F {
        0x72 | 0x73 => (byte(1) & 0x0F, byte(2), byte(3)),
        _ => (byte(2) & 0x0F, byte(12), byte(13)),
    };
    Error::Scsi(ScsiError { opcode, sense_key, asc, ascq })
}
