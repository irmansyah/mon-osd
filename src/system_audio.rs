// src/system_audio.rs
//! System (macOS-wide) output volume and mute, via CoreAudio. Public,
//! documented API -- unlike DDC, this works regardless of monitor/speaker
//! hardware, since it controls the Mac's audio output device directly.
use std::ffi::c_void;
use std::os::raw::c_int;

type AudioObjectID = u32;
type OSStatus = c_int;

#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

const K_AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectID = 1;
const K_SCOPE_GLOBAL: u32 = 0x676C_6F62; // 'glob'
const K_SCOPE_OUTPUT: u32 = 0x6F75_7470; // 'outp'
const K_ELEMENT_MAIN: u32 = 0;
const K_SELECTOR_DEFAULT_OUTPUT_DEVICE: u32 = 0x644F_7574; // 'dOut'
const K_SELECTOR_VOLUME_SCALAR: u32 = 0x766F_6C6D; // 'volm'
const K_SELECTOR_MUTE: u32 = 0x6D75_7465; // 'mute'
                                          //
// add near the top, after existing consts:
const K_SELECTOR_DEVICE_NAME: u32 = 0x6C6E_616D; // 'lnam'

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectHasProperty(object_id: AudioObjectID, address: *const AudioObjectPropertyAddress) -> u8;
    fn AudioObjectGetPropertyData(
        object_id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier_data: *const c_void,
        io_data_size: *mut u32,
        out_data: *mut c_void,
    ) -> OSStatus;
    fn AudioObjectSetPropertyData(
        object_id: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier_data: *const c_void,
        data_size: u32,
        data: *const c_void,
    ) -> OSStatus;
}

fn device_name(device_id: AudioObjectID) -> Option<String> {
    // CFStringRef handling kept minimal: ask CoreAudio for the name as a
    // CFString, then read it out via CFStringGetCString.
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(the_string: *const c_void, buffer: *mut u8, buffer_size: isize, encoding: u32) -> u8;
        fn CFRelease(cf: *const c_void);
    }
    const K_CFSTRING_ENCODING_UTF8: u32 = 0x0800_0100;

    let address = AudioObjectPropertyAddress {
        selector: K_SELECTOR_DEVICE_NAME,
        scope: K_SCOPE_GLOBAL,
        element: K_ELEMENT_MAIN,
    };
    let mut name_ref: *const c_void = std::ptr::null();
    let mut size = std::mem::size_of::<*const c_void>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            device_id,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut name_ref as *mut _ as *mut c_void,
        )
    };
    if status != 0 || name_ref.is_null() {
        return None;
    }
    let mut buf = [0u8; 256];
    let ok = unsafe { CFStringGetCString(name_ref, buf.as_mut_ptr(), buf.len() as isize, K_CFSTRING_ENCODING_UTF8) };
    unsafe { CFRelease(name_ref) };
    if ok == 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

fn friendly_status(status: OSStatus) -> String {
    match status as u32 {
        0x77686F3F => "device has no software volume control ('who?' -- kAudioHardwareUnknownPropertyError; common on HDMI/DisplayPort monitor speakers, which are often fixed-volume)".to_string(),
        0x21707269 => "permission denied ('!pri')".to_string(),
        _ => format!("OSStatus {status}"),
    }
}

fn default_output_device() -> Result<AudioObjectID, String> {
    let address = AudioObjectPropertyAddress {
        selector: K_SELECTOR_DEFAULT_OUTPUT_DEVICE,
        scope: K_SCOPE_GLOBAL,
        element: K_ELEMENT_MAIN,
    };
    let mut device_id: AudioObjectID = 0;
    let mut size = std::mem::size_of::<AudioObjectID>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut device_id as *mut _ as *mut c_void,
        )
    };
    if status != 0 {
        return Err(format!("failed to get default output device (OSStatus {status})"));
    }
    Ok(device_id)
}

