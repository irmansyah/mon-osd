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
        let mut values = HashMap::new();
        if let Ok(contents) = fs::read_to_string(&path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                // format: <display_id>=<av_index>[,<name>]
                let Some((id_str, rest)) = line.split_once('=') else { continue };
                let mut parts = rest.splitn(2, ',');
                let idx_str = parts.next().unwrap_or("");
                let name = parts.next().map(|s| s.to_string()).filter(|s| !s.is_empty());
                if let (Ok(id), Ok(idx)) = (id_str.trim().parse::<u32>(), idx_str.trim().parse::<usize>()) {
                    values.insert(id, DisplayMapping { av_index: idx, name });
                }
            }
        }
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
        let mut out = String::from("# mon-osd cursor-display -> av-index mapping -- auto-generated\n");
        for (id, m) in &self.values {
            match &m.name {
                Some(n) => out.push_str(&format!("{id}={},{n}\n", m.av_index)),
                None => out.push_str(&format!("{id}={}\n", m.av_index)),
            }
        }
        if let Ok(mut f) = fs::File::create(&self.path) {
            let _ = f.write_all(out.as_bytes());
        }
    }
}
