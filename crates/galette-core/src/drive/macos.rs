//! macOS transport: IOKit's SCSITask user client.
//!
//! macOS only lets a process send raw SCSI commands to an optical drive while
//! it holds *exclusive access* to it, which it refuses as long as a volume of
//! the disc is mounted (audio CDs are mounted by the cddafs file system).
//! Taking that access therefore means unmounting the disc through Disk
//! Arbitration first.
//!
//! Unmounting at every poll of the drive would be rude, so exclusive access is
//! taken lazily: TEST UNIT READY goes through the shared MMC interface, and the
//! first other command unmounts the disc and takes the drive. Dropping the
//! transport hands the drive back to the system.
//!
//! The IOKit interfaces are COM-style tables of function pointers, declared
//! below in the exact order of `<IOKit/scsi/SCSITaskLib.h>`.

use std::ffi::{CStr, CString, c_char, c_void};
use std::io;
use std::ptr;
use std::time::{Duration, Instant};

use super::{Transport, mmc, sense_error};
use crate::{Error, Result};

type IoReturn = i32;
type IoObject = u32;
type CfRef = *const c_void;

const IO_RETURN_SUCCESS: IoReturn = 0;
const TASK_STATUS_GOOD: u32 = 0x00;
const SERVICE_RESPONSE_TASK_COMPLETE: u32 = 2;
const DATA_TRANSFER_NONE: u8 = 0;
const DATA_TRANSFER_FROM_TARGET: u8 = 2;
const UNMOUNT_WHOLE: u32 = 0x1;
const REGISTRY_ITERATE_RECURSIVELY: u32 = 0x1;
const STRING_ENCODING_UTF8: u32 = 0x0800_0100;
/// `sizeof(SCSI_Sense_Data)`: fixed format sense data.
const SENSE_LEN: usize = 18;

/// Service classes of optical drives that publish the MMC user client.
const DRIVE_CLASSES: [&CStr; 3] = [c"IOBDServices", c"IODVDServices", c"IOCompactDiscServices"];

#[repr(C)]
#[derive(Clone, Copy)]
struct CfUuidBytes([u8; 16]);

/// `IUNKNOWN_C_GUTS`, the start of every interface.
#[repr(C)]
#[allow(dead_code)] // laid out like the C header, not every field is used
struct IUnknown {
    _reserved: *mut c_void,
    query_interface: unsafe extern "C" fn(*mut c_void, CfUuidBytes, *mut *mut c_void) -> i32,
    add_ref: unsafe extern "C" fn(*mut c_void) -> u32,
    release: unsafe extern "C" fn(*mut c_void) -> u32,
}

/// `IOCFPlugInInterface`; only its `IUnknown` part is used.
#[repr(C)]
#[allow(dead_code)] // laid out like the C header, not every field is used
struct PlugInInterface {
    base: IUnknown,
}

/// `MMCDeviceInterface`, up to the last function used.
#[repr(C)]
#[allow(dead_code)] // laid out like the C header, not every field is used
struct MmcDeviceInterface {
    base: IUnknown,
    version: u16,
    revision: u16,
    inquiry: *const c_void,
    test_unit_ready: unsafe extern "C" fn(*mut c_void, *mut u32, *mut u8) -> IoReturn,
    get_performance: *const c_void,
    get_configuration: *const c_void,
    mode_sense_10: *const c_void,
    set_write_parameters_mode_page: *const c_void,
    get_tray_state: *const c_void,
    set_tray_state: *const c_void,
    read_table_of_contents: *const c_void,
    read_disc_information: *const c_void,
    read_track_information: *const c_void,
    read_dvd_structure: *const c_void,
    get_scsi_task_device_interface: unsafe extern "C" fn(*mut c_void) -> Handle<ScsiTaskDeviceInterface>,
}

/// `SCSITaskDeviceInterface`.
#[repr(C)]
#[allow(dead_code)] // laid out like the C header, not every field is used
struct ScsiTaskDeviceInterface {
    base: IUnknown,
    version: u16,
    revision: u16,
    is_exclusive_access_available: unsafe extern "C" fn(*mut c_void) -> u8,
    add_callback_dispatcher_to_run_loop: *const c_void,
    remove_callback_dispatcher_from_run_loop: *const c_void,
    obtain_exclusive_access: unsafe extern "C" fn(*mut c_void) -> IoReturn,
    release_exclusive_access: unsafe extern "C" fn(*mut c_void) -> IoReturn,
    create_scsi_task: unsafe extern "C" fn(*mut c_void) -> Handle<ScsiTaskInterface>,
}

