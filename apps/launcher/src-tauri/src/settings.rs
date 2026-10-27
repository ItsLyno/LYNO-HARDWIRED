use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_REPO: &str = "ItsLyno/LYNO-HARDWIRED";
pub const DEFAULT_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/ItsLyno/LYNO-HARDWIRED/main/build/manifest.json";

/// A portable MO2 instance the launcher knows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceEntry {
    pub name: String,
    pub dir: PathBuf,
    /// Installed and updated from `Settings::manifest_url`. Otherwise the
    /// player's own MO2: the launcher never installs the build over it.
    #[serde(default)]
    pub build: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// The instance the launcher works with, one of `instances`; none of them
    /// on a first start, until the player sets one up.
    pub instance_dir: PathBuf,
    #[serde(default)]
    pub instances: Vec<InstanceEntry>,
    /// Cyberpunk 2077 folder; detected automatically when empty.
    #[serde(default)]
    pub game_dir: Option<PathBuf>,
    /// URL of the published `manifest.json`.
    #[serde(default = "default_manifest_url")]
    pub manifest_url: String,
    /// Follow the beta channel: builds arrive here before they are released to everyone.
    #[serde(default)]
    pub beta: bool,
    /// The build author releases from this instance (see `lyno_core::author`):
    /// differences from the published build are their next release, so updates
    /// and repairs, which would undo them, are off.
    #[serde(default)]
    pub author_mode: bool,
    /// GitHub repository the author publishes to.
    #[serde(default = "default_repo")]
    pub author_repo: String,
    /// Where builds are packed; `None`: `release-out` in the launcher's data
    /// folder, next to the default instance rather than inside it.
    #[serde(default)]
    pub author_out_dir: Option<PathBuf>,
    /// How often the launcher looks for new versions of the build, itself and
    /// Nexus mods while it runs; 0: only when asked.
    #[serde(default = "default_update_check_hours")]
    pub update_check_hours: u32,
}

pub const BUILD_NAME: &str = "LYNO//HARDWIRED";

/// Where the build is installed by default; launchers before 0.8 had it there too.
pub fn build_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("instance")
}

fn default_repo() -> String {
    DEFAULT_REPO.into()
}

fn default_update_check_hours() -> u32 {
    6
}

fn default_manifest_url() -> String {
    DEFAULT_MANIFEST_URL.into()
}

impl Settings {
    fn defaults(data_dir: &Path) -> Self {
        Self {
            instance_dir: build_dir(data_dir),
            instances: Vec::new(),
            game_dir: None,
            manifest_url: default_manifest_url(),
            beta: false,
            author_mode: false,
            author_repo: default_repo(),
            author_out_dir: None,
            update_check_hours: default_update_check_hours(),
        }
    }

    pub fn load_or_default(path: &Path, data_dir: &Path) -> Self {
        let loaded: Option<Self> = std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok());
        let mut s = match loaded {
            // Launchers before 0.8 knew only the build's instance.
            Some(mut s) if s.instances.is_empty() => {
                s.instances.push(InstanceEntry { name: BUILD_NAME.into(), dir: s.instance_dir.clone(), build: true });
                s
            }
            Some(s) => s,
            None => Self::defaults(data_dir),
        };
        if s.game_dir.is_none() {
            s.game_dir = lyno_core::game::detect().into_iter().next().map(|g| g.path);
        }
        s
    }

    /// Manifest of the channel the player follows: `manifest-beta.json` next to `manifest.json`.
    /// The author follows beta, where their releases land first.
    pub fn channel_url(&self) -> String {
        match self.manifest_url.strip_suffix("manifest.json") {
            Some(dir) if self.beta || self.author_mode => format!("{dir}manifest-beta.json"),
            _ => self.manifest_url.clone(),
        }
    }

    /// The active instance is the build's.
    pub fn build(&self) -> bool {
        self.active().is_some_and(|i| i.build)
    }

    pub fn active(&self) -> Option<&InstanceEntry> {
        self.instances.iter().find(|i| i.dir == self.instance_dir)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).expect("settings serialize"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_reads_the_beta_manifest_next_to_the_release_one() {
        let mut s = Settings::defaults(Path::new("data"));
        assert_eq!(s.channel_url(), DEFAULT_MANIFEST_URL);
        s.beta = true;
        let beta = DEFAULT_MANIFEST_URL.replace("manifest.json", "manifest-beta.json");
        assert_eq!(s.channel_url(), beta);
        s.beta = false;
        s.author_mode = true;
        assert_eq!(s.channel_url(), beta);
        s.beta = true;
        s.manifest_url = "http://localhost/custom".into();
        assert_eq!(s.channel_url(), "http://localhost/custom", "a custom URL has no channels");
    }
}
