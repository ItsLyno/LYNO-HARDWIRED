//! Author builds a release, a user installs it over HTTP, the author
//! changes one mod, the user updates and downloads only that mod.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

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

    fn upload_asset(&mut self, tag: &str, path: &Path) -> lyno_core::Result<()> {
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
