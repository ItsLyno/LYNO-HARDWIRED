//! Author builds a release, a user installs it over HTTP, the author
//! changes one mod, the user updates and downloads only that mod.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use lyno_core::author::{adopt, pending, Pending};
use lyno_core::download::Downloader;
use lyno_core::install::{state_path, Event, Installer};
use lyno_core::manifest::Manifest;
use lyno_core::meta::ModMeta;
use lyno_core::mo2::Instance;
use lyno_core::modlist::{EntryState, ModList};
use lyno_core::package::PackOptions;
use lyno_core::plan::{plan, set_enabled, Action};
use lyno_core::publish::{build, check_published, BuildOptions, ModInfo};
use lyno_core::release::{publish, release_tag, Host, PublishOptions};
use lyno_core::state::State;
use lyno_core::tree::tree_hash;
use lyno_core::verify::{mark, verify, Problem};

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
    write(root, "profiles/LYNO/modlist.txt", b"# x\r\n+REDmod Thing\r\n-Extras_separator\r\n+Archive Mod\r\n+CET\r\n-Core_separator\r\n*DLC: EP1\r\n");
    write(root, "profiles/Private/modlist.txt", b"+Secret\r\n");
    write(root, "downloads/cet.zip", b"not shipped");
    write(root, "overwrite/r6/cache/x", b"not shipped");
    // The separator color as MO2 writes it: #0393e1.
    write(root, "mods/Core_separator/meta.ini", br"[General]
color=@Variant(\0\0\0\x43\x1\xff\xff\x3\x3\x93\x93\xe1\xe1\0\0)
[LYNO]
core=true
");
    std::fs::create_dir_all(root.join("mods/Extras_separator")).unwrap();
    write(root, "mods/CET/meta.ini", b"[General]\nmodid=107\nversion=1.35\ngameName=cyberpunk2077\n");
    write(root, "mods/CET/bin/x64/plugins/cyber_engine_tweaks.asi", &noise(300_000, 1));
    write(root, "mods/CET/bin/x64/plugins/cyber_engine_tweaks/bindings.json", b"{\"overlay\": \"F1\"}");
    write(root, "mods/Archive Mod/meta.ini", b"[General]\nmodid=555\nversion=2.0\n[LYNO]\noptional=true\n");
    write(root, "mods/Archive Mod/archive/pc/mod/a.archive", &noise(200_000, 2));
    write(root, "mods/REDmod Thing/meta.ini", b"[General]\nmodid=777\n");
    write(root, "mods/REDmod Thing/mods/Thing/info.json", b"{\"name\":\"Thing\"}");
}

/// Static file server with Range support. Returns base URL and a request log
/// (`name`, or `name@start` for a Range request).
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
            log2.lock().unwrap().push(if start > 0 { format!("{name}@{start}") } else { name.clone() });
            match std::fs::read(dir.join(&name)) {
                Ok(data) => {
                    let body = &data[start..];
                    let status = if start > 0 { "206 Partial Content" } else { "200 OK" };
                    write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                    if !request.starts_with("HEAD ") {
                        stream.write_all(body).unwrap();
                    }
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
        hash_cache: true,
        cancel: None,
    }
}

/// Stands in for `gh release upload`: `out/` is the author's local staging
/// folder, `release` is what the server hosts (assets of every release).
fn upload(out: &lyno_core::publish::BuildOutput, release: &Path) {
    std::fs::create_dir_all(release).unwrap();
    for a in &out.assets {
        std::fs::copy(&a.path, release.join(&a.file_name)).unwrap();
    }
}

fn try_install(user: &Instance, manifest: &Manifest) -> (Vec<Action>, lyno_core::Result<()>) {
    let state = State::load(&state_path(user)).unwrap();
    let current = ModList::load(&user.modlist_path("LYNO")).unwrap_or_default();
    let p = plan(manifest, &state, &current);
    let actions = p.actions.clone();
    let mut events = Vec::new();
    let result = Installer { inst: user, manifest, downloader: &Downloader::new(), cancel: &AtomicBool::new(false) }
        .apply(&p, &mut |e| events.push(e));
    if result.is_ok() {
        assert!(matches!(events.last(), Some(Event::Done)));
        let Some(Event::Bytes { done, total }) = events.iter().rev().find(|e| matches!(e, Event::Bytes { .. })) else {
            panic!("{events:?}")
        };
        assert_eq!(done, total, "progress ends at 100%");
    }
    (actions, result)
}

fn install(user: &Instance, manifest: &Manifest) -> Vec<Action> {
    let (actions, result) = try_install(user, manifest);
    result.unwrap();
    actions
}