/// `SCSITaskInterface`, up to the last function used.
#[repr(C)]
#[allow(dead_code)] // laid out like the C header, not every field is used
struct ScsiTaskInterface {
    base: IUnknown,
    version: u16,
    revision: u16,
    is_task_active: *const c_void,
    set_task_attribute: *const c_void,
    get_task_attribute: *const c_void,
    set_command_descriptor_block: unsafe extern "C" fn(*mut c_void, *mut u8, u8) -> IoReturn,
    get_command_descriptor_block_size: *const c_void,
    get_command_descriptor_block: *const c_void,
    set_scatter_gather_entries: unsafe extern "C" fn(*mut c_void, *mut SgElement, u8, u64, u8) -> IoReturn,
    set_timeout_duration: unsafe extern "C" fn(*mut c_void, u32) -> IoReturn,
    get_timeout_duration: *const c_void,
    set_task_completion_callback: *const c_void,
    execute_task_async: *const c_void,
    execute_task_sync: unsafe extern "C" fn(*mut c_void, *mut u8, *mut u32, *mut u64) -> IoReturn,
    abort_task: *const c_void,
    get_scsi_service_response: unsafe extern "C" fn(*mut c_void, *mut u32) -> IoReturn,
}

/// `SCSITaskSGElement` (`IOAddressRange` on 64-bit systems).
#[repr(C)]
struct SgElement {
    address: u64,
    length: u64,
}

/// COM objects are handed out as pointers to a pointer to their function table.
type Handle<T> = *mut *const T;

/// Calls `$method` of the COM object `$obj`, passing the object itself first.
macro_rules! call {
    ($obj:expr, $($method:ident).+ $(, $arg:expr)*) => {
        ((**$obj).$($method).+)($obj.cast() $(, $arg)*)
    };
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: CfRef;
    fn CFRelease(cf: CfRef);
    fn CFGetTypeID(cf: CfRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringCreateWithCString(alloc: CfRef, s: *const c_char, encoding: u32) -> CfRef;
    fn CFStringGetCString(s: CfRef, buf: *mut c_char, len: isize, encoding: u32) -> u8;
    #[rustfmt::skip]
    fn CFUUIDGetConstantUUIDWithBytes(
        alloc: CfRef,
        b0: u8, b1: u8, b2: u8, b3: u8, b4: u8, b5: u8, b6: u8, b7: u8,
        b8: u8, b9: u8, b10: u8, b11: u8, b12: u8, b13: u8, b14: u8, b15: u8,
    ) -> CfRef;
    fn CFUUIDGetUUIDBytes(uuid: CfRef) -> CfUuidBytes;
    fn CFRunLoopGetCurrent() -> CfRef;
    fn CFRunLoopRunInMode(mode: CfRef, seconds: f64, return_after_source_handled: u8) -> i32;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CfRef;
    fn IOServiceGetMatchingServices(main_port: u32, matching: CfRef, iterator: *mut IoObject) -> IoReturn;
    fn IOIteratorNext(iterator: IoObject) -> IoObject;
    fn IOObjectRelease(object: IoObject) -> IoReturn;
    fn IORegistryEntrySearchCFProperty(
        entry: IoObject,
        plane: *const c_char,
        key: CfRef,
        alloc: CfRef,
        options: u32,
    ) -> CfRef;
    fn IOCreatePlugInInterfaceForService(
        service: IoObject,
        plugin_type: CfRef,
        interface_type: CfRef,
        interface: *mut Handle<PlugInInterface>,
        score: *mut i32,
    ) -> IoReturn;
    fn IODestroyPlugInInterface(interface: Handle<PlugInInterface>) -> IoReturn;
}

#[link(name = "DiskArbitration", kind = "framework")]
unsafe extern "C" {
    fn DASessionCreate(alloc: CfRef) -> CfRef;
    fn DASessionScheduleWithRunLoop(session: CfRef, run_loop: CfRef, mode: CfRef);
    fn DASessionUnscheduleFromRunLoop(session: CfRef, run_loop: CfRef, mode: CfRef);
    fn DADiskCreateFromBSDName(alloc: CfRef, session: CfRef, name: *const c_char) -> CfRef;
    fn DADiskUnmount(
        disk: CfRef,
        options: u32,
        callback: Option<unsafe extern "C" fn(CfRef, CfRef, *mut c_void)>,
        context: *mut c_void,
    );
    fn DADissenterGetStatus(dissenter: CfRef) -> i32;
}

#[rustfmt::skip]
fn uuid(b: [u8; 16]) -> CfRef {
    // SAFETY: returns a constant, never-released UUID object.
    unsafe {
        CFUUIDGetConstantUUIDWithBytes(
            ptr::null(),
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
            b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15],
        )
    }
}

