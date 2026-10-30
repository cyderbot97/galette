//! Windows transport: the IOCTL_SCSI_PASS_THROUGH_DIRECT control code, sent
//! to the volume of the drive (`\\.\D:`).
//!
//! Unlike macOS, Windows lets a process send raw commands while the disc is
//! mounted, and without administrator rights as long as the volume is opened
//! for reading and writing.

use std::ffi::c_void;
use std::io;
use std::mem::{offset_of, size_of};
use std::ptr;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDriveTypeW, GetLogicalDrives, OPEN_EXISTING,
};
use windows_sys::Win32::Storage::IscsiDisc::{
    IOCTL_SCSI_PASS_THROUGH_DIRECT, SCSI_IOCTL_DATA_IN, SCSI_IOCTL_DATA_UNSPECIFIED, SCSI_PASS_THROUGH_DIRECT,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_ADAPTER_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
    StorageAdapterProperty,
};

use super::{Transport, sense_error};
use crate::{Error, Result};

/// `GetDriveTypeW` result for optical drives.
const DRIVE_CDROM: u32 = 5;
const SENSE_LEN: usize = 32;

/// The command block followed by the buffer that receives sense data, as the
/// driver expects them: `SenseInfoOffset` points inside the same struct.
#[repr(C)]
struct PassThrough {
    sptd: SCSI_PASS_THROUGH_DIRECT,
    sense: [u8; SENSE_LEN],
}

pub struct PassThroughTransport {
    handle: HANDLE,
    /// The adapter refuses data buffers whose address has any of these bits
    /// set (0 for most drives).
    alignment_mask: usize,
    /// Scratch space for commands whose buffer is not aligned enough.
    bounce: Vec<u8>,
}

// SAFETY: a file handle may be used from any thread, and the transport is
// only ever used by one thread at a time (`&mut self`).
unsafe impl Send for PassThroughTransport {}

impl PassThroughTransport {
    /// Opens the drive `device` ("D", "D:", "\\.\D:"), or the first CD drive
    /// if `device` is `None`.
    pub fn open(device: Option<&str>) -> Result<Self> {
        let letter = match device {
            Some(d) => drive_letter(d)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("not a drive letter: {d}")))?,
            None => cd_drives().into_iter().next().ok_or(Error::NoDrive)?,
        };

        let path: Vec<u16> = format!(r"\\.\{letter}:").encode_utf16().chain([0]).collect();
        // Pass-through needs write access; fall back to read-only like Linux,
        // where at least reading commands may still go through.
        let open = |access: u32| {
            // SAFETY: `path` is a NUL-terminated UTF-16 string.
            unsafe {
                CreateFileW(
                    path.as_ptr(),
                    access,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    ptr::null(),
                    OPEN_EXISTING,
                    0,
                    ptr::null_mut(),
                )
            }
        };
        let mut handle = open(GENERIC_READ | GENERIC_WRITE);
        if handle == INVALID_HANDLE_VALUE {
            handle = open(GENERIC_READ);
        }
        if handle == INVALID_HANDLE_VALUE {
            let e = io::Error::last_os_error();
            return Err(io::Error::new(e.kind(), format!("cannot open drive {letter}: {e}")).into());
        }

        let alignment_mask = alignment_mask(handle);
        Ok(Self { handle, alignment_mask, bounce: Vec::new() })
    }
}