/// A download fails halfway through a first install (here: an asset is
/// missing; a lost connection that outlasts the retries ends the same way).
/// What was complete is installed, what was downloaded stays, and the next
/// run continues instead of starting over.
#[test]
fn interrupted_install_resumes() {
    let tmp = tempfile::tempdir().unwrap();
    let author = tmp.path().join("author");
    let release = tmp.path().join("release");
    author_instance(&author);
    let (base_url, requests) = serve(release.clone());
    let mut no_info = |_: &lyno_core::meta::ModMeta| ModInfo { author: None, title: None };
    let out = build(&author, &options("1.0.0", &base_url, &tmp.path().join("out"), None), &mut no_info, &mut |_| {}).unwrap();
    upload(&out, &release);
    let m = out.manifest;

    let archive = m.mod_specs().find(|s| s.id == "archive-mod").unwrap();
    let asset = archive.package.parts[0].url.rsplit('/').next().unwrap().to_owned();
    let hidden = tmp.path().join("hidden");
    std::fs::rename(release.join(&asset), &hidden).unwrap();

    let user = Instance::new(tmp.path().join("user"));
    let (actions, result) = try_install(&user, &m);
    assert_eq!(
        actions,
        [
            Action::Base,
            Action::Install { id: "cet".into() },
            Action::Install { id: "archive-mod".into() },
            Action::Install { id: "redmod-thing".into() },
        ]
    );
    let err = result.unwrap_err();
    assert!(err.to_string().contains("HTTP 404"), "{err}");
    let st = State::load(&state_path(&user)).unwrap();
    assert!(st.base_hash.is_some());
    assert_eq!(st.mods.keys().collect::<Vec<_>>(), ["cet"], "everything before the failure is installed");
    assert_eq!(st.build_version, None);

    // Half of the missing part arrived in an earlier session.
    let data = std::fs::read(&hidden).unwrap();
    let partial = user
        .root()
        .join(".lyno/cache")
        .join(format!("archive-mod-{}.001.partial", &archive.package.hash[..16]));
    std::fs::write(&partial, &data[..data.len() / 2]).unwrap();
    std::fs::rename(&hidden, release.join(&asset)).unwrap();
    let state = State::load(&state_path(&user)).unwrap();
    let p = plan(&m, &state, &ModList::load(&user.modlist_path("LYNO")).unwrap_or_default());
    assert!(lyno_core::install::cached_bytes(&user, &m, &p) >= (data.len() / 2) as u64);

    requests.lock().unwrap().clear();
    assert_eq!(install(&user, &m), [Action::Install { id: "archive-mod".into() }, Action::Install { id: "redmod-thing".into() }]);
    let log = requests.lock().unwrap().clone();
    assert!(log.iter().all(|r| !r.starts_with("cet-") && !r.starts_with("base-")), "{log:?}");
    assert!(log.contains(&format!("{asset}@{}", data.len() / 2)), "resumed, not restarted: {log:?}");
    assert!(!log.contains(&asset), "{log:?}");
    let st = State::load(&state_path(&user)).unwrap();
    assert_eq!(st.build_version.as_deref(), Some("1.0.0"));
    assert_eq!(
        tree_hash(&user.mods_dir().join("Archive Mod")).unwrap(),
        tree_hash(&author.join("mods/Archive Mod")).unwrap()
    );
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
    let out = tmp.path().join("out");
    let out1 = build(&author, &options("1.0.0", &base_url, &out, None), &mut no_info, &mut |_| {}).unwrap();
    upload(&out1, &release);
    assert!(out1.warnings.is_empty(), "{:?}", out1.warnings);
    let m1 = out1.manifest;
    assert_eq!(m1.mod_specs().count(), 3);
    assert!(m1.redmod, "REDmod mod detected");
    let cet = m1.mod_specs().find(|m| m.id == "cet").unwrap();
    assert!(cet.package.parts.len() > 1, "CET should be split into parts");
    assert_eq!(cet.nexus.as_ref().unwrap().url(), "https://www.nexusmods.com/cyberpunk2077/mods/107");
    assert_eq!(cet.author.as_deref(), Some("psiberx"));
    // Core_separator is `[LYNO] core=true`; the archive mod under it is `optional=true`.
    assert!(!cet.optional);
    assert!(m1.mod_specs().find(|m| m.id == "archive-mod").unwrap().optional);
    assert!(m1.mod_specs().find(|m| m.id == "redmod-thing").unwrap().optional, "outside the core section");

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
    let separator = ModMeta::load(&user.mods_dir().join("Core_separator/meta.ini")).unwrap();
    assert_eq!(separator.color.as_deref(), Some("#0393e1"));
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

    let st = State::load(&state_path(&user)).unwrap();
    assert_eq!(st.last_update.as_ref().unwrap().from, None, "first install");

    // The player switches the optional mod off in the launcher; core mods can't be.
    let mut list = ModList::load(&user.modlist_path("LYNO")).unwrap();
    set_enabled(&m1, &st, &mut list, "archive-mod", false).unwrap();
    assert!(set_enabled(&m1, &st, &mut list, "cet", false).is_err());
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

    let out2 = build(&author, &options("1.1.0", &base_url, &out, Some(m1.clone())), &mut no_info, &mut |_| {}).unwrap();
    upload(&out2, &release);
    // Only this build's assets stay in out/; 1.0.0's live in the older release.
    let staged: std::collections::BTreeSet<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tar.zst."))
        .collect();
    assert_eq!(staged, out2.assets.iter().map(|a| a.file_name.clone()).collect());
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
    assert_eq!(list.get("Archive Mod").unwrap().state, EntryState::Disabled, "player's choice survives the update");
    assert_eq!(list.get("CET").unwrap().state, EntryState::Enabled);
    let st = State::load(&state_path(&user)).unwrap();
    assert_eq!(st.build_version.as_deref(), Some("1.1.0"));
    let record = st.last_update.unwrap();
    assert_eq!(record.from.as_deref(), Some("1.0.0"));
    assert_eq!(record.added, ["lyno-settings"]);
    assert_eq!(record.updated, ["archive-mod"]);
    assert!(!user_root.join(".lyno/cache").exists());

    // Integrity check: a fresh install is intact even after MO2 and the game
    // touched it. The player rebinds CET (a setting, not damage); antivirus eats
    // CET's .asi and MO2's exe. Repair downloads only those again, keeps the
    // binding and doesn't count as an update; a repair with reset brings back
    // the build's binding.
    let check_integrity = || verify(&user, &State::load(&state_path(&user)).unwrap(), &AtomicBool::new(false), &mut |_| {}).unwrap();
    let repair = |ids: &[&str], base: bool, reset: bool| {
        let mut st = State::load(&state_path(&user)).unwrap();
        mark(&mut st, &ids.iter().map(|s| s.to_string()).collect::<Vec<_>>(), base, reset);
        st.save(&state_path(&user)).unwrap();
        install(&user, &m2)
    };
    let bindings = user_root.join("mods/CET/bin/x64/plugins/cyber_engine_tweaks/bindings.json");
    write(&user_root, "mods/CET/meta.ini", b"[General]\nlastNexusQuery=2026-10-05\n");
    write(&user_root, "mods/CET/bin/x64/plugins/cyber_engine_tweaks/cyber_engine_tweaks.log", b"log");
    let report = check_integrity();
    assert_eq!((report.damaged.len(), report.customized.len()), (0, 0), "{report:?}");
    std::fs::write(&bindings, b"{\"overlay\": \"F2\"}").unwrap();
    std::fs::remove_file(user_root.join("mods/CET/bin/x64/plugins/cyber_engine_tweaks.asi")).unwrap();
    std::fs::remove_file(user_root.join("ModOrganizer.exe")).unwrap();
    let report = check_integrity();
    assert_eq!(report.checked, 4);
    assert_eq!(report.damaged.len(), 2, "{report:?}");
    let Problem::Files { missing, .. } = &report.damaged[0].problem else { panic!("{report:?}") };
    assert_eq!(missing.sample, ["ModOrganizer.exe"]);
    let Problem::Files { missing, changed, added } = &report.damaged[1].problem else { panic!("{report:?}") };
    assert_eq!(report.damaged[1].id.as_deref(), Some("cet"));
    assert_eq!((missing.sample.as_slice(), changed.count, added.count), (&["bin/x64/plugins/cyber_engine_tweaks.asi".to_string()][..], 0, 0));
    assert_eq!(report.customized[0].files.sample, ["bin/x64/plugins/cyber_engine_tweaks/bindings.json"]);

    requests.lock().unwrap().clear();
    assert_eq!(repair(&["cet"], true, false), [Action::Base, Action::Repair { id: "cet".into(), from_folder: "CET".into() }]);
    assert!(requests.lock().unwrap().iter().all(|r| r.starts_with("cet-") || r.starts_with("base-")), "{requests:?}");
    let report = check_integrity();
    assert_eq!(report.damaged, []);
    assert_eq!(report.customized.len(), 1, "the binding is still the player's");
    assert_eq!(std::fs::read(&bindings).unwrap(), b"{\"overlay\": \"F2\"}");
    assert!(user_root.join("ModOrganizer.exe").exists());
    let st = State::load(&state_path(&user)).unwrap();
    assert!(!st.base_damaged && !st.mods["cet"].damaged);
    assert_eq!(st.last_update.as_ref().unwrap().updated, ["archive-mod"], "repair is not an update");
    let list = ModList::load(&user.modlist_path("LYNO")).unwrap();
    assert_eq!(list.get("Archive Mod").unwrap().state, EntryState::Disabled, "player's choice survives a repair");
    assert!(user_root.join("mods/My Tweak/x.archive").exists());

    assert_eq!(repair(&["cet"], false, true), [Action::Repair { id: "cet".into(), from_folder: "CET".into() }]);
    assert_eq!(std::fs::read(&bindings).unwrap(), b"{\"overlay\": \"F1\"}");
    let report = check_integrity();
    assert_eq!((report.damaged.len(), report.customized.len()), (0, 0), "{report:?}");
    assert!(!State::load(&state_path(&user)).unwrap().mods["cet"].reset_settings);

    // Before publishing: every part, including ones reused from 1.0.0, is
    // reachable with the right size; a missing or truncated asset is caught.
    let check = |m: &Manifest| check_published(m, &Downloader::new(), &|_, _| {});
    assert_eq!(check(&m2), Vec::<String>::new());
    let cet_part = &m2.mod_specs().find(|m| m.id == "cet").unwrap().package.parts[0];
    let file = |url: &str| release.join(url.rsplit('/').next().unwrap());
    std::fs::write(file(&cet_part.url), b"short").unwrap();
    std::fs::remove_file(file(&m2.base.parts[0].url)).unwrap();
    let errors = check(&m2);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("HTTP 404")), "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("5 bytes, expected")), "{errors:?}");

    // A run interrupted after some packages were written: the rerun picks them
    // up instead of packing again, and drops whatever else is in out/.
    let out3 = tmp.path().join("out3");
    let first = build(&author, &options("2.0.0", &base_url, &out3, None), &mut no_info, &mut |_| {}).unwrap();
    std::fs::remove_file(out3.join("manifest.json")).ok();
    write(&out3, "old-0123456789abcdef.tar.zst.001", b"stale part");
    write(&out3, "cet.packing.tar.zst.001", b"half-written part");
    let mut logs = Vec::new();
    let again = build(&author, &options("2.0.0", &base_url, &out3, None), &mut no_info, &mut |m| logs.push(m.to_owned())).unwrap();
    assert!(logs.iter().all(|l| !l.contains("packing") && !l.contains("hashing")), "{logs:?}");
    assert!(logs.iter().any(|l| l.contains("already packed")), "{logs:?}");
    assert_eq!(again.manifest, first.manifest);
    let names = |o: &lyno_core::publish::BuildOutput| o.assets.iter().map(|a| a.file_name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&again), names(&first));
    assert!(!out3.join("old-0123456789abcdef.tar.zst.001").exists());
    assert!(!out3.join("cet.packing.tar.zst.001").exists());

    // Next release with nothing changed: every hash comes from the cache.
    let mut logs = Vec::new();
    let next = build(&author, &options("2.0.1", &base_url, &out3, Some(again.manifest)), &mut no_info, &mut |m| {
        logs.push(m.to_owned())
    })
    .unwrap();
    assert!(next.assets.is_empty(), "{:?}", next.assets);
    assert!(logs.is_empty(), "nothing read or packed: {logs:?}");
    assert!(std::fs::read_dir(&out3).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains(".tar.zst.")));
}

