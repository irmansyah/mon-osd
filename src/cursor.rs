// src/cursor.rs
//! Determines which physical display (CGDirectDisplayID) the mouse cursor
//! is currently on, using public CoreGraphics APIs. Also exposes an
//! EDID-derived vendor/model/serial identity for that display, which is
//! what should actually be persisted -- CGDirectDisplayID itself is
//! session-local and can be reassigned by macOS across power cycles,
//! sleep/wake, and reconnects.
use std::os::raw::c_void;

#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    fn CGGetDisplaysWithPoint(
        point: CGPoint,
        max_displays: u32,
        displays: *mut u32,
        matching_display_count: *mut u32,
    ) -> i32;
    fn CGDisplayIsBuiltin(display: u32) -> i32;
    // Deprecated-but-functional CoreGraphics APIs. These return the raw
    // EDID vendor/product/serial fields for a CGDirectDisplayID, matching
    // exactly what mon_osd::ioav::DisplayIdentity parses from a live EDID
    // read over DDC -- so the two are directly comparable without any
    // decoding on either side.
    fn CGDisplayVendorNumber(display: u32) -> u32;
    fn CGDisplayModelNumber(display: u32) -> u32;
    fn CGDisplaySerialNumber(display: u32) -> u32;
}

pub fn is_builtin_display(display_id: u32) -> Option<bool> {
    Some(unsafe { CGDisplayIsBuiltin(display_id) } != 0)
}

/// Returns the CGDirectDisplayID of the display currently under the mouse
/// cursor, or None if it couldn't be determined. Session-local -- can
/// change across power cycles/reconnects, so don't persist this value
/// directly. Use `display_identity` below for anything saved to disk.
pub fn display_under_cursor() -> Option<u32> {
    unsafe {
        let event = CGEventCreate(std::ptr::null_mut());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        let mut displays = [0u32; 1];
        let mut count: u32 = 0;
        let err = CGGetDisplaysWithPoint(point, 1, displays.as_mut_ptr(), &mut count);
        if err == 0 && count > 0 {
            Some(displays[0])
        } else {
            None
        }
    }
}

/// EDID-derived vendor/model/serial for a CGDirectDisplayID. Stable across
/// power cycles/reconnects since it's read from the monitor's firmware,
/// not assigned by macOS per-session. Use this (not the raw
/// CGDirectDisplayID) as the persistent key when saving a display mapping.
pub fn display_identity(display_id: u32) -> crate::ioav::DisplayIdentity {
    unsafe {
        crate::ioav::DisplayIdentity {
            vendor: CGDisplayVendorNumber(display_id),
            model: CGDisplayModelNumber(display_id),
            serial: CGDisplaySerialNumber(display_id),
        }
    }
}
