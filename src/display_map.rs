// src/display_map.rs
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
    values: HashMap<u32, DisplayMapping>,
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
        PathBuf::from(home).join(".cache").join("mon-osd").join("display-map")
    }

    pub fn get(&self, display_id: u32) -> Option<usize> {
        self.values.get(&display_id).map(|m| m.av_index)
    }

    pub fn all(&self) -> impl Iterator<Item = (&u32, &DisplayMapping)> {
        self.values.iter()
    }

    pub fn set(&mut self, display_id: u32, av_index: usize, name: Option<String>) {
        self.values.insert(display_id, DisplayMapping { av_index, name });
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

fn parse_mappings(contents: &str) -> HashMap<u32, DisplayMapping> {
    let mut values = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((id_str, rest)) = line.split_once('=') else { continue };
        let mut parts = rest.splitn(2, ',');
        let idx_str = parts.next().unwrap_or("");
        let name = parts.next().map(|s| s.to_string()).filter(|s| !s.is_empty());
        if let (Ok(id), Ok(idx)) = (id_str.trim().parse::<u32>(), idx_str.trim().parse::<usize>()) {
            values.insert(id, DisplayMapping { av_index: idx, name });
        }
    }
    values
}

fn serialize_mappings(values: &HashMap<u32, DisplayMapping>) -> String {
    let mut out = String::from("# mon-osd cursor-display -> av-index mapping -- auto-generated\n");
    for (id, m) in values {
        match &m.name {
            Some(n) => out.push_str(&format!("{id}={},{n}\n", m.av_index)),
            None => out.push_str(&format!("{id}={}\n", m.av_index)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mappings_with_and_without_names() {
        let contents = "2=0,Q24P2W1G5\n1=1\n";
        let values = parse_mappings(contents);
        assert_eq!(values[&2].av_index, 0);
        assert_eq!(values[&2].name.as_deref(), Some("Q24P2W1G5"));
        assert_eq!(values[&1].av_index, 1);
        assert_eq!(values[&1].name, None);
    }

    #[test]
    fn serialize_then_parse_round_trips() {
        let mut values = HashMap::new();
        values.insert(2u32, DisplayMapping { av_index: 0, name: Some("Q24P2W1G5".to_string()) });
        values.insert(1u32, DisplayMapping { av_index: 1, name: None });
        let serialized = serialize_mappings(&values);
        let parsed = parse_mappings(&serialized);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[&2].av_index, 0);
        assert_eq!(parsed[&2].name.as_deref(), Some("Q24P2W1G5"));
        assert_eq!(parsed[&1].name, None);
    }

    #[test]
    fn ignores_malformed_lines() {
        let values = parse_mappings("garbage\n2=notanumber\n");
        assert!(values.is_empty());
    }
}
