//! Access to the CD drive.
//!
//! Secure ripping needs raw access to the drive, below the operating system's
//! CD filesystem: we send MMC commands (the SCSI command set of optical drives)
//! ourselves. The commands are built and decoded once, in [`mmc`]; each
//! operating system only provides a [`Transport`] that delivers them.

pub mod mmc;

#[cfg(target_os = "linux")]
mod linux;

use std::time::Duration;

use crate::toc::{SECTOR_BYTES, Toc};
use crate::{Error, Result};

/// Sends one SCSI command to a device and reads its answer into `buf`.
pub trait Transport {
    /// Returns the number of bytes actually transferred.
    fn read(&mut self, cdb: &[u8], buf: &mut [u8], timeout: Duration) -> Result<usize>;
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
    pub fn open(device: Option<&str>) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let transport = linux::SgTransport::open(device.unwrap_or(default_device()))?;
            Ok(Self::new(Box::new(transport)))
        }
        #[cfg(not(target_os = "linux"))]
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

    pub fn read_toc(&mut self) -> Result<Toc> {
        let mut buf = vec![0u8; mmc::TOC_BUFFER_LEN];
        let len = self.transport.read(&mmc::read_toc_cdb(buf.len()), &mut buf, Duration::from_secs(10))?;
        mmc::parse_toc(&buf[..len])
    }

    /// Reads `count` audio sectors starting at `lba` into `buf`
    /// (which must hold `count * SECTOR_BYTES` bytes).
    ///
    /// This is a single raw read: no offset correction and no re-reads yet.
    pub fn read_audio(&mut self, lba: u32, count: u32, buf: &mut [u8]) -> Result<()> {
        let expected = count as usize * SECTOR_BYTES;
        assert_eq!(buf.len(), expected, "buffer size must match sector count");
        let len = self.transport.read(&mmc::read_cd_cdb(lba, count), buf, Duration::from_secs(30))?;
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
