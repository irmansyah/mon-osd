# mon-osd

Minimal CLI for controlling display brightness, contrast, and volume on macOS (Apple Silicon), built for driving Hammerspoon-style OSD keybindings.

Combines three different control paths depending on what's actually being adjusted and which screen it targets, since no single macOS API covers all of it:

| Feature | Built-in display | External display |
|---|---|---|
| Brightness (luminance) | Native `DisplayServices` (private framework) | DDC/CI (VCP `0x10`) |
| Contrast | Not supported (no OS-level control exists) | DDC/CI (VCP `0x12`) |
| Volume | System output via CoreAudio | CoreAudio, falling back to DDC/CI (VCP `0x62`) if the device has no software volume control |
| Mute | System output via CoreAudio | Same (system-wide, not per-display) |

## Why this exists

macOS has no unified, documented API for external-monitor brightness/contrast/volume. This tool talks to external displays over DDC/CI (the VESA MCCS standard) via Apple's private, undocumented `IOAVService` framework — the same reverse-engineered approach used by [MonitorControl](https://github.com/MonitorControl/MonitorControl), [Lunar](https://lunar.fyi), and [m1ddc](https://github.com/waydabber/m1ddc). Built-in-panel brightness and system volume use their own separate (also private, in the brightness case) APIs, since DDC/CI doesn't reach Apple's own screens or the system audio mixer at all.

Because this leans on undocumented, reverse-engineered symbols, expect it to occasionally need adjustment across macOS versions — there's no public header to depend on.

## Build

```bash
cargo build --release
```

`build.rs` (at the project root, alongside `Cargo.toml`) adds `/System/Library/PrivateFrameworks` to the linker's framework search path — required for the `DisplayServices` framework used for built-in brightness. Without it, linking fails with unresolved `DisplayServices*` symbols.

## Commands

```
mon-osd list
mon-osd map <index> [--name <label>]
mon-osd mappings
mon-osd get <feature>
mon-osd set <feature> <value>
mon-osd change <feature> <delta>
mon-osd mute <on|off>

mon-osd --display 0 change volume -10
mon-osd --display 0 change volume +10
mon-osd --display cursor change brightness -- -20
mon-osd --display cursor change brightness -- +20

```

`<feature>` is one of `luminance` (alias `brightness`), `contrast`, or `volume`.

### `--display <selector>`

Applies to `get`/`set`/`change` on `luminance`/`contrast`. Accepts:

- A numeric index from `mon-osd list` (e.g. `--display 0`)
- `cursor` — auto-resolves to whichever physical display the mouse is currently on

Ignored for `volume`/`mute`, which are always system-wide via CoreAudio (with DDC as a fallback for volume specifically, using `--display` to pick the fallback target if CoreAudio can't control the current output device).

Default: `0`.

### Negative deltas

`change`'s `delta` accepts negative numbers directly (e.g. `change volume -10`) — no `--` separator needed.

## One-time setup: mapping displays

`list` enumerates AV-capable `DCPAVServiceProxy` entries in the IORegistry by index, but those indices aren't inherently tied to a specific physical monitor — and multi-monitor Macs can have more than one entry (including one for the built-in panel's own DCP proxy on some configurations).

To use `--display cursor` reliably, map each physical *external* monitor once:

```bash
mon-osd list
# 0: registry id 0x1001f3a89
# 1: registry id 0x100213001

# Move the mouse onto the external monitor, then confirm which index
# actually affects it (watch/listen for a real change):
mon-osd --display 0 set luminance 50

# Once confirmed, map it (auto-labels using `aerospace list-monitors
# --focused` if aerospace is installed, else pass --name explicitly):
mon-osd map 0
```

Do **not** map the built-in display — it doesn't use DDC/CI at all; luminance on it is handled automatically via the native brightness path once the cursor is detected there (see `mon-osd --display cursor change luminance <delta>`), and contrast has no equivalent control on Apple's own panels.

View saved mappings any time with `mon-osd mappings`.

## Known caveats

- **Reads over DDC/CI are flaky by design.** Some displays fail Get VCP requests intermittently (a documented, known issue with this private API on Apple Silicon); some displays never reply to Get VCP at all despite accepting Set VCP writes. `get`/`change` fall back to a locally cached last-known value (`~/.cache/mon-osd/state`) when a hardware read fails, rather than erroring.
- **Contrast is not adjustable on the built-in display**, full stop — this is an OS/hardware limitation, not a bug.
- **Some external monitor speakers report no CoreAudio volume property** (`kAudioHardwareUnknownPropertyError`, common on fixed-volume HDMI/DisplayPort audio passthrough). When that happens, `volume` commands automatically fall back to the monitor's own DDC hardware volume (VCP `0x62`) using `--display` to select the target — pass `--display <index>` or `--display cursor` on volume commands if your default output device hits this.
- **DDC/CI doesn't work over the M1 / entry-level M2 Mac mini's HDMI port** — USB-C/DisplayPort only, a limitation of Apple's private transport, not this tool.

## Project layout

```
src/
├── main.rs              CLI parsing and command dispatch
├── ddc.rs                DDC/CI VCP get/set (packet format ported from MonitorControl's Arm64DDC.swift)
├── ioav.rs                IOAVService FFI + IORegistry DCPAVServiceProxy enumeration
├── cache.rs                Last-known VCP value cache (fallback for unreliable/absent hardware reads)
├── cursor.rs                 Cursor-position → CGDirectDisplayID, built-in display detection
├── display_map.rs             Persisted CGDirectDisplayID → AV-index mapping (for `--display cursor`)
├── native_brightness.rs        Built-in display brightness via DisplayServices
└── system_audio.rs               System volume/mute via CoreAudio, with device-name-aware error reporting
build.rs                             Adds PrivateFrameworks to the linker search path
```

## Credit

DDC/CI packet construction and communication timing/retry logic ported from [MonitorControl](https://github.com/MonitorControl/MonitorControl)'s `Arm64DDC.swift`, which is verified against real Apple Silicon hardware by a large user base. Background on the `IOAVService` reverse-engineering effort: [alinpanaitiu.com/blog/journey-to-ddc-on-m1-macs](https://alinpanaitiu.com/blog/journey-to-ddc-on-m1-macs/).
