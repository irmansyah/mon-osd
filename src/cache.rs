//! Tiny local cache for last-known VCP values, keyed by VCP code.
//!
//! Some monitors (confirmed: this one) accept "Set VCP Feature" writes but
//! never reply to "Get VCP Feature" requests. Since we can't trust hardware
//! reads on such displays, we track the last value *we* set (or successfully
//! read, on displays where reads do work) locally, so `get`/`change` still
//! report accurate numbers without needing a round trip that will never
//! succeed.
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub struct Cache {
    path: PathBuf,
    values: HashMap<u8, (u16, u16)>, // vcp_code -> (current, max)
}

impl Cache {
    pub fn load() -> Self {
        let path = Self::cache_path();
        let values = fs::read_to_string(&path)
            .map(|contents| parse_state(&contents))
            .unwrap_or_default();
        Self { path, values }
    }

    fn cache_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".cache").join("mon-osd").join("state")
    }

    /// Last-known (current, max) for a VCP code, if we've ever set or read it.
    pub fn get(&self, vcp_code: u8) -> Option<(u16, u16)> {
        self.values.get(&vcp_code).copied()
    }

    /// Records a value and persists immediately (state file is tiny, so this
    /// is cheap and keeps us safe against the process being killed).
    pub fn set(&mut self, vcp_code: u8, current: u16, max: u16) {
        self.values.insert(vcp_code, (current, max));
        self.save();
    }

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut f) = fs::File::create(&self.path) {
            let _ = f.write_all(serialize_state(&self.values).as_bytes());
        }
    }
}

// Replace the body of `load()`'s parsing loop and `save()`'s formatting
// loop with calls to these two pure functions, so both are independently
// testable without touching the filesystem:

fn parse_state(contents: &str) -> HashMap<u8, (u16, u16)> {
    let mut values = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((code_str, rest)) = line.split_once('=') else { continue };
        let Some((cur_str, max_str)) = rest.split_once(',') else { continue };
        if let (Ok(code), Ok(cur), Ok(max)) = (
            u8::from_str_radix(code_str.trim(), 16),
            cur_str.trim().parse::<u16>(),
            max_str.trim().parse::<u16>(),
        ) {
            values.insert(code, (cur, max));
        }
    }
    values
}

fn serialize_state(values: &HashMap<u8, (u16, u16)>) -> String {
    let mut out = String::from("# mon-osd cached VCP state -- auto-generated, safe to delete\n");
    for (code, (cur, max)) in values {
        out.push_str(&format!("{code:02x}={cur},{max}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_state_reads_valid_lines() {
        let contents = "# comment\n62=50,100\n10=30,100\n\n";
        let values = parse_state(contents);
        assert_eq!(values.get(&0x62), Some(&(50, 100)));
        assert_eq!(values.get(&0x10), Some(&(30, 100)));
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn parse_state_ignores_malformed_lines() {
        let contents = "not a valid line\n62=notanumber,100\n62=50\n";
        let values = parse_state(contents);
        assert!(values.is_empty());
    }

    #[test]
    fn parse_state_handles_empty_input() {
        assert!(parse_state("").is_empty());
    }

    #[test]
    fn serialize_then_parse_round_trips() {
        let mut values = HashMap::new();
        values.insert(0x62u8, (75u16, 100u16));
        values.insert(0x10u8, (40u16, 100u16));
        let serialized = serialize_state(&values);
        let parsed = parse_state(&serialized);
        assert_eq!(parsed, values);
    }
}
