use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/ItsLyno/LYNO-HARDWIRED/main/build/manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Portable MO2 instance the launcher installs and manages.
    pub instance_dir: PathBuf,
    /// Cyberpunk 2077 folder; detected automatically when empty.
    #[serde(default)]
    pub game_dir: Option<PathBuf>,
    /// URL of the published `manifest.json`.
    #[serde(default = "default_manifest_url")]
    pub manifest_url: String,
}

fn default_manifest_url() -> String {
    DEFAULT_MANIFEST_URL.into()
}

impl Settings {
    fn defaults(data_dir: &Path) -> Self {
        Self { instance_dir: data_dir.join("instance"), game_dir: None, manifest_url: default_manifest_url() }
    }

    pub fn load_or_default(path: &Path, data_dir: &Path) -> Self {
        let mut s: Self = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| Self::defaults(data_dir));
        if s.game_dir.is_none() {
            s.game_dir = lyno_core::game::detect().into_iter().next().map(|g| g.path);
        }
        s
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).expect("settings serialize"))
    }
}
