// src/ioav.rs
//! FFI bindings to Apple's private, undocumented IOAVService API, plus
//! public IOKit registry-enumeration calls used to find and list displays.
use std::ffi::{c_void, CString};
use std::os::raw::c_char;
use std::os::raw::{c_int, c_uint};

pub type IoavServiceRef = *mut c_void;
pub type IoReturn = c_int;
pub const KIO_RETURN_SUCCESS: IoReturn = 0;

#[allow(non_camel_case_types)]
type io_iterator_t = u32;
#[allow(non_camel_case_types)]
type io_service_t = u32;
#[allow(non_camel_case_types)]
type io_object_t = u32;
type MachPortT = u32;
type KernelReturnT = i32;

const MACH_PORT_NULL: MachPortT = 0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOAVServiceCreate(allocator: *const c_void) -> IoavServiceRef;
    fn IOAVServiceCreateWithService(allocator: *const c_void, service: io_service_t) -> IoavServiceRef;
    fn IOAVServiceWriteI2C(
        service: IoavServiceRef,
        chip_address: c_uint,
        data_address: c_uint,
        input_buffer: *const c_void,
        input_buffer_size: c_uint,
    ) -> IoReturn;
    fn IOAVServiceReadI2C(
        service: IoavServiceRef,
        chip_address: c_uint,
        offset: c_uint,
        output_buffer: *mut c_void,
        output_buffer_size: c_uint,
    ) -> IoReturn;

    // Public, documented IOKit registry-enumeration functions.
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingServices(
        master_port: MachPortT,
        matching: *mut c_void,
        existing: *mut io_iterator_t,
    ) -> KernelReturnT;
    fn IOIteratorNext(iterator: io_iterator_t) -> io_service_t;
    fn IOObjectRelease(object: io_object_t) -> KernelReturnT;
    fn IORegistryEntryGetRegistryEntryID(entry: io_service_t, entry_id: *mut u64) -> KernelReturnT;
}

pub const DDC_CHIP_ADDRESS: c_uint = 0x37;

/// Enumerates every `DCPAVServiceProxy` entry currently in the IORegistry
/// (one per AV-capable display on Apple Silicon). Caller owns the returned
/// handles and must IOObjectRelease each one once done with it.
fn enumerate_dcp_av_service_proxies() -> Vec<io_service_t> {
    let mut result = Vec::new();
    let Ok(name) = CString::new("DCPAVServiceProxy") else { return result };
    unsafe {
        let matching = IOServiceMatching(name.as_ptr());
        if matching.is_null() {
            return result;
        }
        let mut iterator: io_iterator_t = 0;
        // IOServiceGetMatchingServices consumes `matching` -- don't release it.
        if IOServiceGetMatchingServices(MACH_PORT_NULL, matching, &mut iterator) != 0 {
            return result;
        }
        loop {
            let service = IOIteratorNext(iterator);
            if service == 0 {
                break;
            }
            result.push(service);
        }
        IOObjectRelease(iterator);
    }
    result
}

/// Basic identifying info for a display found in the registry.
/// `registry_id` is stable across launches but not human-friendly on its
/// own -- cross-reference with `ioreg -c DCPAVServiceProxy -l` or System
/// Information if you need to confirm which physical display an index maps
/// to (product-name lookup isn't implemented here since I can't verify
/// which registry key holds it without real hardware to test against).
pub struct DisplayInfo {
    pub index: usize,
    pub registry_id: u64,
}

pub fn list_displays() -> Vec<DisplayInfo> {
    let services = enumerate_dcp_av_service_proxies();
    let mut out = Vec::with_capacity(services.len());
    for (index, service) in services.iter().enumerate() {
        let mut registry_id: u64 = 0;
        unsafe {
            IORegistryEntryGetRegistryEntryID(*service, &mut registry_id);
        }
        out.push(DisplayInfo { index, registry_id });
    }
    for service in services {
        unsafe { IOObjectRelease(service) };
    }
    out
}

pub struct AvService(IoavServiceRef);

impl AvService {
    /// Grabs a handle to the display at the given index (0 = first found).
    /// Falls back to the legacy `IOAVServiceCreate()` for index 0 if no
    /// DCP proxy is found at all (e.g. on Intel Macs).
    pub fn display_at_index(index: usize) -> Option<Self> {
        let services = enumerate_dcp_av_service_proxies();
        if services.is_empty() {
            if index != 0 {
                return None;
            }
            let svc = unsafe { IOAVServiceCreate(std::ptr::null()) };
            return if svc.is_null() { None } else { Some(Self(svc)) };
        }

        let mut chosen = None;
        for (i, service) in services.iter().enumerate() {
            if i == index {
                let svc = unsafe { IOAVServiceCreateWithService(std::ptr::null(), *service) };
                if !svc.is_null() {
                    chosen = Some(Self(svc));
                }
            }
        }
        for service in services {
            unsafe { IOObjectRelease(service) };
        }
        chosen
    }
    #[allow(dead_code)]
    pub fn default_display() -> Option<Self> {
        Self::display_at_index(0)
    }

    pub fn write_i2c(&self, data_address: u32, packet: &[u8]) -> Result<(), IoReturn> {
        let ret = unsafe {
            IOAVServiceWriteI2C(
                self.0,
                DDC_CHIP_ADDRESS,
                data_address,
                packet.as_ptr() as *const c_void,
                packet.len() as c_uint,
            )
        };
        if ret == KIO_RETURN_SUCCESS { Ok(()) } else { Err(ret) }
    }

    pub fn read_i2c(&self, offset: u32, len: usize) -> Result<Vec<u8>, IoReturn> {
        let mut buf = vec![0u8; len];
        let ret = unsafe {
            IOAVServiceReadI2C(
                self.0,
                DDC_CHIP_ADDRESS,
                offset,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as c_uint,
            )
        };
        if ret == KIO_RETURN_SUCCESS { Ok(buf) } else { Err(ret) }
    }
}
