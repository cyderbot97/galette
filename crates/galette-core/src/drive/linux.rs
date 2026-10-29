//! Linux transport: the SG_IO ioctl, available on every `/dev/sr*` device.
//! See <https://tldp.org/HOWTO/SCSI-Generic-HOWTO/sg_io_hdr_t.html>.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

use super::Transport;
use crate::error::ScsiError;
use crate::{Error, Result};

const SG_IO: libc::c_ulong = 0x2285;
const SG_DXFER_NONE: libc::c_int = -1;
const SG_DXFER_FROM_DEV: libc::c_int = -3;
const SENSE_LEN: usize = 32;

/// `struct sg_io_hdr` from `<scsi/sg.h>`.
#[repr(C)]
struct SgIoHdr {
    interface_id: libc::c_int,
    dxfer_direction: libc::c_int,
    cmd_len: libc::c_uchar,
    mx_sb_len: libc::c_uchar,
    iovec_count: libc::c_ushort,
    dxfer_len: libc::c_uint,
    dxferp: *mut libc::c_void,
    cmdp: *const libc::c_uchar,
    sbp: *mut libc::c_uchar,
    timeout: libc::c_uint,
    flags: libc::c_uint,
    pack_id: libc::c_int,
    usr_ptr: *mut libc::c_void,
    status: libc::c_uchar,
    masked_status: libc::c_uchar,
    msg_status: libc::c_uchar,
    sb_len_wr: libc::c_uchar,
    host_status: libc::c_ushort,
    driver_status: libc::c_ushort,
    resid: libc::c_int,
    duration: libc::c_uint,
    info: libc::c_uint,
}

pub struct SgTransport {
    file: File,
}

impl SgTransport {
    pub fn open(path: &str) -> Result<Self> {
        // O_NONBLOCK lets us open the device even when the tray is empty or open.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| io::Error::new(e.kind(), format!("cannot open {path}: {e}")))?;
        Ok(Self { file })
    }
}

impl Transport for SgTransport {
    fn read(&mut self, cdb: &[u8], buf: &mut [u8], timeout: Duration) -> Result<usize> {
        let mut sense = [0u8; SENSE_LEN];
        let mut hdr = SgIoHdr {
            interface_id: b'S' as libc::c_int,
            dxfer_direction: if buf.is_empty() { SG_DXFER_NONE } else { SG_DXFER_FROM_DEV },
            cmd_len: cdb.len() as u8,
            mx_sb_len: SENSE_LEN as u8,
            iovec_count: 0,
            dxfer_len: buf.len() as u32,
            dxferp: buf.as_mut_ptr().cast(),
            cmdp: cdb.as_ptr(),
            sbp: sense.as_mut_ptr(),
            timeout: timeout.as_millis() as u32,
            flags: 0,
            pack_id: 0,
            usr_ptr: std::ptr::null_mut(),
            status: 0,
            masked_status: 0,
            msg_status: 0,
            sb_len_wr: 0,
            host_status: 0,
            driver_status: 0,
            resid: 0,
            duration: 0,
            info: 0,
        };

        // SAFETY: every pointer in `hdr` refers to a live buffer of the declared
        // length, and they all outlive the call.
        let ret = unsafe { libc::ioctl(self.file.as_raw_fd(), SG_IO, &mut hdr) };
        if ret < 0 {
            return Err(io::Error::last_os_error().into());
        }

        if hdr.status != 0 || hdr.host_status != 0 || hdr.driver_status & 0x0F != 0 {
            return Err(sense_error(cdb[0], &sense[..hdr.sb_len_wr as usize]));
        }
        Ok(buf.len() - hdr.resid.max(0) as usize)
    }
}

/// Decodes fixed (0x70/0x71) or descriptor (0x72/0x73) format sense data.
fn sense_error(opcode: u8, sense: &[u8]) -> Error {
    let byte = |i: usize| sense.get(i).copied().unwrap_or(0);
    let (sense_key, asc, ascq) = match byte(0) & 0x7F {
        0x72 | 0x73 => (byte(1) & 0x0F, byte(2), byte(3)),
        _ => (byte(2) & 0x0F, byte(12), byte(13)),
    };
    Error::Scsi(ScsiError { opcode, sense_key, asc, ascq })
}