/// Stands in for GitHub: releases are folders under the served directory, the
/// published manifest is a string. `lose` names an asset whose upload
/// "succeeds" without the file arriving.
struct FakeHost {
    releases: PathBuf,
    published: Option<String>,
    uploads: Vec<String>,
    lose: Option<String>,
}

impl Host for FakeHost {
    fn published_manifest(&mut self) -> lyno_core::Result<Option<String>> {
        Ok(self.published.clone())
    }

    fn release_assets(&mut self, tag: &str) -> lyno_core::Result<Option<std::collections::HashMap<String, u64>>> {
        let Ok(entries) = std::fs::read_dir(self.releases.join(tag)) else { return Ok(None) };
        Ok(Some(entries.map(|e| e.unwrap()).map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().unwrap().len())).collect()))
    }

    fn create_release(&mut self, tag: &str, _title: &str, _notes: &str) -> lyno_core::Result<()> {
        std::fs::create_dir_all(self.releases.join(tag)).unwrap();
        Ok(())
    }

    fn upload_asset(&mut self, tag: &str, path: &Path, _progress: &dyn Fn(u64)) -> lyno_core::Result<()> {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        self.uploads.push(name.clone());
        if self.lose.as_ref() != Some(&name) {
            std::fs::copy(path, self.releases.join(tag).join(&name)).unwrap();
        }
        Ok(())
    }

    fn push_manifest(&mut self, _version: &str, path: &Path) -> lyno_core::Result<()> {
        self.published = Some(std::fs::read_to_string(path).unwrap());
        Ok(())
    }
}