/// `kIOMMCDeviceUserClientTypeID`
fn mmc_user_client_type() -> CfRef {
    uuid([0x97, 0xAB, 0xCF, 0x2C, 0x23, 0xCC, 0x11, 0xD5, 0xA0, 0xE8, 0x00, 0x30, 0x65, 0x70, 0x48, 0x66])
}

/// `kIOCFPlugInInterfaceID`
fn plugin_interface() -> CfRef {
    uuid([0xC2, 0x44, 0xE8, 0x58, 0x10, 0x9C, 0x11, 0xD4, 0x91, 0xD4, 0x00, 0x50, 0xE4, 0xC6, 0x42, 0x6F])
}

/// `kIOMMCDeviceInterfaceID`
fn mmc_device_interface() -> CfRef {
    uuid([0x1F, 0x65, 0x11, 0x06, 0x23, 0xCC, 0x11, 0xD5, 0xBB, 0xDB, 0x00, 0x30, 0x65, 0x70, 0x48, 0x66])
}

pub struct IoKitTransport {
    service: IoObject,
    plugin: Handle<PlugInInterface>,
    mmc: Handle<MmcDeviceInterface>,
    /// Set once exclusive access is held.
    device: Option<Handle<ScsiTaskDeviceInterface>>,
}

// SAFETY: IOKit user client interfaces may be used from any thread, and the
// transport is only ever used by one thread at a time (`&mut self`).
unsafe impl Send for IoKitTransport {}

impl IoKitTransport {
    /// Opens the drive holding the disc `device` ("disk4", "/dev/disk4"), or
    /// the first optical drive if `device` is `None`.
    pub fn open(device: Option<&str>) -> Result<Self> {
        let wanted = device.map(|d| {
            let name = d.strip_prefix("/dev/").unwrap_or(d);
            name.strip_prefix('r').filter(|n| n.starts_with("disk")).unwrap_or(name).to_owned()
        });
        let mut found = None;
        for service in drive_services() {
            if found.is_none() && wanted.as_ref().is_none_or(|w| bsd_name(service).as_ref() == Some(w)) {
                found = Some(service);
            } else {
                // SAFETY: `service` is an object we own and no longer use.
                unsafe { IOObjectRelease(service) };
            }
        }
        let Some(service) = found else {
            return Err(match device {
                Some(d) => io::Error::new(io::ErrorKind::NotFound, format!("no CD drive holds {d}")).into(),
                None => Error::NoDrive,
            });
        };

        let mut plugin: Handle<PlugInInterface> = ptr::null_mut();
        let mut score = 0;
        // SAFETY: valid service and UUIDs; `plugin` receives a new reference.
        let ret = unsafe {
            IOCreatePlugInInterfaceForService(
                service,
                mmc_user_client_type(),
                plugin_interface(),
                &mut plugin,
                &mut score,
            )
        };
        if ret != IO_RETURN_SUCCESS || plugin.is_null() {
            // SAFETY: we own `service`.
            unsafe { IOObjectRelease(service) };
            return Err(io_error(ret, "cannot open the CD drive"));
        }

        let mut mmc: Handle<MmcDeviceInterface> = ptr::null_mut();
        // SAFETY: `plugin` is a live interface; `mmc` receives a new reference.
        let ret = unsafe {
            call!(plugin, base.query_interface, CFUUIDGetUUIDBytes(mmc_device_interface()), (&raw mut mmc).cast())
        };
        if ret != 0 || mmc.is_null() {
            // SAFETY: both are ours and released once.
            unsafe {
                IODestroyPlugInInterface(plugin);
                IOObjectRelease(service);
            }
            return Err(io_error(ret, "the CD drive does not offer the MMC interface"));
        }

        Ok(Self { service, plugin, mmc, device: None })
    }

