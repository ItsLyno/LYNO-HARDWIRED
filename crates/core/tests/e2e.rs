//! Author builds a release, a user installs it over HTTP, the author
//! changes one mod, the user updates and downloads only that mod.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use lyno_core::download::Downloader;
use lyno_core::install::{state_path, Event, Installer};
use lyno_core::manifest::Manifest;
use lyno_core::mo2::Instance;
use lyno_core::modlist::{EntryState, ModList};
use lyno_core::package::PackOptions;
use lyno_core::plan::{plan, Action};
use lyno_core::publish::{build, BuildOptions, ModInfo};
use lyno_core::state::State;
use lyno_core::tree::tree_hash;

fn write(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

/// Incompressible bytes, so packages really get split into parts.
fn noise(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn author_instance(root: &Path) {
    write(root, "ModOrganizer.exe", b"MZ fake");
    write(root, "portable.txt", b"");
    write(root, "ModOrganizer.ini", b"[General]\r\ngamePath=@ByteArray(D:/Author/Cyberpunk 2077)\r\n");
    write(root, "plugins/old_plugin.py", b"# dropped in v2");
    write(root, "profiles/LYNO/settings.ini", b"[General]\r\nLocalSettings=false\r\n");
    write(root, "profiles/LYNO/UserSettings.json", b"author's graphics");
    write(root, "profiles/LYNO/saves/ManualSave-1/sav.dat", b"author's save");
    write(root, "profiles/LYNO/modlist.txt", b"# x\r\n+REDmod Thing\r\n+Archive Mod\r\n+CET\r\n-Core_separator\r\n*DLC: EP1\r\n");
    write(root, "profiles/Private/modlist.txt", b"+Secret\r\n");
    write(root, "downloads/cet.zip", b"not shipped");
    write(root, "overwrite/r6/cache/x", b"not shipped");
    write(root, "mods/Core_separator/meta.ini", b"");
    write(root, "mods/CET/meta.ini", b"[General]\nmodid=107\nversion=1.35\ngameName=cyberpunk2077\n");
    write(root, "mods/CET/bin/x64/plugins/cyber_engine_tweaks.asi", &noise(300_000, 1));
    write(root, "mods/Archive Mod/meta.ini", b"[General]\nmodid=555\nversion=2.0\n");
    write(root, "mods/Archive Mod/archive/pc/mod/a.archive", &noise(200_000, 2));
    write(root, "mods/REDmod Thing/meta.ini", b"[General]\nmodid=777\n");
    write(root, "mods/REDmod Thing/mods/Thing/info.json", b"{\"name\":\"Thing\"}");
}

/// Static file server with Range support. Returns base URL and a request log.
fn serve(dir: PathBuf) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log2 = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let mut start = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                    start = v.trim().trim_end_matches('-').parse().unwrap();
                }
            }
            let name = request.split_whitespace().nth(1).unwrap().trim_start_matches('/').to_owned();
            log2.lock().unwrap().push(name.clone());
            match std::fs::read(dir.join(&name)) {
                Ok(data) => {
                    let body = &data[start..];
                    let status = if start > 0 { "206 Partial Content" } else { "200 OK" };
                    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                    stream.write_all(body).unwrap();
                }
                Err(_) => {
                    write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                }
            }
        }
    });
    (format!("http://{addr}"), log)
}

fn options(version: &str, base_url: &str, out: &Path, previous: Option<Manifest>) -> BuildOptions {
    BuildOptions {
        name: "LYNO//HARDWIRED".into(),
        profile: "LYNO".into(),
        build_version: version.into(),
        game_version: "2.31".into(),
        mo2_version: "2.5.2".into(),
        base_url: base_url.into(),
        out_dir: out.to_path_buf(),
        previous,
        changelog: vec![],
        pack: PackOptions { part_size: 100 * 1024, zstd_level: 1 },
    }
}

