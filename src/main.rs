// src/main.rs
mod cache;
mod cursor;
mod ddc;
mod display_map;
mod ioav;
mod native_brightness;
mod system_audio;

use cache::Cache;
use clap::{Parser, Subcommand};
use display_map::DisplayMap;
use ioav::AvService;

#[derive(Parser)]
#[command(name = "mon-osd", about = "Minimal brightness/volume/contrast control across built-in and external displays")]
struct Cli {
    /// Which external display to target for DDC (luminance/contrast on
    /// non-built-in screens): an index from `mon-osd list`, or "cursor" to
    /// auto-pick whichever monitor the mouse is on. Ignored for volume/mute
    /// (always system-wide) and for luminance when the cursor is on the
    /// built-in display (routed to the native brightness API instead).
    #[arg(long, global = true, default_value = "0")]
    display: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Clone)]
enum Command {
    /// List AV-capable external displays found in the IORegistry
    List,
    /// Associate the display currently under the mouse cursor with an AV
    /// index (from `mon-osd list`). Move the mouse onto an external
    /// monitor, then run this. Auto-labels with the aerospace-reported
    /// monitor name unless --name is given. Do NOT run this while the
    /// cursor is on the built-in display -- it doesn't use DDC at all.
    Map {
        index: usize,
        #[arg(long)]
        name: Option<String>,
    },
    /// Show all saved cursor-display -> AV-index mappings
    Mappings,
    /// Print current + max value for a feature ("luminance" | "volume" | "contrast")
    Get { feature: String },
    /// Set a feature to an absolute value (0-100 typical)
    Set { feature: String, value: u16 },
    /// Adjust a feature by +/- delta, clamped to [0, max]. Prints the
    /// resulting value on stdout, so Hammerspoon can draw an accurate bar
    /// without needing a second round trip.
    Change {
        feature: String,
        #[arg(allow_hyphen_values = true)]
        delta: i32,
    },
    /// Set mute on/off (system output mute -- always CoreAudio, not DDC)
    Mute { state: String },
}

fn vcp_code(feature: &str) -> Result<u8, String> {
    match feature.to_lowercase().as_str() {
        "luminance" | "brightness" => Ok(ddc::VCP_LUMINANCE),
        "contrast" => Ok(ddc::VCP_CONTRAST),
        "volume" => Ok(ddc::VCP_VOLUME),
        other => Err(format!("unknown feature '{other}' (expected luminance|contrast|volume)")),
    }
}