    /// Unmounts the disc and takes the drive for ourselves.
    fn obtain_exclusive_access(&mut self) -> Result<Handle<ScsiTaskDeviceInterface>> {
        if let Some(device) = self.device {
            return Ok(device);
        }
        // SAFETY: `self.mmc` is live; the returned interface is a new reference.
        let device = unsafe { call!(self.mmc, get_scsi_task_device_interface) };
        if device.is_null() {
            return Err(io_error(0, "cannot open the SCSI interface of the CD drive"));
        }
        // The disc may be (re)mounted between our unmount and the request,
        // and the kernel needs a moment after an unmount: try a few times.
        let mut ret = IO_RETURN_SUCCESS;
        for _ in 0..10 {
            if let Some(name) = bsd_name(self.service) {
                unmount(&name);
            }
            // SAFETY: `device` is a live interface.
            ret = unsafe { call!(device, obtain_exclusive_access) };
            if ret == IO_RETURN_SUCCESS {
                self.device = Some(device);
                return Ok(device);
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        // SAFETY: we own this reference.
        unsafe { call!(device, base.release) };
        Err(io_error(ret, "cannot take the CD drive: is another application (Music, Finder…) using the disc?"))
    }

    /// TEST UNIT READY without exclusive access.
    fn test_unit_ready(&mut self, opcode: u8) -> Result<usize> {
        let mut status = 0;
        let mut sense = [0u8; SENSE_LEN];
        // SAFETY: `self.mmc` is live; `status` and `sense` are large enough.
        let ret = unsafe { call!(self.mmc, test_unit_ready, &mut status, sense.as_mut_ptr()) };
        if ret != IO_RETURN_SUCCESS {
            return Err(Error::DriveNotResponding(format!("command 0x{opcode:02X}: IOKit error 0x{ret:08X}")));
        }
        if status != TASK_STATUS_GOOD {
            return Err(sense_error(opcode, &sense));
        }
        Ok(0)
    }
}

impl Transport for IoKitTransport {
    fn read(&mut self, cdb: &[u8], buf: &mut [u8], timeout: Duration) -> Result<usize> {
        if self.device.is_none() && cdb == mmc::test_unit_ready_cdb() {
            return self.test_unit_ready(cdb[0]);
        }
        let device = self.obtain_exclusive_access()?;

        // SAFETY: `device` is live; the task is released by `Task`'s Drop.
        let task = Task(unsafe { call!(device, create_scsi_task) });
        if task.0.is_null() {
            return Err(Error::DriveNotResponding(format!("command 0x{:02X}: cannot create a SCSI task", cdb[0])));
        }
        let t = task.0;
        let mut cdb_copy = cdb.to_vec();
        let mut sg = SgElement { address: buf.as_mut_ptr() as u64, length: buf.len() as u64 };
        let (entries, direction) =
            if buf.is_empty() { (0, DATA_TRANSFER_NONE) } else { (1, DATA_TRANSFER_FROM_TARGET) };
        let mut sense = [0u8; SENSE_LEN];
        let mut status = 0;
        let mut transferred = 0;
        let mut response = 0;
        // SAFETY: the CDB, scatter-gather entry and data buffer all outlive the
        // synchronous execution of the task.
        let ret = unsafe {
            call!(t, set_command_descriptor_block, cdb_copy.as_mut_ptr(), cdb.len() as u8);
            call!(t, set_scatter_gather_entries, &mut sg, entries, buf.len() as u64, direction);
            call!(t, set_timeout_duration, timeout.as_millis() as u32);
            let ret = call!(t, execute_task_sync, sense.as_mut_ptr(), &mut status, &mut transferred);
            call!(t, get_scsi_service_response, &mut response);
            ret
        };

        // A task that did not complete never reached the disc: the drive or
        // its USB bridge failed, or the drive was unplugged. A command that the
        // drive rejected completes, with a CHECK CONDITION status.
        if ret != IO_RETURN_SUCCESS || response != SERVICE_RESPONSE_TASK_COMPLETE {
            return Err(Error::DriveNotResponding(format!(
                "command 0x{:02X}: IOKit error 0x{ret:08X}, service response {response}",
                cdb[0]
            )));
        }
        if status != TASK_STATUS_GOOD {
            return Err(sense_error(cdb[0], &sense));
        }
        Ok(transferred as usize)
    }
}

impl Drop for IoKitTransport {
    fn drop(&mut self) {
        // SAFETY: every handle is ours and released exactly once, in the
        // reverse order of their creation.
        unsafe {
            if let Some(device) = self.device {
                call!(device, release_exclusive_access);
                call!(device, base.release);
            }
            call!(self.mmc, base.release);
            IODestroyPlugInInterface(self.plugin);
            IOObjectRelease(self.service);
        }
    }
}

/// A SCSI task, released when dropped.
struct Task(Handle<ScsiTaskInterface>);

impl Drop for Task {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: we own this reference.
            unsafe { call!(self.0, base.release) };
        }
    }
}

