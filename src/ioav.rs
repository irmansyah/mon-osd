//! FFI bindings to Apple's private, undocumented IOAVService API.
//!
//! These symbols have no public header -- they were reverse-engineered by
//! the community (see https://alinpanaitiu.com/blog/journey-to-ddc-on-m1-macs/
//! and the MonitorControl/m1ddc/Lunar source). They live in a framework that
//! is already part of the base macOS system image, but Apple has never
//! published a header for them, so we declare the prototypes ourselves.
//!
//! IMPORTANT: this is unverified against real hardware from this session --
//! I don't have access to a Mac to compile/link/test. Expect the first
//! `cargo build` on your machine to surface link errors or wrong-argument
//! issues that we'll need to iterate on together.

use std::ffi::c_void;
use std::os::raw::{c_int, c_uint};

/// Opaque handle to an AV service (roughly: "a connection to one display's
/// DDC channel"). CFTypeRef under the hood.
pub type IoavServiceRef = *mut c_void;

pub type IoReturn = c_int;
pub const KIO_RETURN_SUCCESS: IoReturn = 0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    /// Creates a service handle for the *default* AV-capable external
    /// display. Fine for a single-external-monitor setup; multi-monitor
    /// setups need IOAVServiceCreateWithService + IOKit registry
    /// enumeration instead (not implemented here -- out of scope for now).
    ///
    /// `allocator` may be NULL, which CoreFoundation treats as
    /// "use the default allocator".
    fn IOAVServiceCreate(allocator: *const c_void) -> IoavServiceRef;

    /// Writes `input_buffer` (an I2C/DDC packet) to `chip_address` on the
    /// display's DDC channel.
    fn IOAVServiceWriteI2C(
        service: IoavServiceRef,
        chip_address: c_uint,
        data_address: c_uint,
        input_buffer: *const c_void,
        input_buffer_size: c_uint,
    ) -> IoReturn;

    /// Reads `output_buffer_size` bytes from `chip_address` at `offset`.
    fn IOAVServiceReadI2C(
        service: IoavServiceRef,
        chip_address: c_uint,
        offset: c_uint,
        output_buffer: *mut c_void,
        output_buffer_size: c_uint,
    ) -> IoReturn;
}

/// Standard DDC/CI 7-bit I2C device address for a display (0x37).
pub const DDC_CHIP_ADDRESS: c_uint = 0x37;

pub struct AvService(IoavServiceRef);

impl AvService {
    /// Grabs a handle to the default external display.
    /// Returns None if no AV-capable display service was found (e.g. no
    /// external monitor connected, or connected via the M1/entry-M2 HDMI
    /// port, which this private API does not support -- USB-C/DP only).
    pub fn default_display() -> Option<Self> {
        let svc = unsafe { IOAVServiceCreate(std::ptr::null()) };
        if svc.is_null() {
            None
        } else {
            Some(Self(svc))
        }
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
        if ret == KIO_RETURN_SUCCESS {
            Ok(())
        } else {
            Err(ret)
        }
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
        if ret == KIO_RETURN_SUCCESS {
            Ok(buf)
        } else {
            Err(ret)
        }
    }
}