/// The author plays in their own launcher instance: mods under `LYNO USER
/// MODS` are theirs and stay out of the build. Publishing uploads what is
/// missing and makes the manifest live only once every part downloads and the
/// author confirms; a re-run after a failure uploads only what is still missing.
#[test]
fn personal_mods_stay_out_and_publish_goes_live_last() {
    let tmp = tempfile::tempdir().unwrap();
    let author = tmp.path().join("author");
    let releases = tmp.path().join("releases");
    let out = tmp.path().join("out");
    author_instance(&author);
    std::fs::create_dir_all(author.join("mods/LYNO USER MODS_separator")).unwrap();
    write(&author, "mods/My Test/archive/pc/mod/test.archive", b"experiment");
    let mut list = ModList::load(&author.join("profiles/LYNO/modlist.txt")).unwrap();
    list.entries.push(lyno_core::modlist::Entry::separator("LYNO USER MODS"));
    list.entries.push(lyno_core::modlist::Entry::enabled("My Test"));
    list.save(&author.join("profiles/LYNO/modlist.txt")).unwrap();

    let (root, _) = serve(releases.clone());
    let base_url = format!("{root}/{}", release_tag("1.0.0"));
    let mut no_info = |_: &ModMeta| ModInfo::default();
    let built = build(&author, &options("1.0.0", &base_url, &out, None), &mut no_info, &mut |_| {}).unwrap();
    assert_eq!(built.manifest.mod_specs().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["CET", "Archive Mod", "REDmod Thing"]);
    let titles: Vec<_> = built
        .manifest
        .mods
        .iter()
        .filter_map(|e| match e {
            lyno_core::manifest::ModEntry::Separator { title, .. } => Some(title.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(titles, ["Core", "Extras"]);
    assert_eq!(built.warnings, ["1 mod(s) under LYNO USER MODS are not shipped: My Test"]);
    std::fs::write(out.join("manifest.json"), serde_json::to_string(&built.manifest).unwrap()).unwrap();

    let lost = built.assets[0].file_name.clone();
    let mut host = FakeHost { releases: releases.clone(), published: None, uploads: vec![], lose: Some(lost.clone()) };
    let opts = PublishOptions { out: &out, download_root: &root };
    let no = AtomicBool::new(false);
    let run = |host: &mut FakeHost, answer: bool| publish(host, &opts, &Downloader::new(), &no, &mut |_| answer, &|_| {});

    let err = run(&mut host, true).unwrap_err().to_string();
    assert!(err.starts_with("1 part(s) are not downloadable") && err.contains(&lost), "{err}");
    assert_eq!(host.published, None, "nothing goes live while a part is missing");
    assert_eq!(host.uploads.len(), built.assets.len());

    host.lose = None;
    host.uploads.clear();
    assert!(matches!(run(&mut host, false), Err(lyno_core::Error::Cancelled)));
    assert_eq!(host.uploads, [lost], "only the missing asset is uploaded again");
    assert_eq!(host.published, None, "not confirmed");

    host.uploads.clear();
    let live = run(&mut host, true).unwrap();
    assert_eq!(live.build_version, "1.0.0");
    assert!(host.uploads.is_empty());
    assert_eq!(Manifest::from_json(host.published.as_ref().unwrap()).unwrap(), built.manifest);

    let err = run(&mut host, true).unwrap_err().to_string();
    assert_eq!(err, "build 1.0.0 is already published");
}

/// The author installs the build with the launcher like a player, keeps
/// playing and editing mods there, and releases from that instance. Their
/// changes show up as pending, not as damage; the release reuses every package
/// they didn't touch; afterwards the instance is the installed build again,
/// with nothing to download.
#[test]
fn author_releases_from_launcher_instance() {
    let tmp = tempfile::tempdir().unwrap();
    let author = tmp.path().join("author");
    let release = tmp.path().join("release");
    let out = tmp.path().join("out");
    author_instance(&author);
    let (base_url, _) = serve(release.clone());
    let mut nexus = |_: &ModMeta| ModInfo { author: Some("psiberx".into()), title: None };
    let out1 = build(&author, &options("1.0.0", &base_url, &out, None), &mut nexus, &mut |_| {}).unwrap();
    upload(&out1, &release);
    let m1 = out1.manifest;

    let inst = Instance::new(tmp.path().join("launcher"));
    install(&inst, &m1);
    let check = || verify(&inst, &State::load(&state_path(&inst)).unwrap(), &AtomicBool::new(false), &mut |_| {}).unwrap();
    assert_eq!(pending(&inst, &m1).unwrap(), Pending::default());
    assert_eq!(check().damaged, []);

    // Edit a mod, add one to the build, switch one off, try one privately.
    write(inst.root(), "mods/Archive Mod/archive/pc/mod/a.archive", &noise(200_000, 9));
    write(inst.root(), "mods/New Mod/archive/pc/mod/new.archive", b"new");
    write(inst.root(), "mods/My Test/archive/pc/mod/test.archive", b"experiment");
    std::fs::create_dir_all(inst.mods_dir().join("LYNO USER MODS_separator")).unwrap();
    let path = inst.modlist_path("LYNO");
    let mut list = ModList::load(&path).unwrap();
    list.entries.iter_mut().find(|e| e.name == "REDmod Thing").unwrap().state = EntryState::Disabled;
    list.entries.push(lyno_core::modlist::Entry::enabled("New Mod"));
    list.entries.push(lyno_core::modlist::Entry::separator("LYNO USER MODS"));
    list.entries.push(lyno_core::modlist::Entry::enabled("My Test"));
    list.save(&path).unwrap();

    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        pending(&inst, &m1).unwrap(),
        Pending { added: names(&["New Mod"]), toggled: names(&["REDmod Thing"]), personal: names(&["My Test"]), ..Default::default() }
    );
    let report = check();
    assert_eq!(report.damaged.iter().map(|d| d.id.as_deref()).collect::<Vec<_>>(), [Some("archive-mod")], "{report:?}");

    // No Nexus key in the launcher: authors stay as published.
    let mut no_info = |_: &ModMeta| ModInfo::default();
    let mut cancelled = options("1.1.0", &base_url, &out, Some(m1.clone()));
    cancelled.cancel = Some(std::sync::Arc::new(AtomicBool::new(true)));
    assert!(matches!(build(inst.root(), &cancelled, &mut no_info, &mut |_| {}), Err(lyno_core::Error::Cancelled)));
    let out2 = build(inst.root(), &options("1.1.0", &base_url, &out, Some(m1.clone())), &mut no_info, &mut |_| {}).unwrap();
    assert_eq!(out2.manifest.mod_specs().find(|m| m.id == "cet").unwrap().author.as_deref(), Some("psiberx"));
    let repacked: Vec<_> = out2.repacked.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(repacked, ["Archive Mod", "New Mod"], "base, CET and the switched mod are reused");
    let sizes: u64 = out2.repacked.iter().map(|r| out2.upload_size(&r.name)).sum();
    assert_eq!(sizes, out2.assets.iter().map(|a| a.size).sum::<u64>(), "every new part belongs to a repacked package");
    assert!(out2.upload_size("New Mod") > 0);
    upload(&out2, &release);
    let m2 = out2.manifest;
    assert!(!m2.mod_specs().any(|m| m.name == "My Test"));
    assert!(!m2.mod_specs().find(|m| m.name == "REDmod Thing").unwrap().enabled);

    let st = adopt(&inst, &m2, Some(&out)).unwrap();
    assert_eq!(st.build_version.as_deref(), Some("1.1.0"));
    let record = st.last_update.as_ref().unwrap();
    assert_eq!((record.added.as_slice(), record.updated.as_slice()), (&["new-mod".to_string()][..], &["archive-mod".to_string()][..]));
    let p = plan(&m2, &st, &ModList::load(&path).unwrap());
    assert!(p.is_up_to_date(), "{:?}", p.actions);
    assert_eq!(pending(&inst, &m2).unwrap(), Pending { personal: vec!["My Test".into()], ..Default::default() });
    assert_eq!(check().damaged, []);
    assert!(inst.root().join("mods/My Test/archive/pc/mod/test.archive").exists());

    // Rename (kept by `[LYNO] id=`), reorder and drop a mod.
    write(inst.root(), "mods/New Mod/meta.ini", b"[LYNO]\nid=new-mod\n");
    std::fs::rename(inst.mods_dir().join("New Mod"), inst.mods_dir().join("Newer Mod")).unwrap();
    let mut list = ModList::load(&path).unwrap();
    list.entries.iter_mut().find(|e| e.name == "New Mod").unwrap().name = "Newer Mod".into();
    list.entries.retain(|e| e.name != "REDmod Thing");
    let cet = list.entries.iter().position(|e| e.name == "CET").unwrap();
    list.entries.swap(cet, cet + 1);
    list.save(&path).unwrap();
    let p = pending(&inst, &m2).unwrap();
    assert_eq!(p.renamed, [("New Mod".to_string(), "Newer Mod".to_string())]);
    assert_eq!(p.removed, ["REDmod Thing"]);
    assert!(p.reordered && p.added.is_empty() && p.toggled.is_empty(), "{p:?}");

    // A build from another instance is not adopted.
    let other = Instance::new(tmp.path().join("other"));
    install(&other, &m1);
    assert!(adopt(&other, &m2, None).unwrap_err().to_string().contains("\"New Mod\""));
}

/// What a fake GitHub holds: the manifest on `main` and releases with assets.
#[derive(Default)]
struct FakeGitHub {
    /// (blob sha, content)
    manifest: Option<(String, Vec<u8>)>,
    /// (id, tag)
    releases: Vec<(u64, String)>,
    /// asset id → (release id, name, bytes)
    assets: std::collections::BTreeMap<u64, (u64, String, Vec<u8>)>,
    next_id: u64,
    /// "METHOD path" of every API call.
    log: Vec<String>,
}

/// GitHub's REST API as far as [`lyno_core::github::GitHub`] uses it, plus
/// release downloads under `/download/<tag>/<name>`. Uploads must carry a
/// Content-Length, as on uploads.github.com.
fn fake_github(token: &'static str) -> (String, std::sync::Arc<std::sync::Mutex<FakeGitHub>>) {
    use base64::Engine;
    use std::io::Read;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let state = std::sync::Arc::new(std::sync::Mutex::new(FakeGitHub { next_id: 1, ..Default::default() }));
    let shared = state.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            let mut headers = std::collections::HashMap::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':') {
                    headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
                }
            }
            let mut words = request.split_whitespace();
            let (method, target) = (words.next().unwrap().to_owned(), words.next().unwrap().to_owned());
            let (path, query) = target.split_once('?').unwrap_or((&target, ""));
            let mut body = Vec::new();
            if headers.contains_key("transfer-encoding") {
                write!(stream, "HTTP/1.1 411 Length Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                continue;
            }
            if let Some(len) = headers.get("content-length") {
                body.resize(len.parse().unwrap(), 0);
                reader.read_exact(&mut body).unwrap();
            }
            let json = |v: serde_json::Value| v.to_string().into_bytes();
            let mut st = shared.lock().unwrap();
            let (status, out): (u16, Vec<u8>) = if let Some(rest) = path.strip_prefix("/download/") {
                let (tag, name) = rest.split_once('/').unwrap();
                let rid = st.releases.iter().find(|r| r.1 == tag).map(|r| r.0);
                match st.assets.values().find(|a| Some(a.0) == rid && a.1 == name) {
                    Some(a) => (200, a.2.clone()),
                    None => (404, vec![]),
                }
            } else if headers.get("authorization").map(String::as_str) != Some(&format!("Bearer {token}")) {
                (401, json(serde_json::json!({ "message": "Bad credentials" })))
            } else {
                st.log.push(format!("{method} {path}"));
                let id_after = |prefix: &str| path.strip_prefix(prefix).and_then(|r| r.split('/').next()).and_then(|r| r.parse::<u64>().ok());
                match (method.as_str(), path) {
                    ("GET", "/repos/o/r") => (200, json(serde_json::json!({ "permissions": { "push": true } }))),
                    ("GET", "/repos/o/r/contents/build/manifest.json") => match &st.manifest {
                        None => (404, json(serde_json::json!({ "message": "Not Found" }))),
                        Some((_, content)) if headers.get("accept").is_some_and(|a| a.contains("raw")) => (200, content.clone()),
                        Some((sha, _)) => (200, json(serde_json::json!({ "sha": sha }))),
                    },
                    ("PUT", "/repos/o/r/contents/build/manifest.json") => {
                        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        match (&st.manifest, v["sha"].as_str()) {
                            (Some((sha, _)), Some(given)) if sha != given => (409, json(serde_json::json!({ "message": "is at x but expected y" }))),
                            (Some(_), None) => (422, json(serde_json::json!({ "message": "\"sha\" wasn't supplied." }))),
                            _ => {
                                let content = base64::engine::general_purpose::STANDARD.decode(v["content"].as_str().unwrap()).unwrap();
                                st.manifest = Some((blake3::hash(&content).to_hex()[..12].to_owned(), content));
                                (200, json(serde_json::json!({})))
                            }
                        }
                    }
                    ("GET", p) if p.starts_with("/repos/o/r/releases/tags/") => {
                        let tag = p.rsplit('/').next().unwrap();
                        match st.releases.iter().find(|r| r.1 == tag) {
                            Some((id, _)) => (200, json(serde_json::json!({ "id": id }))),
                            None => (404, json(serde_json::json!({ "message": "Not Found" }))),
                        }
                    }
                    ("GET", p) if p.ends_with("/assets") => {
                        let rid = id_after("/repos/o/r/releases/").unwrap();
                        let list: Vec<_> = st
                            .assets
                            .iter()
                            .filter(|(_, a)| a.0 == rid)
                            .map(|(id, a)| serde_json::json!({ "id": id, "name": a.1, "size": a.2.len(), "state": "uploaded" }))
                            .collect();
                        (200, json(serde_json::json!(list)))
                    }
                    ("POST", "/repos/o/r/releases") => {
                        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        assert_eq!(v["make_latest"], "false", "the launcher installer stays the latest release");
                        let id = st.next_id;
                        st.next_id += 1;
                        st.releases.push((id, v["tag_name"].as_str().unwrap().to_owned()));
                        (201, json(serde_json::json!({ "id": id })))
                    }
                    ("DELETE", p) if p.starts_with("/repos/o/r/releases/assets/") => {
                        let aid = id_after("/repos/o/r/releases/assets/").unwrap();
                        st.assets.remove(&aid);
                        (204, vec![])
                    }
                    ("POST", p) if p.ends_with("/assets") => {
                        let rid = id_after("/repos/o/r/releases/").unwrap();
                        let name = query.strip_prefix("name=").unwrap().to_owned();
                        let id = st.next_id;
                        st.next_id += 1;
                        let size = body.len();
                        st.assets.insert(id, (rid, name, body));
                        (201, json(serde_json::json!({ "id": id, "size": size, "state": "uploaded" })))
                    }
                    _ => (404, json(serde_json::json!({ "message": format!("no route {method} {path}") }))),
                }
            };
            drop(st);
            let reason = if status < 300 { "OK" } else { "Error" };
            write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", out.len()).unwrap();
            if method != "HEAD" {
                stream.write_all(&out).unwrap();
            }
        }
    });
    (format!("http://{addr}"), state)
}

