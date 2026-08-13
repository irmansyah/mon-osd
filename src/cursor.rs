// src/cursor.rs
//! Determines which physical display (CGDirectDisplayID) the mouse cursor
//! is currently on, using public CoreGraphics APIs.
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
}

pub fn is_builtin_display(display_id: u32) -> Option<bool> {
    Some(unsafe { CGDisplayIsBuiltin(display_id) } != 0)
}

/// Returns the CGDirectDisplayID of the display currently under the mouse
/// cursor, or None if it couldn't be determined.
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