fn install(user: &Instance, manifest: &Manifest) -> Vec<Action> {
    let state = State::load(&state_path(user)).unwrap();
    let current = ModList::load(&user.modlist_path("LYNO")).unwrap_or_default();
    let p = plan(manifest, &state, &current);
    let actions = p.actions.clone();
    let mut events = Vec::new();
    Installer { inst: user, manifest, downloader: &Downloader::new(), cancel: &AtomicBool::new(false) }
        .apply(&p, &mut |e| events.push(e))
        .unwrap();
    assert!(matches!(events.last(), Some(Event::Done)));
    actions
}

#[test]
fn build_install_update() {
    let tmp = tempfile::tempdir().unwrap();
    let author = tmp.path().join("author");
    let release = tmp.path().join("release");
    let user_root = tmp.path().join("user");
    author_instance(&author);
    let (base_url, requests) = serve(release.clone());

    // v1
    let mut no_info = |_: &lyno_core::meta::ModMeta| ModInfo { author: Some("psiberx".into()), title: None };
    let out1 = build(&author, &options("1.0.0", &base_url, &release, None), &mut no_info, &mut |_| {}).unwrap();
    assert!(out1.warnings.is_empty(), "{:?}", out1.warnings);
    let m1 = out1.manifest;
    assert_eq!(m1.mod_specs().count(), 3);
    assert!(m1.redmod, "REDmod mod detected");
    let cet = m1.mod_specs().find(|m| m.id == "cet").unwrap();
    assert!(cet.package.parts.len() > 1, "CET should be split into parts");
    assert_eq!(cet.nexus.as_ref().unwrap().url(), "https://www.nexusmods.com/cyberpunk2077/mods/107");
    assert_eq!(cet.author.as_deref(), Some("psiberx"));

    let user = Instance::new(&user_root);
    let actions = install(&user, &m1);
    assert_eq!(actions.len(), 4, "{actions:?}");

    // Base: MO2 + build profile, but not downloads/overwrite/other profiles.
    assert!(user.is_installed() && user.is_portable());
    assert!(!user_root.join("downloads/cet.zip").exists());
    assert!(!user_root.join("overwrite").exists());
    assert!(!user_root.join("profiles/Private").exists());
    assert!(user_root.join("profiles/LYNO/settings.ini").exists());
    assert!(!user_root.join("profiles/LYNO/UserSettings.json").exists());
    assert!(!user_root.join("profiles/LYNO/saves").exists());
    assert!(user_root.join("plugins/old_plugin.py").exists());
    for name in ["CET", "Archive Mod"] {
        assert_eq!(tree_hash(&user.mods_dir().join(name)).unwrap(), tree_hash(&author.join("mods").join(name)).unwrap());
    }
    assert!(user.mods_dir().join("Core_separator").is_dir());
    let managed = |l: ModList| l.entries.into_iter().filter(|e| e.state != EntryState::Unmanaged).collect::<Vec<_>>();
    assert_eq!(
        managed(ModList::load(&user.modlist_path("LYNO")).unwrap()),
        managed(ModList::load(&author.join("profiles/LYNO/modlist.txt")).unwrap())
    );
    user.set_game_path(Path::new(r"C:\Games\Cyberpunk 2077")).unwrap();
    assert!(std::fs::read_to_string(user.ini_path()).unwrap().contains("gamePath=@ByteArray(C:/Games/Cyberpunk 2077)"));

    // The user adds their own mod; updates must keep it.
    write(&user_root, "mods/My Tweak/x.archive", b"mine");
    let mut list = ModList::load(&user.modlist_path("LYNO")).unwrap();
    list.entries.push(lyno_core::modlist::Entry::enabled("My Tweak"));
    list.save(&user.modlist_path("LYNO")).unwrap();

    // The user's game already created a mod settings file in overwrite.
    write(&user_root, "overwrite/red4ext/plugins/mod_settings/user.ini", b"player default");

    // v2: the archive mod changes; MO2 touched CET's meta.ini on an update
    // check; the game wrote logs and caches; mod settings were moved from
    // overwrite into a settings mod; an MO2 plugin was dropped.
    write(&author, "mods/Archive Mod/archive/pc/mod/a.archive", &noise(200_000, 3));
    write(&author, "mods/CET/meta.ini", b"[General]\nmodid=107\nversion=1.35\ngameName=cyberpunk2077\nlastNexusQuery=2026-10-04\n");
    write(&author, "mods/CET/bin/x64/plugins/cyber_engine_tweaks/cyber_engine_tweaks.log", b"log");
    write(&author, "mods/CET/r6/cache/final.redscripts", b"author's bundle");
    write(&author, "mods/LYNO Settings/red4ext/plugins/mod_settings/user.ini", b"build settings");
    write(&author, "mods/LYNO Settings/meta.ini", b"[LYNO]\nid=lyno-settings\n");
    write(&author, "overwrite/bin/x64/plugins/cyber_engine_tweaks/mods/x/settings.json", b"{}");
    std::fs::remove_file(author.join("plugins/old_plugin.py")).unwrap();
    let mut list = ModList::load(&author.join("profiles/LYNO/modlist.txt")).unwrap();
    list.entries.push(lyno_core::modlist::Entry::enabled("LYNO Settings"));
    list.save(&author.join("profiles/LYNO/modlist.txt")).unwrap();

    let out2 = build(&author, &options("1.1.0", &base_url, &release, Some(m1.clone())), &mut no_info, &mut |_| {}).unwrap();
    let new_assets: std::collections::BTreeSet<_> =
        out2.assets.iter().map(|a| a.file_name.split('-').next().unwrap().to_owned()).collect();
    assert_eq!(new_assets, ["archive", "base", "lyno"].map(String::from).into(), "{:?}", out2.assets);
    let repacked: Vec<_> = out2.repacked.iter().map(|r| (r.name.as_str(), r.changed)).collect();
    assert_eq!(repacked, [("base", true), ("Archive Mod", true), ("LYNO Settings", false)]);
    assert_eq!(out2.warnings.len(), 3, "{:?}", out2.warnings);
    assert!(out2.warnings.iter().any(|w| w.contains("\"CET\": 2 generated file(s)")), "{:?}", out2.warnings);
    assert!(out2.warnings.iter().any(|w| w.starts_with("overwrite/ has 1 file(s)")), "{:?}", out2.warnings);
    let m2 = out2.manifest;

    requests.lock().unwrap().clear();
    let actions = install(&user, &m2);
    assert_eq!(
        actions,
        [
            Action::Base,
            Action::Update { id: "archive-mod".into(), from_folder: "Archive Mod".into() },
            Action::Install { id: "lyno-settings".into() },
        ]
    );
    assert!(requests.lock().unwrap().iter().all(|r| !r.starts_with("cet-")));
    assert!(!user_root.join("plugins/old_plugin.py").exists(), "dropped base file removed");
    assert!(user_root.join("ModOrganizer.exe").exists());
    assert!(!user_root.join("overwrite/red4ext/plugins/mod_settings/user.ini").exists(), "build settings win");
    assert_eq!(
        std::fs::read(user_root.join(".lyno/overwrite-backup/1.1.0/red4ext/plugins/mod_settings/user.ini")).unwrap(),
        b"player default"
    );
    assert_eq!(
        tree_hash(&user.mods_dir().join("Archive Mod")).unwrap(),
        tree_hash(&author.join("mods/Archive Mod")).unwrap()
    );
    assert!(user_root.join("mods/My Tweak/x.archive").exists());
    let list = ModList::load(&user.modlist_path("LYNO")).unwrap();
    assert_eq!(list.entries.last().unwrap().name, "My Tweak");
    assert_eq!(State::load(&state_path(&user)).unwrap().build_version.as_deref(), Some("1.1.0"));
    assert!(!user_root.join(".lyno/cache").exists());
}