/// Volume element: master (0) if the device has it, else falls back to
/// channel 1 (typical for devices with only per-channel volume).
fn volume_element(device_id: AudioObjectID) -> u32 {
    let master_addr = AudioObjectPropertyAddress {
        selector: K_SELECTOR_VOLUME_SCALAR,
        scope: K_SCOPE_OUTPUT,
        element: K_ELEMENT_MAIN,
    };
    if unsafe { AudioObjectHasProperty(device_id, &master_addr) } != 0 {
        K_ELEMENT_MAIN
    } else {
        1
    }
}

fn get_volume_scalar(device_id: AudioObjectID) -> Result<f32, String> {
    let address = AudioObjectPropertyAddress {
        selector: K_SELECTOR_VOLUME_SCALAR,
        scope: K_SCOPE_OUTPUT,
        element: volume_element(device_id),
    };
    let mut volume: f32 = 0.0;
    let mut size = std::mem::size_of::<f32>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            device_id,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            &mut volume as *mut _ as *mut c_void,
        )
    };
    if status != 0 {
        let name = device_name(device_id).unwrap_or_else(|| "current output device".to_string());
        return Err(format!("failed to get volume on \"{name}\": {}", friendly_status(status)));
    }
    Ok(volume)
}

fn set_volume_scalar(device_id: AudioObjectID, volume: f32) -> Result<(), String> {
    let volume = volume.clamp(0.0, 1.0);
    let address = AudioObjectPropertyAddress {
        selector: K_SELECTOR_VOLUME_SCALAR,
        scope: K_SCOPE_OUTPUT,
        element: volume_element(device_id),
    };
    let status = unsafe {
        AudioObjectSetPropertyData(
            device_id,
            &address,
            0,
            std::ptr::null(),
            std::mem::size_of::<f32>() as u32,
            &volume as *const _ as *const c_void,
        )
    };
    if status != 0 {
        let name = device_name(device_id).unwrap_or_else(|| "current output device".to_string());
        return Err(format!("failed to set volume on \"{name}\": {}", friendly_status(status)));
    }
    Ok(())
}

/// Returns (current, max) as 0-100, matching the DDC-style convention used
/// elsewhere in this CLI.
pub fn get_volume_percent() -> Result<(u16, u16), String> {
    let device = default_output_device()?;
    let scalar = get_volume_scalar(device)?;
    Ok(((scalar * 100.0).round() as u16, 100))
}

pub fn set_volume_percent(value: u16) -> Result<(), String> {
    let device = default_output_device()?;
    set_volume_scalar(device, value.min(100) as f32 / 100.0)
}

pub fn change_volume_percent(delta: i32) -> Result<(u16, u16), String> {
    let (current, max) = get_volume_percent()?;
    let new_val = (current as i32 + delta).clamp(0, max as i32) as u16;
    set_volume_percent(new_val)?;
    Ok((new_val, max))
}

pub fn set_mute(mute_on: bool) -> Result<(), String> {
    let device = default_output_device()?;
    let address = AudioObjectPropertyAddress {
        selector: K_SELECTOR_MUTE,
        scope: K_SCOPE_OUTPUT,
        element: K_ELEMENT_MAIN,
    };
    let value: u32 = if mute_on { 1 } else { 0 };
    let status = unsafe {
        AudioObjectSetPropertyData(
            device,
            &address,
            0,
            std::ptr::null(),
            std::mem::size_of::<u32>() as u32,
            &value as *const _ as *const c_void,
        )
    };
    if status != 0 {
        return Err(format!("failed to set mute (OSStatus {status})"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_known_who_error() {
        let msg = friendly_status(0x77686F3F_u32 as OSStatus);
        assert!(msg.contains("no software volume control"));
    }

    #[test]
    fn falls_back_to_raw_status_for_unknown_codes() {
        let msg = friendly_status(-1);
        assert_eq!(msg, "OSStatus -1");
    }
}
