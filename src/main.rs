mod ddc;
mod ioav;

use clap::{Parser, Subcommand};
use ioav::AvService;

#[derive(Parser)]
#[command(name = "mon-osd", about = "Minimal DDC/CI control for one external display")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print current + max value for a VCP feature ("luminance" | "volume" | "contrast")
    Get { feature: String },
    /// Set a VCP feature to an absolute value (0-100 typical)
    Set { feature: String, value: u16 },
    /// Adjust a VCP feature by +/- delta, clamped to [0, max]. Prints the
    /// resulting value on stdout, so Hammerspoon can draw an accurate bar
    /// without needing a second round trip.
    Change { feature: String, delta: i32 },
    /// Set mute on/off (VCP 0x8D: 1 = mute, 2 = unmute)
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

fn main() {
    let cli = Cli::parse();

    let svc = match AvService::default_display() {
        Some(s) => s,
        None => {
            eprintln!("no AV-capable external display found (check it's on USB-C/DisplayPort, not the M1/entry-M2 HDMI port)");
            std::process::exit(1);
        }
    };

    let result = match cli.command {
        Command::Get { feature } => vcp_code(&feature).and_then(|code| {
            ddc::get_vcp(&svc, code).map(|r| println!("{} {}", r.current, r.max))
        }),
        Command::Set { feature, value } => {
            vcp_code(&feature).and_then(|code| ddc::set_vcp(&svc, code, value))
        }
        Command::Change { feature, delta } => vcp_code(&feature).and_then(|code| {
            let reply = ddc::get_vcp(&svc, code)?;
            let new_val = (reply.current as i32 + delta).clamp(0, reply.max as i32) as u16;
            ddc::set_vcp(&svc, code, new_val)?;
            println!("{} {}", new_val, reply.max);
            Ok(())
        }),
        Command::Mute { state } => {
            let val: u16 = match state.to_lowercase().as_str() {
                "on" | "true" | "1" => 1,
                "off" | "false" | "0" => 2,
                other => {
                    eprintln!("unknown mute state '{other}' (expected on|off)");
                    std::process::exit(1);
                }
            };
            ddc::set_vcp(&svc, ddc::VCP_MUTE, val)
        }
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