/// Best-effort: asks `aerospace` which monitor currently has focus, so
/// `map` can auto-label entries instead of storing blind hex IDs. Returns
/// None if aerospace isn't installed or the call fails -- purely cosmetic.
fn aerospace_focused_monitor_name() -> Option<String> {
    let output = std::process::Command::new("aerospace")
        .args(["list-monitors", "--focused"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let line = text.lines().next()?;
    let (_, name) = line.split_once('|')?;
    Some(name.trim().to_string())
}

/// Resolves `--display` (a literal index, or "cursor") to a concrete AV
/// index for DDC-based commands (external monitor luminance/contrast).
fn resolve_display_index(selector: &str) -> Result<usize, String> {
    if selector.eq_ignore_ascii_case("cursor") {
        let display_id = cursor::display_under_cursor()
            .ok_or_else(|| "could not determine which display the cursor is on".to_string())?;

        if cursor::is_builtin_display(display_id) == Some(true) {
            return Err(
                "cursor is on the built-in display -- DDC/CI isn't supported on Apple's own panels, only external monitors over USB-C/DisplayPort".to_string()
            );
        }

        let map = DisplayMap::load();
        map.get(display_id).ok_or_else(|| {
            format!(
                "no mapping for the display under the cursor (CGDirectDisplayID {display_id:#x}) -- \
                 move the mouse onto each monitor once and run `mon-osd map <index>` (see `mon-osd list` for indices)"
            )
        })
    } else {
        selector
            .parse::<usize>()
            .map_err(|_| format!("invalid --display value '{selector}' (expected a number or \"cursor\")"))
    }
}

/// Fetches the current VCP value from an external monitor, preferring a
/// live hardware read but falling back to the last cached value (or a sane
/// default) when the display doesn't reply.
fn get_vcp_cached(svc: &AvService, cache: &mut Cache, code: u8) -> ddc::VcpReply {
    match ddc::get_vcp(svc, code) {
        Ok(reply) => {
            cache.set(code, reply.current, reply.max);
            reply
        }
        Err(_) => {
            let (current, max) = cache.get(code).unwrap_or((50, 100));
            ddc::VcpReply { current, max }
        }
    }
}

fn main() {
    let cli = Cli::parse();

    // --- List: doesn't need any display resolved ---
    if matches!(cli.command, Command::List) {
        let displays = ioav::list_displays();
        if displays.is_empty() {
            eprintln!("no AV-capable displays found in the IORegistry");
            std::process::exit(1);
        }
        for d in displays {
            println!("{}: registry id {:#x}", d.index, d.registry_id);
        }
        return;
    }

    // --- Map: associate the display under the cursor with an AV index ---
    if let Command::Map { index, name } = &cli.command {
        let Some(display_id) = cursor::display_under_cursor() else {
            eprintln!("could not determine which display the cursor is on");
            std::process::exit(1);
        };
        let resolved_name = name.clone().or_else(aerospace_focused_monitor_name);
        let mut map = DisplayMap::load();
        map.set(display_id, *index, resolved_name.clone());
        match resolved_name {
            Some(n) => println!("mapped display {display_id:#x} (\"{n}\") -> AV index {index}"),
            None => println!("mapped display {display_id:#x} -> AV index {index}"),
        }
        return;
    }

    // --- Mappings: show saved cursor-display -> AV-index mappings ---
    if matches!(cli.command, Command::Mappings) {
        let map = DisplayMap::load();
        for (id, m) in map.all() {
            match &m.name {
                Some(n) => println!("{id:#x} -> AV index {} (\"{n}\")", m.av_index),
                None => println!("{id:#x} -> AV index {}", m.av_index),
            }
        }
        return;
    }

    // --- Volume and mute: always system-wide via CoreAudio, never DDC ---
    match &cli.command {
        Command::Get { feature } if feature.eq_ignore_ascii_case("volume") => {
            match system_audio::get_volume_percent() {
                Ok((cur, max)) => println!("{cur} {max}"),
                Err(e) if e.contains("no software volume control") => {
                    let av_index = match resolve_display_index(&cli.display) {
                        Ok(i) => i,
                        Err(resolve_err) => {
                            eprintln!("error: system volume unavailable ({e}), and DDC fallback failed: {resolve_err}");
                            std::process::exit(1);
                        }
                    };
                    let Some(svc) = AvService::display_at_index(av_index) else {
                        eprintln!("error: system volume unavailable ({e}), and no AV display at index {av_index} for DDC fallback");
                        std::process::exit(1);
                    };
                    let mut cache = Cache::load();
                    let r = get_vcp_cached(&svc, &mut cache, ddc::VCP_VOLUME);
                    println!("{} {}", r.current, r.max);
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Command::Set { feature, value } if feature.eq_ignore_ascii_case("volume") => {
            match system_audio::set_volume_percent(*value) {
                Ok(()) => {}
                Err(e) if e.contains("no software volume control") => {
                    // CoreAudio can't control this device -- fall back to the
                    // monitor's own hardware volume via DDC.
                    let av_index = match resolve_display_index(&cli.display) {
                        Ok(i) => i,
                        Err(resolve_err) => {
                            eprintln!("error: system volume unavailable ({e}), and DDC fallback failed: {resolve_err}");
                            std::process::exit(1);
                        }
                    };
                    let Some(svc) = AvService::display_at_index(av_index) else {
                        eprintln!("error: system volume unavailable ({e}), and no AV display at index {av_index} for DDC fallback");
                        std::process::exit(1);
                    };
                    if let Err(ddc_err) = ddc::set_vcp(&svc, ddc::VCP_VOLUME, *value) {
                        eprintln!("error: system volume unavailable ({e}), and DDC fallback failed: {ddc_err}");
                        std::process::exit(1);
                    }
                    let mut cache = Cache::load();
                    let max = cache.get(ddc::VCP_VOLUME).map(|(_, m)| m).unwrap_or(100);
                    cache.set(ddc::VCP_VOLUME, *value, max);
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Command::Change { feature, delta } if feature.eq_ignore_ascii_case("volume") => {
            match system_audio::change_volume_percent(*delta) {
                Ok((new_val, max)) => {
                    println!("{new_val} {max}");
                }
                Err(e) if e.contains("no software volume control") => {
                    let av_index = match resolve_display_index(&cli.display) {
                        Ok(i) => i,
                        Err(resolve_err) => {
                            eprintln!("error: system volume unavailable ({e}), and DDC fallback failed: {resolve_err}");
                            std::process::exit(1);
                        }
                    };
                    let Some(svc) = AvService::display_at_index(av_index) else {
                        eprintln!("error: system volume unavailable ({e}), and no AV display at index {av_index} for DDC fallback");
                        std::process::exit(1);
                    };
                    let mut cache = Cache::load();
                    let reply = get_vcp_cached(&svc, &mut cache, ddc::VCP_VOLUME);
                    let new_val = (reply.current as i32 + *delta).clamp(0, reply.max as i32) as u16;
                    if let Err(ddc_err) = ddc::set_vcp(&svc, ddc::VCP_VOLUME, new_val) {
                        eprintln!("error: system volume unavailable ({e}), and DDC fallback failed: {ddc_err}");
                        std::process::exit(1);
                    }
                    cache.set(ddc::VCP_VOLUME, new_val, reply.max);
                    println!("{new_val} {}", reply.max);
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        }
        Command::Mute { state } => {
            let mute_on = match state.to_lowercase().as_str() {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                other => {
                    eprintln!("unknown mute state '{other}' (expected on|off)");
                    std::process::exit(1);
                }
            };
            if let Err(e) = system_audio::set_mute(mute_on) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            return;
        }
        _ => {}
    }

    // --- Luminance on the built-in display: native DisplayServices, not DDC ---
    if cli.display.eq_ignore_ascii_case("cursor") {
        if let Some(display_id) = cursor::display_under_cursor() {
            if cursor::is_builtin_display(display_id) == Some(true) {
                match &cli.command {
                    Command::Get { feature } if vcp_code(feature).ok() == Some(ddc::VCP_LUMINANCE) => {
                        match native_brightness::get_brightness_percent(display_id) {
                            Ok((cur, max)) => println!("{cur} {max}"),
                            Err(e) => {
                                eprintln!("error: {e}");
                                std::process::exit(1);
                            }
                        }
                        return;
                    }
                    Command::Set { feature, value } if vcp_code(feature).ok() == Some(ddc::VCP_LUMINANCE) => {
                        if let Err(e) = native_brightness::set_brightness_percent(display_id, *value) {
                            eprintln!("error: {e}");
                            std::process::exit(1);
                        }
                        return;
                    }
                    Command::Change { feature, delta } if vcp_code(feature).ok() == Some(ddc::VCP_LUMINANCE) => {
                        let (current, max) =
                            native_brightness::get_brightness_percent(display_id).unwrap_or((50, 100));
                        let new_val = (current as i32 + delta).clamp(0, max as i32) as u16;
                        if let Err(e) = native_brightness::set_brightness_percent(display_id, new_val) {
                            eprintln!("error: {e}");
                            std::process::exit(1);
                        }
                        println!("{new_val} {max}");
                        return;
                    }
                    _ => {
                        eprintln!(
                            "error: cursor is on the built-in display -- only brightness is controllable there, not contrast"
                        );
                        std::process::exit(1);
                    }
                }
            }
        }
    }

    // --- Everything else: DDC over an external monitor ---
    let av_index = match resolve_display_index(&cli.display) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let svc = match AvService::display_at_index(av_index) {
        Some(s) => s,
        None => {
            eprintln!(
                "no AV-capable display at index {av_index} (check it's on USB-C/DisplayPort, not the M1/entry-M2 HDMI port; run `mon-osd list` to see available indices)"
            );
            std::process::exit(1);
        }
    };

    let result = match cli.command {
        Command::List | Command::Map { .. } | Command::Mappings | Command::Mute { .. } => {
            unreachable!("handled above")
        }
        Command::Get { feature } => vcp_code(&feature).and_then(|code| {
            let mut cache = Cache::load();
            let r = get_vcp_cached(&svc, &mut cache, code);
            println!("{} {}", r.current, r.max);
            Ok(())
        }),
        Command::Set { feature, value } => vcp_code(&feature).and_then(|code| {
            ddc::set_vcp(&svc, code, value)?;
            let mut cache = Cache::load();
            let max = cache.get(code).map(|(_, m)| m).unwrap_or(100);
            cache.set(code, value, max);
            Ok(())
        }),
        Command::Change { feature, delta } => vcp_code(&feature).and_then(|code| {
            let mut cache = Cache::load();
            let reply = get_vcp_cached(&svc, &mut cache, code);
            let new_val = (reply.current as i32 + delta).clamp(0, reply.max as i32) as u16;
            ddc::set_vcp(&svc, code, new_val)?;
            cache.set(code, new_val, reply.max);
            println!("{} {}", new_val, reply.max);
            Ok(())
        }),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcp_code_accepts_known_features_case_insensitively() {
        assert_eq!(vcp_code("volume"), Ok(ddc::VCP_VOLUME));
        assert_eq!(vcp_code("VOLUME"), Ok(ddc::VCP_VOLUME));
        assert_eq!(vcp_code("luminance"), Ok(ddc::VCP_LUMINANCE));
        assert_eq!(vcp_code("brightness"), Ok(ddc::VCP_LUMINANCE)); // alias
        assert_eq!(vcp_code("contrast"), Ok(ddc::VCP_CONTRAST));
    }

    #[test]
    fn vcp_code_rejects_unknown_features() {
        assert!(vcp_code("saturation").is_err());
        assert!(vcp_code("").is_err());
    }

    #[test]
    fn change_clamps_to_valid_range() {
        // Mirrors the clamp logic used in the Change arm.
        let current = 95i32;
        let max = 100i32;
        let new_val = (current + 20).clamp(0, max);
        assert_eq!(new_val, 100);

        let new_val = (5i32 - 20).clamp(0, max);
        assert_eq!(new_val, 0);
    }
}
