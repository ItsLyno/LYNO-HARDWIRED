# LYNO//HARDWIRED

Launcher for a Cyberpunk 2077 mod build based on Mod Organizer 2. The build (portable
MO2 + instance config + every mod) is distributed through GitHub Releases of this
repo; `build/manifest.json` on `main` points at the current version. The launcher
installs and updates the build, sets the game path, starts the game through MO2.

## Layout

| Path | What |
|---|---|
| `crates/core` (`lyno-core`) | Everything that matters: manifest, packages, download, install, update plan, MO2 files, game detection |
| `crates/pack` (`lyno-pack`) | Author CLI: `build` (MO2 instance → `out/manifest.json` + `tar.zst` parts), `publish` (upload, HTTP-check every part, push the manifest) |
| `apps/launcher` | Tauri 2 + React + TypeScript + Tailwind. `src-tauri/src/commands.rs` is the IPC layer over `lyno-core` |
| `docs/` | Reference: [Cyberpunk under MO2](docs/cyberpunk-mo2.md), [MO2 instance format](docs/mo2-instance.md), [release process](docs/release-process.md), vendored MO2 Cyberpunk plugin in `docs/reference/` |

Core modules: `publish.rs` (author side: build manifest, reuse unchanged packages),
`release.rs` (publish a build in the safe order over a `Host`: releases + manifest; `lyno-pack` implements it with `gh`/`git`),
`author.rs` (author mode: the author releases from their launcher instance; `pending` mod-list changes since the installed build, `adopt` a just-published build as installed),
`plan.rs` (manifest + state → actions + new `modlist.txt`), `install.rs` (apply plan),
`prefetch.rs` (parallel part downloads running ahead of `install`), `download.rs` (one part: resume, retries),
`verify.rs` (integrity check; repair = flags in `state.json` that `plan` turns into `Repair` actions),
`files.rs` (per-file hashes recorded at unpack, `.lyno/files/`; launcher-local, not in the manifest), `report.rs` (diagnostic zip),
`package.rs` (tar.zst split into ≤1.9 GB parts), `tree.rs` (tree hash), `rules.rs`
(which files ship and which are hashed), `mo2.rs`, `modlist.rs`, `meta.rs`, `state.rs`.

## Commands

```sh
cargo test -p lyno-core -p lyno-pack          # unit tests + tests/e2e.rs (build → install → update over a local HTTP server)
cargo clippy -p lyno-core -p lyno-pack --all-targets
cd apps/launcher && pnpm install && pnpm build # frontend
pnpm dev                                       # UI in a browser on mock data (src/mock.ts)
```

The launcher crate (`lyno-hardwired`) is clippy-checked on Linux in CI. Locally on Linux,
`cargo clippy -p lyno-hardwired` needs `libgtk-3-dev libwebkit2gtk-4.1-dev
libsoup-3.0-dev` and an existing `apps/launcher/dist/` folder.

CI (`.github/workflows/ci.yml`): core tests, frontend build and launcher clippy on
Ubuntu; `pnpm tauri build` on Windows (artifact `lyno-hardwired-setup`) only on a
manual run (Actions → CI → Run workflow).
`launcher-release.yml` (manual, from main) builds a signed installer, releases
`launcher-v<version>` and commits `launcher/latest.json`, which the launcher's
updater reads. Launcher logs go through `log::` macros to `tauri-plugin-log`.

## Invariants — don't break these

- **Package identity = tree hash** of shipped files minus `meta.ini`
  (`rules::is_hashed`). `publish` and `package::unpack` must use the same filter.
  Changing what is hashed or how is a manifest format change.
- **Manifest format changes bump `SCHEMA_VERSION`** (`manifest.rs`). The launcher
  accepts only its own schema, so a new launcher must be released before a build in
  the new format. New optional fields: `#[serde(default)]`, no bump needed.
- **Never ship runtime files**: `r6/cache/`, `r6/logs/`, `red4ext/logs/`, `*.log`,
  `archive/pc/mod/modlist.txt` (`rules::is_generated`); never ship profile
  `saves/`, `UserSettings.json`, `modlist.txt` (`rules::is_private_profile_file`).
  New generated files from frameworks go into `rules.rs` with a test; so do new
  settings file types (`rules::is_settings`: not damage in verify, kept on repair). Background
  in `docs/cyberpunk-mo2.md`.
- **The manifest goes live last**: `release::publish` pushes `build/manifest.json`
  only after every part answers over HTTP (front ends only implement `release::Host`); same for `launcher/latest.json` in
  `launcher-release.yml`.
- **Reused packages keep URLs into older releases.** Old `build-*` releases must
  never be deleted; nothing in code may assume all assets are in the latest release.
- **Player mods are untouchable**: anything not in `state.json` stays under the
  `LYNO USER MODS` separator (`plan::USER_SEPARATOR`). `build` never ships what is
  under it: the author may build from a launcher instance with personal mods.
- **The author's disk is the bottleneck** of `lyno-pack` (hundreds of mods,
  hundreds of GB; zstd skips the already Oodle-compressed `.archive` data).
  `publish::Packer` reads each file at most once (hash while packing), trusts
  `hash_cache` (size + mtime) for unchanged files, and resumes from
  `<package>.parts.json` records in `out/`. Keep it that way; never use the cache
  to verify downloads.
- **Updates are crash-safe**: unpack to `.lyno/staging`, verify hash, swap folders
  (`package::swap_folder`), save `state.json` after every action.
- `modlist.txt` is stored highest-priority-first; `ModList` holds UI order
  (lowest first). Easy to get backwards — see `docs/mo2-instance.md`.
- MO2 executable titles come from the author's `ModOrganizer.ini`; the launcher
  uses `Cyberpunk 2077` / `Cyberpunk 2077 (REDmod)` (`mo2::game_executable`).

## Conventions

- Code, comments, commit messages: English. User-facing strings (launcher UI,
  error messages returned to the UI) and README / `docs/release-process.md`: Russian.
  Reference docs for development (`docs/*.md` other than release-process): English.
- The repo is **not** rustfmt-formatted (lines up to ~120 chars). Don't run
  `cargo fmt` over existing files; match the surrounding style.
- Doc comments explain *why* (MO2 / Cyberpunk behavior), not what the code does.
- Behavior changes to build/install/update get covered in `crates/core/tests/e2e.rs`.
- The README «Статус» checklist tracks open features (free-space check, game version
  check).
