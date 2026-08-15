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
// Standard I2C address for reading a display's EDID over DDC/CI. This is a
// VESA-standardized address (not Apple-private), so it's the same on every
// DDC-capable monitor regardless of vendor.
pub const EDID_CHIP_ADDRESS: c_uint = 0x50;
const EDID_LEN: usize = 128;
const EDID_MAGIC: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

/// Vendor/model/serial pulled from a display's EDID. These three values
/// are burned into the monitor's firmware, so unlike CGDirectDisplayID or
/// IORegistryEntryID, they survive power cycles, sleep/wake, and
/// reconnects -- this is what a persistent display mapping should be keyed
/// on.
///
/// Field values match exactly what CoreGraphics' CGDisplayVendorNumber /
/// CGDisplayModelNumber / CGDisplaySerialNumber return for a
/// CGDirectDisplayID (see src/cursor.rs), so an identity read live from
/// EDID here can be compared directly against one computed from the
/// cursor's CGDirectDisplayID, with no decoding needed on either side.
///
/// Caveat: some monitors don't set a real serial number in their EDID
/// (leave it 0), so two identical-model, unserialized monitors could in
/// theory collide. Not something we can detect or fix from software --
/// if you hit it, `mon-osd list`'s printed identity will look identical
/// for both and you'll need `mon-osd mappings` + physical testing to sort
/// out which is which.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayIdentity {
    pub vendor: u32,
    pub model: u32,
    pub serial: u32,
}

fn parse_edid_identity(edid: &[u8; EDID_LEN]) -> Option<DisplayIdentity> {
    if edid[0..8] != EDID_MAGIC {
        return None;
    }
    let vendor = u16::from_be_bytes([edid[8], edid[9]]) as u32;
    let model = u16::from_le_bytes([edid[10], edid[11]]) as u32;
    let serial = u32::from_le_bytes([edid[12], edid[13], edid[14], edid[15]]);
    Some(DisplayIdentity { vendor, model, serial })
}

unsafe fn read_edid_identity(svc: IoavServiceRef) -> Option<DisplayIdentity> {
    let mut buf = [0u8; EDID_LEN];
    let ret = unsafe {
        IOAVServiceReadI2C(
            svc,
            EDID_CHIP_ADDRESS,
            0,
            buf.as_mut_ptr() as *mut c_void,
            EDID_LEN as c_uint,
        )
    };
    if ret != KIO_RETURN_SUCCESS {
        return None;
    }
    parse_edid_identity(&buf)
}

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
/// `registry_id` is stable across launches but, like CGDirectDisplayID, is
/// NOT guaranteed stable across power cycles/reconnects -- use `identity`
/// (EDID-derived) for anything you need to persist.
pub struct DisplayInfo {
    pub index: usize,
    pub registry_id: u64,
    /// Best-effort EDID identity, read live over DDC. None if the read or
    /// parse failed (monitor asleep, doesn't answer EDID-over-DDC, etc.).
    pub identity: Option<DisplayIdentity>,
}

pub fn list_displays() -> Vec<DisplayInfo> {
    let services = enumerate_dcp_av_service_proxies();
    let mut out = Vec::with_capacity(services.len());
    for (index, service) in services.iter().enumerate() {
        let mut registry_id: u64 = 0;
        unsafe {
            IORegistryEntryGetRegistryEntryID(*service, &mut registry_id);
        }
        let identity = unsafe {
            let svc = IOAVServiceCreateWithService(std::ptr::null(), *service);
            if svc.is_null() { None } else { read_edid_identity(svc) }
        };
        out.push(DisplayInfo { index, registry_id, identity });
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

    /// Reads this display's EDID over DDC and extracts its vendor/model/
    /// serial identity. None if the read or parse fails.
    pub fn identity(&self) -> Option<DisplayIdentity> {
        unsafe { read_edid_identity(self.0) }
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
