// src/native_brightness.rs
//! Built-in display brightness via the private DisplayServices framework
//! (same reverse-engineered API MonitorControl and the `brightness` CLI
//! use). DDC/CI doesn't reach Apple's own panels, so this is the only way
//! to control the laptop screen's brightness in code.
use std::os::raw::c_int;

#[link(name = "DisplayServices", kind = "framework")]
unsafe extern "C" {
    fn DisplayServicesGetBrightness(display_id: u32, brightness: *mut f32) -> c_int;
    fn DisplayServicesSetBrightness(display_id: u32, brightness: f32) -> c_int;
}

/// Returns (current, max) as 0-100, matching the DDC-style convention.
pub fn get_brightness_percent(display_id: u32) -> Result<(u16, u16), String> {
    let mut b: f32 = 0.0;
    let ret = unsafe { DisplayServicesGetBrightness(display_id, &mut b) };
    if ret != 0 {
        return Err(format!("DisplayServicesGetBrightness failed (status {ret})"));
    }
    Ok(((b * 100.0).round() as u16, 100))
}

pub fn set_brightness_percent(display_id: u32, value: u16) -> Result<(), String> {
    let ret = unsafe { DisplayServicesSetBrightness(display_id, value.min(100) as f32 / 100.0) };
    if ret != 0 {
        return Err(format!("DisplayServicesSetBrightness failed (status {ret})"));
    }
    Ok(())
}
