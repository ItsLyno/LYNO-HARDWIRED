//! The Coop profile: a second MO2 profile of the build's instance with exactly the mods the author
//! has on in Coop. NightCoop only reports differences in mod sets between players, so the launcher
//! makes them equal: before every Coop start the profile's list is written from the manifest, and
//! what the player has installed or switched on themselves stays off in it.

use crate::manifest::{Manifest, COOP_PROFILE};
use crate::modlist::ModList;
use crate::mo2::Instance;
use crate::plan::coop_modlist;
use crate::state::State;
use crate::{rules, Error, Result};

/// Which profile the launcher works with is the player's choice: Coop is on while this marker is in
/// the instance's launcher folder.
fn marker(inst: &Instance) -> std::path::PathBuf {
    inst.root().join(".lyno").join("coop")
}

pub fn is_active(inst: &Instance) -> bool {
    marker(inst).is_file()
}

pub fn set_active(inst: &Instance, active: bool) -> Result<()> {
    let path = marker(inst);
    if active {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        std::fs::write(&path, b"").map_err(|e| Error::io(&path, e))
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(Error::io(&path, e)),
            _ => Ok(()),
        }
    }
}

/// Makes the Coop profile if it isn't there and writes its mod list. The other profile files
/// (`settings.ini`, `plugins.txt`, …) are copied from the build's profile once: MO2 keeps
/// writing them, and the player's saves and graphics settings are not copied (`is_private_profile_file`).
pub fn prepare(inst: &Instance, manifest: &Manifest, state: &State) -> Result<()> {
    if !manifest.has_coop() {
        return Err(Error::Manifest("В этой сборке нет набора модов для Coop".into()));
    }
    let from = inst.profile_dir(&manifest.profile);
    let to = inst.profile_dir(COOP_PROFILE);
    std::fs::create_dir_all(&to).map_err(|e| Error::io(&to, e))?;
    if let Ok(files) = std::fs::read_dir(&from) {
        for f in files.flatten().filter(|f| f.file_type().is_ok_and(|t| t.is_file())) {
            let name = f.file_name();
            let target = to.join(&name);
            if !rules::is_private_profile_file(&name.to_string_lossy()) && !target.exists() {
                std::fs::copy(f.path(), &target).map_err(|e| Error::io(&target, e))?;
            }
        }
    }
    let current = ModList::load(&inst.modlist_path(&manifest.profile)).unwrap_or_default();
    let on_disk: Vec<String> = inst.scan_mods()?.into_keys().collect();
    let list = coop_modlist(manifest, state, &current, &on_disk);
    let path = inst.modlist_path(COOP_PROFILE);
    if ModList::load(&path).ok().as_ref() != Some(&list) {
        list.save(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::{manifest, spec};
    use crate::manifest::{ModEntry, ModSpec};
    use crate::modlist::Entry;
    use crate::state::InstalledMod;

    fn write(root: &std::path::Path, rel: &str, data: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    #[test]
    fn coop_is_a_marker_in_the_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let inst = Instance::new(tmp.path());
        assert!(!is_active(&inst));
        set_active(&inst, false).unwrap();
        set_active(&inst, true).unwrap();
        assert!(is_active(&inst));
        set_active(&inst, false).unwrap();
        assert!(!is_active(&inst));
    }

    #[test]
    fn the_profile_is_made_from_the_build_and_rewritten_before_each_start() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "profiles/LYNO/modlist.txt", "+Mine\r\n-cet folder\r\n");
        write(root, "profiles/LYNO/settings.ini", "[General]\r\nLocalSaves=false\r\n");
        write(root, "profiles/LYNO/UserSettings.json", "my graphics");
        write(root, "profiles/LYNO/saves/ManualSave-1/sav.dat", "my save");
        std::fs::create_dir_all(root.join("mods/Mine")).unwrap();
        std::fs::create_dir_all(root.join("mods/cet folder")).unwrap();
        let inst = Instance::new(root);
        let m = manifest(vec![ModEntry::Mod(ModSpec { coop: Some(true), ..spec("cet", "h") })]);
        let mut st = State::default();
        st.mods.insert("cet".into(), InstalledMod { folder: "cet folder".into(), hash: "h".into(), damaged: false, reset_settings: false });

        prepare(&inst, &m, &st).unwrap();
        let coop = root.join("profiles/Coop");
        assert_eq!(std::fs::read_to_string(coop.join("settings.ini")).unwrap(), "[General]\r\nLocalSaves=false\r\n");
        assert!(!coop.join("UserSettings.json").exists() && !coop.join("saves").exists(), "the player's private files stay theirs");
        assert_eq!(ModList::load(&coop.join("modlist.txt")).unwrap().entries, [Entry::disabled("Mine"), Entry::enabled("cet folder")]);

        // The player turns their mod on in MO2's Coop profile; the next start puts it back.
        write(root, "profiles/Coop/modlist.txt", "+Mine\r\n+cet folder\r\n");
        write(root, "profiles/Coop/settings.ini", "MO2 changed it");
        prepare(&inst, &m, &st).unwrap();
        assert_eq!(ModList::load(&coop.join("modlist.txt")).unwrap().entries, [Entry::disabled("Mine"), Entry::enabled("cet folder")]);
        assert_eq!(std::fs::read_to_string(coop.join("settings.ini")).unwrap(), "MO2 changed it", "other files are made once");
        assert_eq!(std::fs::read_to_string(root.join("profiles/LYNO/modlist.txt")).unwrap(), "+Mine\r\n-cet folder\r\n", "the build's profile is untouched");

        let no_coop = manifest(vec![ModEntry::Mod(spec("cet", "h"))]);
        assert!(prepare(&inst, &no_coop, &st).is_err());
    }
}
