//! Minimal DDC/CI "VCP Feature" get/set implementation, per the public
//! VESA Monitor Control Command Set (MCCS) spec. This part is standard and
//! well documented (same packet layout ddcutil/ddcctl/m1ddc all use) --
//! the only Apple-specific/undocumented piece is the transport in ioav.rs.

use crate::ioav::AvService;

// Common VCP (Virtual Control Panel) feature codes, per MCCS.
pub const VCP_LUMINANCE: u8 = 0x10; // "brightness"
pub const VCP_CONTRAST: u8 = 0x12;
pub const VCP_VOLUME: u8 = 0x62;
pub const VCP_MUTE: u8 = 0x8D; // 1 = mute, 2 = unmute

/// Host ("source") address used in the DDC/CI packet body.
const HOST_ADDRESS: u8 = 0x51;
/// Sub-address DDC/CI writes are addressed to.
const DATA_ADDRESS: u32 = 0x51;

fn checksum(seed: u8, bytes: &[u8]) -> u8 {
    bytes.iter().fold(seed, |acc, b| acc ^ b)
}

/// Builds a "Set VCP Feature" packet: sets `vcp_code` to `value`.
fn build_set_packet(vcp_code: u8, value: u16) -> Vec<u8> {
    let hi = (value >> 8) as u8;
    let lo = (value & 0xFF) as u8;
    // body: [host_addr, length|0x80, command=0x03, vcp_code, val_hi, val_lo]
    let body = [HOST_ADDRESS, 0x84, 0x03, vcp_code, hi, lo];
    let mut packet = body.to_vec();
    packet.push(checksum(0x6E, &body));
    packet
}

/// Builds a "Get VCP Feature" request packet.
fn build_get_packet(vcp_code: u8) -> Vec<u8> {
    // body: [host_addr, length|0x80, command=0x01, vcp_code]
    let body = [HOST_ADDRESS, 0x82, 0x01, vcp_code];
    let mut packet = body.to_vec();
    packet.push(checksum(0x6E, &body));
    packet
}

/// Parsed reply to a "Get VCP Feature" request.
pub struct VcpReply {
    pub current: u16,
    pub max: u16,
}

/// Sends a Set VCP Feature command for `vcp_code` = `value`.
pub fn set_vcp(svc: &AvService, vcp_code: u8, value: u16) -> Result<(), String> {
    let packet = build_set_packet(vcp_code, value);
    svc.write_i2c(DATA_ADDRESS, &packet)
        .map_err(|e| format!("write_i2c failed with IOReturn {e:#x}"))
}

/// Sends a Get VCP Feature request and parses the reply.
///
/// NOTE: real DDC/CI displays need a few ms between the write (request) and
/// the read (reply) -- we sleep briefly here. If reads come back as all
/// zeroes or checksum-invalid on your monitor, try increasing the delay
/// first before assuming the transport itself is broken.
pub fn get_vcp(svc: &AvService, vcp_code: u8) -> Result<VcpReply, String> {
    let request = build_get_packet(vcp_code);
    svc.write_i2c(DATA_ADDRESS, &request)
        .map_err(|e| format!("write_i2c (request) failed with IOReturn {e:#x}"))?;

    std::thread::sleep(std::time::Duration::from_millis(50));

    // Reply body: [dest=0x6E, len|0x80, cmd=0x02, result, vcp_code, type, max_hi, max_lo, cur_hi, cur_lo, checksum]
    let reply = svc
        .read_i2c(0, 11)
        .map_err(|e| format!("read_i2c failed with IOReturn {e:#x}"))?;

    if reply.len() < 11 {
        return Err(format!("short reply: {} bytes", reply.len()));
    }

    let expected_chk = checksum(0x50, &reply[..reply.len() - 1]);
    if expected_chk != reply[reply.len() - 1] {
        return Err(format!(
            "checksum mismatch (got {:#x}, expected {:#x}) -- reply: {reply:02x?}",
            reply[reply.len() - 1],
            expected_chk
        ));
    }

    if reply[2] != 0x02 {
        return Err(format!("unexpected reply command byte: {:#x}", reply[2]));
    }
    if reply[4] != vcp_code {
        return Err(format!(
            "reply VCP code {:#x} does not match requested {:#x}",
            reply[4], vcp_code
        ));
    }

    let max = ((reply[6] as u16) << 8) | reply[7] as u16;
    let current = ((reply[8] as u16) << 8) | reply[9] as u16;
    Ok(VcpReply { current, max })
}
