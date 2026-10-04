//! `nxm://` links: what Nexus Mods hands to a mod manager when the player
//! clicks "Mod Manager Download" on a file.
//!
//! `nxm://cyberpunk2077/mods/107/files/12345?key=…&expires=…&user_id=…`.
//! `key` and `expires` are a one-time permission to fetch that file: without
//! them the API gives download links to Premium accounts only, which is how
//! free accounts download through a mod manager at all.

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NxmLink {
    /// Nexus game domain, lowercase (`cyberpunk2077`).
    pub game: String,
    pub mod_id: u64,
    pub file_id: u64,
    /// The one-time permission of a free account; `None` for links the
    /// launcher makes for Premium accounts.
    pub key: Option<String>,
    pub expires: Option<u64>,
}

impl NxmLink {
    pub fn parse(url: &str) -> Result<Self> {
        let bad = |why: &str| Error::Manifest(format!("{why}: {url}"));
        let rest = url
            .trim()
            .strip_prefix("nxm://")
            .or_else(|| url.trim().strip_prefix("NXM://"))
            .ok_or_else(|| bad("not an nxm:// link"))?;
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
        let id = |s: &str| s.parse::<u64>().ok().filter(|&n| n > 0);
        let (game, mod_id, file_id) = match parts.as_slice() {
            [game, "mods", m, "files", f] => (game, id(m), id(f)),
            [_, "collections", ..] => return Err(bad("collections are not supported")),
            _ => return Err(bad("unknown nxm:// link")),
        };
        let (Some(mod_id), Some(file_id)) = (mod_id, file_id) else { return Err(bad("bad mod or file id")) };

        let mut key = None;
        let mut expires = None;
        for pair in query.split('&') {
            match pair.split_once('=') {
                Some(("key", v)) if !v.is_empty() => key = Some(v.to_owned()),
                Some(("expires", v)) => expires = v.parse().ok(),
                _ => {}
            }
        }
        Ok(Self { game: game.to_lowercase(), mod_id, file_id, key, expires })
    }

    pub fn is_collection(url: &str) -> bool {
        url.split('?').next().is_some_and(|p| p.split('/').nth(3) == Some("collections"))
    }
}

/// The page of a mod's files on Nexus, scrolled to `file_id`: where a free
/// account clicks "Mod Manager Download" to send the launcher an nxm link.
pub fn file_page(game: &str, mod_id: u64, file_id: Option<u64>) -> String {
    match file_id {
        Some(f) => format!("https://www.nexusmods.com/{game}/mods/{mod_id}?tab=files&file_id={f}"),
        None => format!("https://www.nexusmods.com/{game}/mods/{mod_id}?tab=files"),
    }
}

/// Whether the launcher receives nxm links, and who does otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandlerStatus {
    /// Registering is possible here (Windows).
    pub supported: bool,
    pub ours: bool,
    /// Program name of the current handler when it isn't the launcher
    /// (`nxmhandler.exe` of MO2, `Vortex.exe`).
    pub other: Option<String>,
}

/// The command Windows runs for a link.
pub fn handler_command(exe: &std::path::Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Program path of a handler command: quoted or up to the first space.
pub fn command_exe(command: &str) -> Option<&str> {
    let c = command.trim();
    let exe = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => c.split(' ').next()?,
    };
    Some(exe).filter(|e| !e.is_empty())
}

fn status_of(command: Option<&str>, exe: &std::path::Path) -> HandlerStatus {
    let current = command.and_then(command_exe);
    let ours = current.is_some_and(|c| c.eq_ignore_ascii_case(&exe.to_string_lossy()));
    let other = current
        .filter(|_| !ours)
        .map(|c| c.rsplit(['\\', '/']).next().unwrap_or(c).to_owned());
    HandlerStatus { supported: cfg!(windows), ours, other }
}

/// Registering takes over nxm links from MO2 or Vortex for the whole user
/// account, so the launcher does it only on request, remembers the previous
/// command and puts it back on [`unregister`].
#[cfg(windows)]
mod platform {
    use std::path::{Path, PathBuf};

    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    use crate::{Error, Result};

    const KEY: &str = r"Software\Classes\nxm";
    const COMMAND: &str = r"shell\open\command";
    /// Where the previous handler's command waits to be put back.
    const PREVIOUS: &str = "LynoPreviousCommand";

