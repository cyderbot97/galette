use std::fmt;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Scsi(ScsiError),

    #[error("invalid TOC: {0}")]
    InvalidToc(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("this feature is not supported on this platform yet")]
    Unsupported,
}

/// A SCSI/MMC command rejected by the drive, decoded from its sense data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScsiError {
    pub opcode: u8,
    pub sense_key: u8,
    pub asc: u8,
    pub ascq: u8,
}

impl ScsiError {
    /// ASC 0x3A: "medium not present", the most common case for a user.
    pub fn is_no_medium(&self) -> bool {
        self.asc == 0x3A
    }
}

impl fmt::Display for ScsiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_no_medium() {
            return write!(f, "no disc in the drive");
        }
        write!(
            f,
            "drive rejected command 0x{:02X} (sense key 0x{:X}, ASC 0x{:02X}, ASCQ 0x{:02X})",
            self.opcode, self.sense_key, self.asc, self.ascq
        )
    }
}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::Network(e.to_string())
    }
}