/// The launcher publishes through the GitHub API: creates the release,
/// uploads every part with progress, replaces a part an earlier run left
/// half-uploaded, and commits the manifest last. A manifest changed on GitHub
/// meanwhile is not overwritten.
#[test]
fn publishes_through_github_api() {
    use lyno_core::github::GitHub;
    use lyno_core::release::Event;
    let tmp = tempfile::tempdir().unwrap();
    let author = tmp.path().join("author");
    let out = tmp.path().join("out");
    author_instance(&author);
    let (root, gh) = fake_github("tok");
    let download = format!("{root}/download");
    let mut no_info = |_: &ModMeta| ModInfo::default();
    let built = build(&author, &options("1.0.0", &format!("{download}/build-1.0.0"), &out, None), &mut no_info, &mut |_| {}).unwrap();
    std::fs::write(out.join("manifest.json"), serde_json::to_string(&built.manifest).unwrap()).unwrap();

    let err = GitHub::with_roots("o/r", "wrong", &root, &root).check_access().unwrap_err().to_string();
    assert!(err.contains("401") && err.contains("wrong or expired"), "{err}");
    let mut host = GitHub::with_roots("o/r", "tok", &root, &root);
    host.check_access().unwrap();

    let opts = PublishOptions { out: &out, download_root: &download };
    let no = AtomicBool::new(false);
    let events = std::sync::Mutex::new(Vec::new());
    let live = publish(&mut host, &opts, &Downloader::new(), &no, &mut |_| true, &|e| events.lock().unwrap().push(e)).unwrap();
    assert_eq!(live, built.manifest);
    let total: u64 = built.assets.iter().map(|a| a.size).sum();
    assert!(events.lock().unwrap().contains(&Event::UploadBytes { done: total, total }), "upload progress reaches the end");
    {
        let st = gh.lock().unwrap();
        assert_eq!(st.assets.len(), built.assets.len());
        assert_eq!(Manifest::from_json(std::str::from_utf8(&st.manifest.as_ref().unwrap().1).unwrap()).unwrap(), built.manifest);
    }

    // 1.1.0: an earlier run created the release and left one part half-uploaded.
    write(&author, "mods/Archive Mod/archive/pc/mod/a.archive", &noise(200_000, 5));
    let m1 = built.manifest;
    let built = build(&author, &options("1.1.0", &format!("{download}/build-1.1.0"), &out, Some(m1)), &mut no_info, &mut |_| {}).unwrap();
    std::fs::write(out.join("manifest.json"), serde_json::to_string(&built.manifest).unwrap()).unwrap();
    let half = &built.assets[0];
    {
        let mut st = gh.lock().unwrap();
        let (rid, aid) = (st.next_id, st.next_id + 1);
        st.next_id += 2;
        st.releases.push((rid, "build-1.1.0".into()));
        let data = std::fs::read(&half.path).unwrap();
        st.assets.insert(aid, (rid, half.file_name.clone(), data[..data.len() / 2].to_vec()));
        st.log.clear();
    }

    // Someone publishes from elsewhere while this publish is waiting for confirmation.
    let mut host = GitHub::with_roots("o/r", "tok", &root, &root);
    let mut meanwhile = |_: &Manifest| {
        gh.lock().unwrap().manifest.as_mut().unwrap().0 = "other".into();
        true
    };
    let err = publish(&mut host, &opts, &Downloader::new(), &no, &mut meanwhile, &|_| {}).unwrap_err().to_string();
    assert!(err.contains("changed on GitHub"), "{err}");
    let log = gh.lock().unwrap().log.clone();
    assert!(log.iter().any(|l| l.starts_with("DELETE /repos/o/r/releases/assets/")), "half-uploaded part replaced: {log:?}");
    assert!(!log.iter().any(|l| l == "POST /repos/o/r/releases"), "existing release reused: {log:?}");

    gh.lock().unwrap().log.clear();
    let mut host = GitHub::with_roots("o/r", "tok", &root, &root);
    publish(&mut host, &opts, &Downloader::new(), &no, &mut |_| true, &|_| {}).unwrap();
    let st = gh.lock().unwrap();
    assert!(!st.log.iter().any(|l| l.starts_with("POST /repos/o/r/releases/")), "nothing uploaded again: {:?}", st.log);
    assert_eq!(Manifest::from_json(std::str::from_utf8(&st.manifest.as_ref().unwrap().1).unwrap()).unwrap().build_version, "1.1.0");
}

