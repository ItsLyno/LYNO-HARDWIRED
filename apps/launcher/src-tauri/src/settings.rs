use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Portable MO2 instance managed by the launcher.
    pub instance_dir: PathBuf,
    /// MO2 profile the build lives in.
    pub profile: String,
    /// URL of the published `manifest.json`.
    pub manifest_url: String,
}

impl Settings {
    fn defaults(data_dir: &Path) -> Self {
        Self {
            instance_dir: data_dir.join("instance"),
            profile: "LYNO".into(),
            manifest_url: "https://github.com/ItsLyno/LYNO-HARDWIRED/releases/latest/download/manifest.json".into(),
        }
    }

    pub fn load_or_default(path: &Path, data_dir: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| Self::defaults(data_dir))
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).expect("settings serialize"))
    }
}
