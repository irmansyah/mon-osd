// src/display_map.rs
//! Persists AV-index mappings for external displays, keyed by their
//! EDID-derived DisplayIdentity (vendor/model/serial) rather than by
//! CGDirectDisplayID. CGDirectDisplayID is reassigned by macOS across
//! power cycles, sleep/wake, and reconnects, which was causing saved
//! mappings to silently point at the wrong AV index (or nothing) after
//! e.g. turning a monitor off and back on. EDID identity is read from the
//! monitor's own firmware and doesn't change with any of that.
use crate::ioav::DisplayIdentity;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub struct DisplayMapping {
    pub av_index: usize,
    pub name: Option<String>,
}

pub struct DisplayMap {
    path: PathBuf,
    values: HashMap<DisplayIdentity, DisplayMapping>,
}

impl DisplayMap {
    pub fn load() -> Self {
        let path = Self::path();
        let values = fs::read_to_string(&path)
            .map(|contents| parse_mappings(&contents))
            .unwrap_or_default();
        Self { path, values }
    }

    fn path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        // Deliberately a new filename (not the old "display-map") since
        // the key format changed from CGDirectDisplayID to EDID identity
        // -- this avoids misreading an old-format file as the new one.
        // Old mappings aren't migrated automatically; re-run `mon-osd map
        // <index>` on each external monitor once after upgrading.
        PathBuf::from(home).join(".cache").join("mon-osd").join("display-map-edid")
    }

    pub fn get(&self, identity: DisplayIdentity) -> Option<usize> {
        self.values.get(&identity).map(|m| m.av_index)
    }

    pub fn all(&self) -> impl Iterator<Item = (&DisplayIdentity, &DisplayMapping)> {
        self.values.iter()
    }

    pub fn set(&mut self, identity: DisplayIdentity, av_index: usize, name: Option<String>) {
        self.values.insert(identity, DisplayMapping { av_index, name });
        self.save();
    }

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut f) = fs::File::create(&self.path) {
            let _ = f.write_all(serialize_mappings(&self.values).as_bytes());
        }
    }
}

fn parse_mappings(contents: &str) -> HashMap<DisplayIdentity, DisplayMapping> {
    let mut values = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((id_str, rest)) = line.split_once('=') else { continue };

        let id_parts: Vec<&str> = id_str.split(':').collect();
        if id_parts.len() != 3 {
            continue;
        }
        let (Ok(vendor), Ok(model), Ok(serial)) = (
            id_parts[0].trim().parse::<u32>(),
            id_parts[1].trim().parse::<u32>(),
            id_parts[2].trim().parse::<u32>(),
        ) else {
            continue;
        };

        let mut parts = rest.splitn(2, ',');
        let idx_str = parts.next().unwrap_or("");
        let name = parts.next().map(|s| s.to_string()).filter(|s| !s.is_empty());
        let Ok(idx) = idx_str.trim().parse::<usize>() else { continue };

        values.insert(DisplayIdentity { vendor, model, serial }, DisplayMapping { av_index: idx, name });
    }
    values
}

fn serialize_mappings(values: &HashMap<DisplayIdentity, DisplayMapping>) -> String {
    let mut out = String::from("# mon-osd display-identity (vendor:model:serial) -> av-index mapping -- auto-generated\n");
    for (id, m) in values {
        match &m.name {
            Some(n) => out.push_str(&format!("{}:{}:{}={},{n}\n", id.vendor, id.model, id.serial, m.av_index)),
            None => out.push_str(&format!("{}:{}:{}={}\n", id.vendor, id.model, id.serial, m.av_index)),
        }
    }
    out
}

// --- Pinned volume AV index -------------------------------------------------
//
// Unlike brightness/contrast, system volume is a single, system-wide
// source -- there's only ever one output device active, regardless of
// which monitor the cursor happens to be on. So volume's DDC fallback
// (used when CoreAudio can't control the output, e.g. it's an external
// monitor's speakers) isn't resolved via cursor position or per-display
// EDID identity at all. It's pinned once via `mon-osd map-volume <index>`
// and used as-is from then on.

fn volume_index_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".cache").join("mon-osd").join("volume-av-index")
}

/// Loads the pinned volume AV index, if one has been set via
/// `mon-osd map-volume <index>`.
pub fn load_volume_index() -> Option<usize> {
    fs::read_to_string(volume_index_path()).ok()?.trim().parse().ok()
}

/// Pins the AV index used as the volume DDC fallback.
pub fn save_volume_index(index: usize) {
    let path = volume_index_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, index.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(vendor: u32, model: u32, serial: u32) -> DisplayIdentity {
        DisplayIdentity { vendor, model, serial }
    }

    #[test]
    fn parse_mappings_with_and_without_names() {
        let contents = "1554:31292:16843009=0,Q24P2W1G5\n4268:1234=1\n";
        // second line intentionally malformed (missing a segment) -- confirms it's skipped
        let values = parse_mappings(contents);
        assert_eq!(values.len(), 1);
        assert_eq!(values[&id(1554, 31292, 16843009)].av_index, 0);
        assert_eq!(values[&id(1554, 31292, 16843009)].name.as_deref(), Some("Q24P2W1G5"));
    }

    #[test]
    fn serialize_then_parse_round_trips() {
        let mut values = HashMap::new();
        values.insert(id(1554, 31292, 16843009), DisplayMapping { av_index: 0, name: Some("Q24P2W1G5".to_string()) });
        values.insert(id(4268, 1, 2), DisplayMapping { av_index: 1, name: None });
        let serialized = serialize_mappings(&values);
        let parsed = parse_mappings(&serialized);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[&id(1554, 31292, 16843009)].av_index, 0);
        assert_eq!(parsed[&id(1554, 31292, 16843009)].name.as_deref(), Some("Q24P2W1G5"));
        assert_eq!(parsed[&id(4268, 1, 2)].name, None);
    }

    #[test]
    fn ignores_malformed_lines() {
        let values = parse_mappings("garbage\n1:2:3=notanumber\n");
        assert!(values.is_empty());
    }
}