/// A fake Nexus: the API answers and the CDN file are static files of `dir`,
/// named by request path (query string included).
fn fake_nexus(dir: &Path, base: &str, now: u64, zip: &[u8]) {
    let api = "v1/games/cyberpunk2077/mods";
    let file = |id: u64, cat: &str, version: &str, size: usize, at: u64| {
        format!(
            r#"{{"file_id":{id},"name":"Main File","version":"{version}","category_name":"{cat}","file_name":"mod-{id}.zip","size_in_bytes":{size},"uploaded_timestamp":{at},"description":"ignored"}}"#
        )
    };
    let files_42 = format!(
        r#"{{"files":[{},{}],"file_updates":[{{"old_file_id":10,"new_file_id":11,"old_file_name":"a","new_file_name":"b"}}]}}"#,
        file(10, "OLD_VERSION", "1.0", 1, 1),
        file(11, "MAIN", "1.1", zip.len(), 2)
    );
    write(dir, &format!("{api}/42/files.json"), files_42.as_bytes());
    write(dir, &format!("{api}/42.json"), br#"{"name":"Old Mod","version":"1.1","author":"someone","available":true}"#);
    let files_107 = format!(
        r#"{{"files":[{},{}],"file_updates":[{{"old_file_id":1,"new_file_id":2}}]}}"#,
        file(1, "OLD_VERSION", "1.35", 1, 1),
        file(2, "MAIN", "1.36", zip.len(), 2)
    );
    write(dir, &format!("{api}/107/files.json"), files_107.as_bytes());
    write(dir, &format!("{api}/107.json"), br#"{"name":"Cyber Engine Tweaks","version":"1.36"}"#);
    // A free account's link carries a key; without one the API refuses (no file: 404).
    let link = format!(r#"[{{"name":"Nexus CDN","short_name":"Nexus CDN","URI":"{base}/cdn/mod.zip"}}]"#);
    write(dir, &format!("{api}/42/files/11/download_link.json?key=abc&expires=99"), link.as_bytes());
    write(dir, &format!("{api}/107/files/2/download_link.json?key=abc&expires=99"), link.as_bytes());
    // 107 changed after its check below; 42 was recorded by the download.
    let updated = format!(r#"[{{"mod_id":107,"latest_file_update":{now},"latest_mod_activity":{now}}}]"#);
    write(dir, &format!("{api}/updated.json?period=1d"), updated.as_bytes());
    write(dir, "cdn/mod.zip", zip);
}

fn mod_zip() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(&mut buf);
    w.start_file("Old Mod 1.1/archive/pc/mod/new.archive", zip::write::SimpleFileOptions::default()).unwrap();
    w.write_all(b"new version").unwrap();
    w.start_file("Old Mod 1.1/readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
    w.write_all(b"dropped").unwrap();
    w.finish().unwrap();
    buf.into_inner()
}

#[test]
fn nexus_updates_are_tracked_and_installed_from_nxm_links() {
    use lyno_core::mod_install::{fetch_and_install, Context, Outcome};
    use lyno_core::nexus::NexusApi;
    use lyno_core::nxm::NxmLink;
    use lyno_core::tracking::{check, status, tracked_mods, Cache, Status};

    let tmp = tempfile::tempdir().unwrap();
    let user = Instance::new(tmp.path().join("user"));
    let root = user.root();
    write(root, "profiles/LYNO/modlist.txt", b"+Old Mod\r\n-LYNO USER MODS_separator\r\n+CET\r\n");
    write(root, "mods/CET/meta.ini", b"[General]\nmodid=107\nversion=1.35\ngameName=cyberpunk2077\n[installedFiles]\n1\\modid=107\n1\\fileid=1\n[LYNO]\nid=cet\n");
    write(root, "mods/CET/bin/x64/plugins/cyber_engine_tweaks.asi", b"cet");
    write(root, "mods/Old Mod/meta.ini", b"[General]\nmodid=42\nversion=1.0\ngameName=cyberpunk2077\n[installedFiles]\n1\\modid=42\n1\\fileid=10\n");
    write(root, "mods/Old Mod/archive/pc/mod/old.archive", b"old version");
    write(root, ".lyno/state.json", br#"{"buildVersion":"1.0.0","baseHash":"x","mods":{"cet":{"folder":"CET","hash":"h"}}}"#);

    let nexus_dir = tmp.path().join("nexus");
    std::fs::create_dir_all(&nexus_dir).unwrap();
    let (base, log) = serve(nexus_dir.clone());
    let now = 1_800_000_000;
    fake_nexus(&nexus_dir, &base, now, &mod_zip());
    let api = NexusApi::with_base(&base, "key");
    let downloader = Downloader::new();
    let never = || false;
    let player = Context { inst: &user, profile: "LYNO", author: false, mo2_running: false, now };

    // The player's own mod: the new file replaces the folder in place.
    let link = NxmLink::parse("nxm://cyberpunk2077/mods/42/files/11?key=abc&expires=99&user_id=1").unwrap();
    let mut progress = Vec::new();
    let (dl, outcome) = fetch_and_install(&api, &downloader, &player, &link, &never, &mut |p| progress.push(p)).unwrap().unwrap();
    assert_eq!(outcome, Outcome::Installed { folder: "Old Mod".into() });
    assert_eq!(dl.version.as_deref(), Some("1.1"));
    let folder = root.join("mods/Old Mod");
    assert!(!folder.join("archive/pc/mod/old.archive").exists());
    assert_eq!(std::fs::read(folder.join("archive/pc/mod/new.archive")).unwrap(), b"new version");
    assert!(!folder.join("readme.txt").exists());
    let meta = ModMeta::load(&folder.join("meta.ini")).unwrap();
    assert_eq!((meta.file_id, meta.version.as_deref()), (Some(11), Some("1.1")));
    assert!(root.join("downloads/mod-11.zip").is_file() && root.join("downloads/mod-11.zip.meta").is_file());
    let list = ModList::load(&user.modlist_path("LYNO")).unwrap();
    assert_eq!(list.entries.len(), 3, "updated in place, no new entry");

    // The build's mod is updated with the build, not from Nexus.
    let cet = NxmLink::parse("nxm://cyberpunk2077/mods/107/files/2?key=abc&expires=99").unwrap();
    let refused = fetch_and_install(&api, &downloader, &player, &cet, &never, &mut |_| {}).unwrap();
    assert_eq!(refused.unwrap_err().0, "CET");
    assert!(!log.lock().unwrap().iter().any(|r| r.contains("107/files/2/download_link")), "nothing downloaded");

    // Tracking: 42 was just recorded; 107 is asked for, and `updated.json` is
    // only needed for mods checked before.
    let (tracked, untracked) = tracked_mods(&user, "LYNO").unwrap();
    assert_eq!(tracked.iter().map(|t| (t.folder.as_str(), t.personal, t.managed)).collect::<Vec<_>>(), [("CET", false, true), ("Old Mod", true, false)]);
    assert!(untracked.is_empty());
    let mut cache = Cache::load(&Cache::path(&user));
    log.lock().unwrap().clear();
    check(&api, &mut cache, &tracked, now + 60, false, &never, &mut |_, _| {}).unwrap();
    let asked = log.lock().unwrap().clone();
    assert!(asked.iter().any(|r| r.ends_with("107/files.json")), "{asked:?}");
    assert!(!asked.iter().any(|r| r.ends_with("42/files.json")), "{asked:?}");
    let by_folder = |f: &str| status(tracked.iter().find(|t| t.folder == f).unwrap(), cache.get("cyberpunk2077", tracked.iter().find(|t| t.folder == f).unwrap().mod_id));
    assert!(matches!(by_folder("CET"), Status::Update { file: Some(ref f), .. } if f.file_id == 2));
    assert_eq!(by_folder("Old Mod"), Status::UpToDate);

    // A second check within the day asks only `updated.json`, which lists 107
    // as changed at `now`, before its check: nothing to fetch again.
    log.lock().unwrap().clear();
    check(&api, &mut cache, &tracked, now + 120, false, &never, &mut |_, _| {}).unwrap();
    assert_eq!(log.lock().unwrap().clone(), ["v1/games/cyberpunk2077/mods/updated.json?period=1d"]);

    // The author updates build mods; the [LYNO] id stays, so it is the same build mod.
    let author = Context { author: true, ..player };
    let (_, outcome) = fetch_and_install(&api, &downloader, &author, &cet, &never, &mut |_| {}).unwrap().unwrap();
    assert_eq!(outcome, Outcome::Installed { folder: "CET".into() });
    let meta = ModMeta::load(&root.join("mods/CET/meta.ini")).unwrap();
    assert_eq!((meta.file_id, meta.lyno_id.as_deref()), (Some(2), Some("cet")));

    // With MO2 open the archive only goes to its downloads.
    let open = Context { mo2_running: true, ..player };
    let (_, outcome) = fetch_and_install(&api, &downloader, &open, &link, &never, &mut |_| {}).unwrap().unwrap();
    assert_eq!(outcome, Outcome::Mo2Open);
}
