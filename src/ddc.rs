// src/ddc.rs

//! DDC/CI "VCP Feature" get/set, ported from MonitorControl's Arm64DDC.swift
//! (github.com/MonitorControl/MonitorControl), which is verified against
//! real Apple Silicon hardware. Our earlier hand-rolled packet format
//! duplicated the source address byte (once literally in the packet, once
//! via the `dataAddress` transport parameter), which corrupted every frame
//! after the first byte -- writes reported success but never took effect.
use crate::ioav::AvService;
use std::time::Duration;

pub const VCP_LUMINANCE: u8 = 0x10;
pub const VCP_CONTRAST: u8 = 0x12;
pub const VCP_VOLUME: u8 = 0x62;
pub const VCP_MUTE: u8 = 0x8D;

const CHIP_ADDRESS_7BIT: u8 = 0x37;
const DATA_ADDRESS: u32 = 0x51;

const NUM_RETRY_ATTEMPTS: u32 = 5;
const NUM_WRITE_CYCLES: u32 = 2;
const WRITE_SLEEP_US: u64 = 10_000;
const READ_SLEEP_US: u64 = 50_000;
const RETRY_SLEEP_US: u64 = 20_000;

fn checksum(seed: u8, bytes: &[u8]) -> u8 {
    bytes.iter().fold(seed, |acc, b| acc ^ b)
}

/// Builds the request packet for `send` (the payload -- either just a VCP
/// code for a Get, or [vcp_code, hi, lo] for a Set). No source-address byte
/// is included; the DDC opcode is implicit since Get VCP (0x01) always
/// carries exactly 1 payload byte and Set VCP (0x03) always carries exactly
/// 3, so `send.len()` doubles as the opcode -- a quirk of the DDC spec that
/// MonitorControl's implementation relies on.
fn build_packet(send: &[u8]) -> Vec<u8> {
    let mut packet = vec![0x80 | (send.len() as u8 + 1), send.len() as u8];
    packet.extend_from_slice(send);
    packet.push(0); // checksum placeholder

    let seed = if send.len() == 1 {
        CHIP_ADDRESS_7BIT << 1
    } else {
        (CHIP_ADDRESS_7BIT << 1) ^ (DATA_ADDRESS as u8)
    };
    let last = packet.len() - 1;
    packet[last] = checksum(seed, &packet[..last]);
    packet
}

/// Writes `send`, and if `reply_len > 0`, reads back and checksum-verifies
/// a reply. Retries the whole cycle up to NUM_RETRY_ATTEMPTS times.
fn perform_ddc_communication(svc: &AvService, send: &[u8], reply_len: usize) -> Result<Vec<u8>, String> {
    let packet = build_packet(send);
    let mut last_err = String::new();

    for _ in 0..NUM_RETRY_ATTEMPTS {
        let mut write_ok = false;
        for _ in 0..NUM_WRITE_CYCLES {
            std::thread::sleep(Duration::from_micros(WRITE_SLEEP_US));
            write_ok = svc.write_i2c(DATA_ADDRESS, &packet).is_ok();
        }

        if reply_len > 0 {
            std::thread::sleep(Duration::from_micros(READ_SLEEP_US));
            match svc.read_i2c(0, reply_len) {
                Ok(reply) if reply.len() == reply_len => {
                    let last = reply.len() - 1;
                    let expected = checksum(0x50, &reply[..last]);
                    if expected == reply[last] {
                        return Ok(reply);
                    }
                    last_err = format!(
                        "checksum mismatch (got {:#x}, expected {:#x}) -- reply: {reply:02x?}",
                        reply[last], expected
                    );
                }
                Ok(reply) => last_err = format!("short reply: {} bytes", reply.len()),
                Err(e) => last_err = format!("read_i2c failed with IOReturn {e:#x}"),
            }
        } else if write_ok {
            return Ok(Vec::new());
        } else {
            last_err = "write_i2c failed".to_string();
        }

        std::thread::sleep(Duration::from_micros(RETRY_SLEEP_US));
    }

    Err(format!(
        "DDC communication failed after {NUM_RETRY_ATTEMPTS} attempts, last error: {last_err}"
    ))
}

pub struct VcpReply {
    pub current: u16,
    pub max: u16,
}

pub fn set_vcp(svc: &AvService, vcp_code: u8, value: u16) -> Result<(), String> {
    let hi = (value >> 8) as u8;
    let lo = (value & 0xFF) as u8;
    let send = [vcp_code, hi, lo];
    perform_ddc_communication(svc, &send, 0).map(|_| ())
}

pub fn get_vcp(svc: &AvService, vcp_code: u8) -> Result<VcpReply, String> {
    let send = [vcp_code];
    let reply = perform_ddc_communication(svc, &send, 11)?;
    let max = ((reply[6] as u16) << 8) | reply[7] as u16;
    let current = ((reply[8] as u16) << 8) | reply[9] as u16;
    Ok(VcpReply { current, max })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_xor_fold_starting_from_seed() {
        assert_eq!(checksum(0x00, &[]), 0x00);
        assert_eq!(checksum(0xFF, &[0xFF]), 0x00);
        assert_eq!(checksum(0x50, &[0x01, 0x02]), 0x50 ^ 0x01 ^ 0x02);
    }

    #[test]
    fn get_packet_has_expected_shape() {
        // Get VCP for volume (0x62): send = [0x62], a single payload byte.
        let packet = build_packet(&[VCP_VOLUME]);
        // [length(0x80|2), opcode(=len=1... wait see below), vcp_code, checksum]
        assert_eq!(packet.len(), 4);
        assert_eq!(packet[0], 0x82); // 0x80 | (1 + 1)
        assert_eq!(packet[1], 1);    // send.len() as u8 -- doubles as opcode
        assert_eq!(packet[2], VCP_VOLUME);
        // Checksum should validate against the seed used for single-byte sends.
        let seed = CHIP_ADDRESS_7BIT << 1;
        let expected = checksum(seed, &packet[..3]);
        assert_eq!(packet[3], expected);
    }

    #[test]
    fn set_packet_has_expected_shape() {
        // Set VCP for luminance (0x10) to 0x1234: send = [vcp, hi, lo], 3 bytes.
        let send = [VCP_LUMINANCE, 0x12, 0x34];
        let packet = build_packet(&send);
        assert_eq!(packet.len(), 6);
        assert_eq!(packet[0], 0x84); // 0x80 | (3 + 1)
        assert_eq!(packet[1], 3);
        assert_eq!(&packet[2..5], &send);
        let seed = (CHIP_ADDRESS_7BIT << 1) ^ (DATA_ADDRESS as u8);
        let expected = checksum(seed, &packet[..5]);
        assert_eq!(packet[5], expected);
    }

    #[test]
    fn packet_never_includes_a_literal_source_address_byte() {
        // Regression test for the bug where HOST_ADDRESS (0x51) was
        // duplicated as packet[0] *and* passed separately as the
        // transport's data_address, corrupting every byte after it.
        let packet = build_packet(&[VCP_VOLUME]);
        assert_ne!(packet[0], 0x51, "packet must not start with a literal source address byte");
    }

    #[test]
    fn vcp_codes_are_distinct() {
        let codes = [VCP_LUMINANCE, VCP_CONTRAST, VCP_VOLUME, VCP_MUTE];
        for i in 0..codes.len() {
            for j in (i + 1)..codes.len() {
                assert_ne!(codes[i], codes[j]);
            }
        }
    }
}