impl Transport for PassThroughTransport {
    fn read(&mut self, cdb: &[u8], buf: &mut [u8], timeout: Duration) -> Result<usize> {
        // Read into an aligned spot of `bounce` when `buf` is not aligned.
        let direct = buf.as_ptr() as usize & self.alignment_mask == 0;
        let data: *mut u8 = if direct {
            buf.as_mut_ptr()
        } else {
            self.bounce.resize(buf.len() + self.alignment_mask, 0);
            let start = self.bounce.as_ptr() as usize;
            let skip = (self.alignment_mask + 1 - (start & self.alignment_mask)) & self.alignment_mask;
            self.bounce[skip..].as_mut_ptr()
        };

        // SAFETY: an all-zero SCSI_PASS_THROUGH_DIRECT is valid (null buffer).
        let mut pt: PassThrough = unsafe { std::mem::zeroed() };
        pt.sptd.Length = size_of::<SCSI_PASS_THROUGH_DIRECT>() as u16;
        pt.sptd.CdbLength = cdb.len() as u8;
        pt.sptd.Cdb[..cdb.len()].copy_from_slice(cdb);
        pt.sptd.SenseInfoLength = SENSE_LEN as u8;
        pt.sptd.SenseInfoOffset = offset_of!(PassThrough, sense) as u32;
        pt.sptd.DataIn = if buf.is_empty() { SCSI_IOCTL_DATA_UNSPECIFIED } else { SCSI_IOCTL_DATA_IN } as u8;
        pt.sptd.DataTransferLength = buf.len() as u32;
        pt.sptd.DataBuffer = data.cast::<c_void>();
        pt.sptd.TimeOutValue = timeout.as_secs().max(1) as u32;

        let mut returned = 0;
        let pt_ptr = (&raw mut pt).cast::<c_void>();
        // SAFETY: `pt` is both input and output and outlives the synchronous
        // call; `DataBuffer` points to `buf.len()` writable bytes.
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_SCSI_PASS_THROUGH_DIRECT,
                pt_ptr,
                size_of::<PassThrough>() as u32,
                pt_ptr,
                size_of::<PassThrough>() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };

        // A failing call never reached the disc: the drive or its USB bridge
        // failed, or the drive was unplugged. A command that the drive
        // rejected succeeds, with a CHECK CONDITION status.
        if ok == 0 {
            return Err(Error::DriveNotResponding(format!(
                "command 0x{:02X}: {}",
                cdb[0],
                io::Error::last_os_error()
            )));
        }
        if pt.sptd.ScsiStatus != 0 {
            return Err(sense_error(cdb[0], &pt.sense));
        }
        let len = (pt.sptd.DataTransferLength as usize).min(buf.len());
        if !direct {
            // SAFETY: `data` points to at least `buf.len()` bytes of `bounce`.
            buf[..len].copy_from_slice(unsafe { std::slice::from_raw_parts(data, len) });
        }
        Ok(len)
    }
}

impl Drop for PassThroughTransport {
    fn drop(&mut self) {
        // SAFETY: we own the handle and close it once.
        unsafe { CloseHandle(self.handle) };
    }
}

/// Letters of every CD drive, in alphabetical order.
fn cd_drives() -> Vec<char> {
    // SAFETY: no arguments; returns a bit mask of drive letters.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| (b'A' + i) as char)
        .filter(|&letter| {
            let root: Vec<u16> = format!(r"{letter}:\").encode_utf16().chain([0]).collect();
            // SAFETY: `root` is a NUL-terminated UTF-16 string.
            unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_CDROM }
        })
        .collect()
}

/// "D", "d:", "D:\" or "\\.\D:" → 'D'.
fn drive_letter(device: &str) -> Option<char> {
    let name = device.strip_prefix(r"\\.\").unwrap_or(device);
    let name = name.trim_end_matches('\\').trim_end_matches(':');
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// The adapter's `AlignmentMask`, or 0 if it does not tell.
fn alignment_mask(handle: HANDLE) -> usize {
    // SAFETY: both structs are plain data, valid when zeroed.
    let mut query: STORAGE_PROPERTY_QUERY = unsafe { std::mem::zeroed() };
    query.PropertyId = StorageAdapterProperty;
    query.QueryType = PropertyStandardQuery;
    let mut desc: STORAGE_ADAPTER_DESCRIPTOR = unsafe { std::mem::zeroed() };
    let mut returned = 0;
    // SAFETY: both buffers are live and of the declared sizes.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&raw const query).cast(),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            (&raw mut desc).cast(),
            size_of::<STORAGE_ADAPTER_DESCRIPTOR>() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    let covered = returned as usize >= offset_of!(STORAGE_ADAPTER_DESCRIPTOR, AlignmentMask) + size_of::<u32>();
    if ok != 0 && covered { desc.AlignmentMask as usize } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::drive_letter;

    #[test]
    fn drive_letters() {
        assert_eq!(drive_letter("D"), Some('D'));
        assert_eq!(drive_letter("e:"), Some('E'));
        assert_eq!(drive_letter(r"D:\"), Some('D'));
        assert_eq!(drive_letter(r"\\.\F:"), Some('F'));
        assert_eq!(drive_letter("/dev/sr0"), None);
        assert_eq!(drive_letter(""), None);
    }
}