    fn err(e: std::io::Error) -> Error {
        Error::io(PathBuf::from(format!(r"HKCU\{KEY}")), e)
    }

    pub fn current() -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(r"{KEY}\{COMMAND}")).ok()?.get_value("").ok()
    }

    pub fn register(exe: &Path) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(KEY).map_err(err)?;
        let ours = super::handler_command(exe);
        if let Some(previous) = current().filter(|c| super::command_exe(c) != super::command_exe(&ours)) {
            key.set_value(PREVIOUS, &previous).map_err(err)?;
        }
        key.set_value("", &"URL:NXM Protocol").map_err(err)?;
        key.set_value("URL Protocol", &"").map_err(err)?;
        let (command, _) = key.create_subkey(COMMAND).map_err(err)?;
        command.set_value("", &ours).map_err(err)
    }

    pub fn unregister() -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey_with_flags(KEY, winreg::enums::KEY_ALL_ACCESS) else { return Ok(()) };
        match key.get_value::<String, _>(PREVIOUS) {
            Ok(previous) => {
                let (command, _) = key.create_subkey(COMMAND).map_err(err)?;
                command.set_value("", &previous).map_err(err)?;
                key.delete_value(PREVIOUS).map_err(err)
            }
            Err(_) => hkcu.delete_subkey_all(KEY).map_err(err),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;

    use crate::{Error, Result};

    pub fn current() -> Option<String> {
        None
    }

    pub fn register(_exe: &Path) -> Result<()> {
        Err(Error::Manifest("nxm links can only be registered on Windows".into()))
    }

    pub fn unregister() -> Result<()> {
        Ok(())
    }
}

pub fn handler_status(exe: &std::path::Path) -> HandlerStatus {
    status_of(platform::current().as_deref(), exe)
}

pub fn register(exe: &std::path::Path) -> Result<()> {
    platform::register(exe)
}

/// Gives nxm links back to the handler that had them before, if any.
pub fn unregister() -> Result<()> {
    platform::unregister()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_free_account_link() {
        let l = NxmLink::parse("nxm://Cyberpunk2077/mods/107/files/12345?key=abc&expires=1700000000&user_id=9").unwrap();
        assert_eq!(l.game, "cyberpunk2077");
        assert_eq!((l.mod_id, l.file_id), (107, 12345));
        assert_eq!(l.key.as_deref(), Some("abc"));
        assert_eq!(l.expires, Some(1_700_000_000));
    }

    #[test]
    fn parses_link_without_key() {
        let l = NxmLink::parse("nxm://cyberpunk2077/mods/107/files/12345").unwrap();
        assert_eq!((l.key, l.expires), (None, None));
    }

    #[test]
    fn reads_handler_commands() {
        let exe = std::path::Path::new(r"C:\Program Files\LYNO HARDWIRED\lyno-hardwired.exe");
        let ours = handler_command(exe);
        assert_eq!(ours, r#""C:\Program Files\LYNO HARDWIRED\lyno-hardwired.exe" "%1""#);
        assert!(status_of(Some(&ours), exe).ours);
        let mo2 = status_of(Some(r#""D:\MO2\nxmhandler.exe" "%1""#), exe);
        assert_eq!((mo2.ours, mo2.other.as_deref()), (false, Some("nxmhandler.exe")));
        assert_eq!(command_exe(r"C:\Vortex\Vortex.exe -d %1"), Some(r"C:\Vortex\Vortex.exe"));
        assert_eq!(status_of(None, exe), HandlerStatus { supported: cfg!(windows), ours: false, other: None });
    }

    #[test]
    fn rejects_collections_and_garbage() {
        let c = "nxm://cyberpunk2077/collections/abcdef/revisions/3";
        assert!(NxmLink::is_collection(c));
        assert!(NxmLink::parse(c).unwrap_err().to_string().contains("collections"));
        assert!(NxmLink::parse("https://nexusmods.com").is_err());
        assert!(NxmLink::parse("nxm://cyberpunk2077/mods/x/files/1").is_err());
        assert!(NxmLink::parse("nxm://cyberpunk2077/mods/1/files/0").is_err());
    }
}