/// Every optical drive of the system. The caller owns the returned objects.
fn drive_services() -> Vec<IoObject> {
    let mut services = Vec::new();
    for class in DRIVE_CLASSES {
        let mut iterator = 0;
        // SAFETY: IOServiceGetMatchingServices consumes the matching dictionary.
        let ret = unsafe { IOServiceGetMatchingServices(0, IOServiceMatching(class.as_ptr()), &mut iterator) };
        if ret != IO_RETURN_SUCCESS {
            continue;
        }
        loop {
            // SAFETY: `iterator` is valid until released below.
            let service = unsafe { IOIteratorNext(iterator) };
            if service == 0 {
                break;
            }
            services.push(service);
        }
        // SAFETY: we own the iterator.
        unsafe { IOObjectRelease(iterator) };
    }
    services
}

/// BSD name ("disk4") of the disc in the drive, if there is one.
fn bsd_name(service: IoObject) -> Option<String> {
    // SAFETY: the key and the returned property are released here; the
    // property's type is checked before reading it as a string.
    unsafe {
        let key = CFStringCreateWithCString(ptr::null(), c"BSD Name".as_ptr(), STRING_ENCODING_UTF8);
        let value = IORegistryEntrySearchCFProperty(
            service,
            c"IOService".as_ptr(),
            key,
            ptr::null(),
            REGISTRY_ITERATE_RECURSIVELY,
        );
        CFRelease(key);
        if value.is_null() {
            return None;
        }
        let mut buf = [0 as c_char; 64];
        let ok = CFGetTypeID(value) == CFStringGetTypeID()
            && CFStringGetCString(value, buf.as_mut_ptr(), buf.len() as isize, STRING_ENCODING_UTF8) != 0;
        CFRelease(value);
        ok.then(|| CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned())
    }
}

/// Asks Disk Arbitration to unmount every volume of the disc `name`, and
/// waits for the answer. Failures are left to the exclusive access request.
fn unmount(name: &str) {
    let Ok(name) = CString::new(name) else { return };
    // Written by `unmounted` once Disk Arbitration answers.
    let mut outcome: Option<i32> = None;

    // SAFETY: the session and disk are released before returning; `outcome`
    // outlives the run loop iterations during which the callback can fire.
    unsafe {
        let session = DASessionCreate(ptr::null());
        if session.is_null() {
            return;
        }
        let disk = DADiskCreateFromBSDName(ptr::null(), session, name.as_ptr());
        if !disk.is_null() {
            let run_loop = CFRunLoopGetCurrent();
            DASessionScheduleWithRunLoop(session, run_loop, kCFRunLoopDefaultMode);
            DADiskUnmount(disk, UNMOUNT_WHOLE, Some(unmounted), (&raw mut outcome).cast());
            let deadline = Instant::now() + Duration::from_secs(10);
            while (&raw const outcome).read_volatile().is_none() && Instant::now() < deadline {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, 1);
            }
            DASessionUnscheduleFromRunLoop(session, run_loop, kCFRunLoopDefaultMode);
            CFRelease(disk);
        }
        CFRelease(session);
    }
}

unsafe extern "C" fn unmounted(_disk: CfRef, dissenter: CfRef, context: *mut c_void) {
    let status = if dissenter.is_null() { 0 } else { unsafe { DADissenterGetStatus(dissenter) } };
    // SAFETY: `context` is the `outcome` of `unmount`, still waiting for us.
    unsafe { context.cast::<Option<i32>>().write_volatile(Some(status)) };
}

fn io_error(ret: IoReturn, what: &str) -> Error {
    io::Error::other(format!("{what} (IOKit error 0x{ret:08X})")).into()
}
