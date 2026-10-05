// Browser-only stand-in for the Tauri backend (`pnpm dev` without Tauri),
// so screens can be built and screenshotted without Windows or MO2.
// The data is illustrative, not the real build.
import type {
  ArchiveTarget,
  AuthorEvent,
  BuildInfo,
  BuiltRelease,
  Folder,
  GameInstall,
  InstanceEntry,
  LauncherUpdate,
  ModRow,
  ModUpdateRow,
  NexusHandlers,
  NexusJob,
  NexusStatus,
  NxmHandler,
  UpdatesView,
  Pending,
  Secret,
  Settings,
  Status,
  UpdateEvent,
  UpdateFinished,
  VerifyReport,
  FomodOutline,
  FomodSelection,
  FomodState,
  FomodWizard,
  ArchiveRoot,
  DownloadItem,
  RateLimit,
  UserRow,
  OverwriteInfo,
  Executable,
  FileDropHandlers,
} from "./api";

const GB = 1024 ** 3;
const MB = 1024 ** 2;

function mod(name: string, author: string, version: string, size: number, nexusId: number, extra: Partial<ModRow> = {}): ModRow {
  return {
    kind: "mod",
    id: name.toLowerCase().replace(/[^a-z0-9]+/g, "-"),
    name,
    title: null,
    version,
    author,
    nexusUrl: `https://www.nexusmods.com/cyberpunk2077/mods/${nexusId}`,
    enabled: true,
    optional: true,
    size,
    outdated: false,
    installed: true,
    recent: null,
    damaged: false,
    ...extra,
  } as ModRow;
}

const mods: ModRow[] = [
  { kind: "separator", title: "Фреймворки", color: "#00b4d8" },
  mod("Cyber Engine Tweaks", "yamashi", "1.35.0", 24 * MB, 107, { optional: false, outdated: true }),
  mod("RED4ext", "WopsS", "1.27.0", 3 * MB, 2380, { optional: false, outdated: true }),
  mod("redscript", "jekky", "0.5.27", 6 * MB, 1511, { optional: false }),
  mod("ArchiveXL", "psiberx", "1.21.1", 2 * MB, 4198, { optional: false }),
  mod("TweakXL", "psiberx", "1.10.6", 2 * MB, 4197, { optional: false }),
  mod("Codeware", "psiberx", "1.15.0", 3 * MB, 7780, { optional: false }),
  mod("Mod Settings", "jackhumbert", "0.2.11", 1 * MB, 4885, { optional: false }),
  { kind: "separator", title: "Графика", color: "#c77dff" },
  mod("Nova LUT", "Kvan7", "2.3", 180 * MB, 2075, { recent: "updated" }),
  mod("Ultra Plus", "sammilucia", "5.1.2", 12 * MB, 10490),
  mod("HD Reworked Project", "HalkHogan", "1.0", 9.4 * GB, 7652, { title: "HD Reworked Project — Ultra Quality", enabled: false }),
  { kind: "separator", title: "Геймплей", color: null },
  mod("Better Vehicle Handling", "Erok", "2.4", 1 * MB, 3401, { installed: false }),
  mod("Immersive Rippers", "xBaebsae", "1.2", 4 * MB, 9330, { recent: "added" }),
  mod("Never Lose Your Car", "Jelle Bakker", "1.0.3", 1 * MB, 6012, { enabled: false }),
  mod("Appearance Menu Mod", "MxOrcinus", "2.6.3", 420 * MB, 790),
  ...bulk(),
];

// A list as long as the real build's (hundreds of mods, a dozen-plus separators) to see the page at scale.
function bulk(): ModRow[] {
  const groups: [string, string | null, string[]][] = [
    ["00 - Core - Runtimes & Frameworks", "#ff3b5c", ["Input Loader", "Cyberpunk 2077 Input Loader", "RED4ext Patch", "Native Settings UI", "Redscript Plus", "Cyberpunk Engine Hooks", "Equipment-EX", "Virtual Atelier", "Mod Settings Extras", "TweakDB Extensions", "Archive Hot Reload", "Script Debugger", "Arasaka UI Framework", "ReShade Bridge"]],
    ["01 - Core - Libraries & Dependencies", "#ff3b5c", ["RadioExt", "Audioware", "World Builder", "Entity Spawner", "Let There Be Flight", "Cyberware-EX", "Spawn0", "Appearance Change Unlocker", "Body Mod Base", "Hair Physics Library", "Codeware Extras", "Vehicle Framework", "Weapon Framework", "Quest Framework", "Dialogue Framework", "Character Creator Library", "UI Kit", "Sound Library", "Icon Pack Base", "Localization Helper", "Animation Framework", "Photo Mode Library", "Clothing Base", "Gun Sounds Core", "Lighting Base", "Crowd Library", "Scene Tools", "Database Patcher", "Holo Library", "Map Library"]],
    ["02 - Fixes - Engine, Performance & Streaming", "#3cf2a0", ["Streaming Fix", "Shader Cache Fix", "Memory Pool Tweaks", "Texture Streaming Budget", "Stutter Fix", "LOD Fix", "Crowd Density Fix", "Async Compute Fix", "Ray Tracing Fixes", "DLSS Enabler", "Frame Gen Fix", "Shadow Cascade Fix", "VRAM Fix", "Loading Screen Fix", "Pop-in Reducer"]],
    ["03 - Fixes - Gameplay, Quest & System", "#3cf2a0", ["Map Waypoint Bug Fixes", "Radioport Dropout Fix", "Conflict Begone", "Vehicle Exit Fix", "Hammers are not bats", "Second Heart Fix", "Speedometer Fixes", "No crowd panic from stealth activity", "Photomode Camera Reset Fix", "Fixed NPC Vehicle Reactions", "Quest Trigger Fix", "Elevator Fix", "Wardrobe Fix", "Stash Fix", "Phone Call Fix", "Police Spawn Fix"]],
    ["04 - Visuals - Lighting & Weather", "#c77dff", ["Natural Weather", "Night City Lights", "Better Sky", "Volumetric Fog Tweaks", "Neon Reflections", "Sun Shadows", "Rain Overhaul", "Interior Lighting", "Car Headlights Overhaul", "Street Lamps HD"]],
    ["05 - Visuals - Textures & Models", "#c77dff", ["4K Roads", "HD Signs", "Better Trees", "Detailed Interiors", "Vehicle Textures HD", "NPC Faces Overhaul", "Weapon Textures HD", "Graffiti Pack", "Billboards HD", "Water Textures", "Debris Overhaul", "Food Props HD"]],
    ["06 - Characters - Body & Faces", "#f3e600", ["Hyst Body", "VTK Body", "Skin Overhaul", "Eye Textures HD", "Better Teeth", "Tattoo Pack", "Hair Pack", "Makeup Collection", "Nails Pack", "Freckles"]],
    ["07 - Characters - Clothing", "#f3e600", ["Arasaka Jacket", "Netrunner Suit", "Nomad Pack", "Corpo Suits", "Street Kid Hoodies", "Boots Collection", "Sunglasses Pack", "Glove Pack", "Rings and Jewelry", "Cyberdeck Visuals", "Masks Pack", "Hats Collection", "Backpacks"]],
    ["08 - Gameplay - Combat", null, ["Better Netrunning", "Enemies Rebalance", "Stealth Overhaul", "Smarter AI", "Melee Overhaul", "Grenade Tweaks", "Recoil Overhaul", "Headshot Damage", "Cyberware Rebalance", "Quickhack Rebalance", "Boss Overhaul"]],
    ["09 - Gameplay - Driving & Vehicles", null, ["Better Traffic", "Car Radio Overhaul", "Vehicle Combat Tweaks", "Motorcycle Lean", "Drive an Aerial Vehicle", "Cars Sound Overhaul", "Fuel System", "Vehicle Summon Anywhere", "Garage Expansion"]],
    ["10 - Gameplay - Economy & Loot", null, ["Loot Overhaul", "Vendor Rebalance", "Crafting Overhaul", "Iconic Weapons Rework", "Better Stash", "Money Rebalance", "Apartment Shopping"]],
    ["11 - World - NPCs & Crowds", "#00b4d8", ["Crowd Overhaul", "NPC Routines", "Gang Patrols", "Police Overhaul", "Street Vendors", "Joytoys Overhaul", "Bar Patrons", "Night Life", "Pedestrian Reactions"]],
    ["12 - World - Locations", "#00b4d8", ["Afterlife Expansion", "Apartment Interiors", "Open Shops", "Megabuilding Interiors", "Badlands Camps", "Pacifica Expansion", "Rooftops Access", "Hidden Clubs"]],
    ["13 - UI & HUD", "#ffb020", ["Minimap Tweaks", "Better HUD", "Clean Inventory", "Item Tooltips", "Better Map", "Quest Log Overhaul", "Font Overhaul", "Crosshair Pack", "Damage Numbers", "Compass HUD", "Photo Mode UI"]],
    ["14 - Audio & Radio", "#ffb020", ["Radio Stations Expansion", "Ambient Sounds", "Gun Sounds Overhaul", "Footsteps Overhaul", "Music Replacer", "Silent Menus"]],
    ["99 - Patches & Compatibility", "#8b91a1", ["Compatibility Patch Pack", "Body x Clothing Refits", "Lighting x Weather Patch", "HUD x Map Patch", "AI x Stealth Patch", "Final Load Order Patch"]],
  ];
  let n = 20000;
  return groups.flatMap(([title, color, names]) => [
    { kind: "separator", title, color } as ModRow,
    ...names.map((name, i) =>
      mod(name, "author", `1.${i % 7}.${i % 3}`, ((i % 9) + 1) * 12 * MB, n++, {
        enabled: n % 17 !== 0,
        optional: !/^0[01] /.test(title),
      }),
    ),
  ]);
}

const BUILD_DIR = "C:\\Users\\V\\AppData\\Local\\dev.lyno.hardwired\\instance";

let settings: Settings = {
  instanceDir: BUILD_DIR,
  instances: [{ name: "LYNO//HARDWIRED", dir: BUILD_DIR, build: true }],
  gameDir: "D:\\SteamLibrary\\steamapps\\common\\Cyberpunk 2077",
  manifestUrl: "https://raw.githubusercontent.com/ItsLyno/LYNO-HARDWIRED/main/build/manifest.json",
  authorMode: false,
  authorRepo: "ItsLyno/LYNO-HARDWIRED",
  authorOutDir: null,
};

const AUTHOR_MODE_NO_UPDATE =
  "В режиме автора обновление и починка выключены: они откатили бы ваши правки модов. Выключите режим автора в настройках, если нужно поставить опубликованную версию.";

const pending: Pending = {
  added: ["Kiroshi Night Vision", "Glitch Effects Tweaks"],
  removed: ["Never Lose Your Car"],
  renamed: [["Nova LUT", "Nova LUT 2"]],
  toggled: ["HD Reworked Project"],
  reordered: true,
  personal: ["My Test Weapon"],
};

let status: Status = {
  mo2Installed: true,
  gameDir: settings.gameDir,
  gameFound: true,
  installedVersion: "1.3.2",
  modsTotal: 14,
  modsEnabled: 13,
  gameRunning: false,
  mo2Running: false,
  updating: false,
};

let build: BuildInfo = {
  name: "LYNO//HARDWIRED",
  latestVersion: "1.4.0",
  installedVersion: "1.3.2",
  gameVersion: "2.31",
  changelog: [
    {
      version: "1.4.0",
      date: "2026-10-02",
      notes: [
        "Обновлены Cyber Engine Tweaks и RED4ext под патч 2.31",
        "Добавлен Better Vehicle Handling",
        "Удалён Immersive Traffic — конфликтовал с новым трафиком",
      ],
    },
    { version: "1.3.2", date: "2026-09-18", notes: ["Исправлен порядок загрузки архивов освещения", "Обновлён Nova LUT"] },
    { version: "1.3.0", date: "2026-09-01", notes: ["Новый пресет ReShade", "Добавлены 6 модов на одежду", "Обновлён ArchiveXL"] },
  ],
  mods,
  upToDate: false,
  changes: 4,
  repairs: 0,
  downloadSize: 28 * MB,
  downloaded: 9 * MB,
  online: true,
  lastUpdate: { from: "1.3.0", to: "1.3.2", removed: ["Immersive Traffic"] },
};

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
let progress: ((e: UpdateEvent) => void) | null = null;
let finished: ((f: UpdateFinished) => void) | null = null;
let cancelled = false;
let verifyProgress: ((e: UpdateEvent) => void) | null = null;
let verifyCancelled = false;

const secrets = { github: false, nexus: false };
let built: BuiltRelease | null = null;
let author: ((e: AuthorEvent) => void) | null = null;
let authorCancelled = false;

async function simulateBuild(version: string, gameVersion: string, notes: string[]) {
  authorCancelled = false;
  const names = mods.flatMap((m) => (m.kind === "mod" ? [m.name] : []));
  const total = names.length + 1;
  for (const [i, name] of ["base (MO2, профиль)", ...names].entries()) {
    if (authorCancelled) return author?.({ kind: "finished", job: "build", ok: false, error: null });
    const changed = i === 3 || i === 9;
    author?.({ kind: "step", index: i + 1, total, label: `${name}: ${changed ? "packing 180 MB" : "hashing 24 MB"}` });
    await delay(changed ? 700 : 150);
  }
  built = {
    version,
    gameVersion,
    notes,
    repacked: [
      { name: "ArchiveXL", changed: true, size: 2 * MB },
      { name: "Nova LUT", changed: true, size: 180 * MB },
      { name: "Kiroshi Night Vision", changed: false, size: 14 * MB },
    ],
    uploadSize: 196 * MB,
    mods: names.length,
    warnings: ['"Glitch Effects Tweaks": no Nexus mod id in meta.ini, no link in the launcher'],
    restored: false,
  };
  author?.({ kind: "finished", job: "build", ok: true, error: null });
}

async function simulatePublish() {
  authorCancelled = false;
  if (!built) return;
  const version = built.version;
  author?.({ kind: "log", line: `Сейчас опубликована версия ${build.latestVersion}` });
  author?.({ kind: "log", line: `Создан релиз build-${version}` });
  const parts = [2 * MB, 180 * MB, 14 * MB];
  const total = parts.reduce((a, b) => a + b, 0);
  let done = 0;
  for (const [i, size] of parts.entries()) {
    author?.({ kind: "step", index: i + 1, total: parts.length, label: `Загрузка part-${i + 1}.tar.zst.001` });
    for (let t = 1; t <= 8; t++) {
      if (authorCancelled) return author?.({ kind: "finished", job: "publish", ok: false, error: null });
      await delay(120);
      author?.({ kind: "bytes", done: done + (size * t) / 8, total });
    }
    done += size;
  }
  for (let i = 1; i <= 40; i += 3) {
    author?.({ kind: "step", index: i, total: 40, label: "Проверка частей по HTTP" });
    await delay(40);
  }
  author?.({ kind: "log", line: `Версия ${version} опубликована` });
  status = { ...status, installedVersion: version };
  build = { ...build, latestVersion: version, installedVersion: version, upToDate: true, changes: 0, downloadSize: 0 };
  built = null;
  author?.({ kind: "finished", job: "publish", ok: true, error: null });
}

async function simulateVerify(): Promise<VerifyReport | null> {
  verifyCancelled = false;
  const installed = build.mods.filter((m) => m.kind === "mod" && m.installed);
  const total = installed.reduce((n, m) => n + (m.kind === "mod" ? m.size : 0), 0);
  let done = 0;
  for (const [i, m] of installed.entries()) {
    if (m.kind !== "mod") continue;
    verifyProgress?.({ kind: "step", index: i + 1, total: installed.length, label: m.name });
    for (let t = 0; t < 4; t++) {
      if (verifyCancelled) return null;
      await delay(80);
      done += m.size / 4;
      verifyProgress?.({ kind: "bytes", done, total });
    }
  }
  const files = (...sample: string[]) => ({ count: sample.length, sample });
  return {
    checked: installed.length,
    damaged: [
      {
        id: null,
        folder: "",
        problem: { kind: "files", missing: files("ModOrganizer.exe", "dlls/Qt6Core.dll"), changed: files(), added: files() },
      },
      {
        id: "cyber-engine-tweaks",
        folder: "Cyber Engine Tweaks",
        problem: { kind: "files", missing: files("bin/x64/plugins/cyber_engine_tweaks.asi"), changed: files(), added: files() },
      },
      { id: "nova-lut", folder: "Nova LUT", problem: { kind: "missingFolder" } },
      { id: "ultra-plus", folder: "Ultra Plus", problem: { kind: "changed" } },
    ],
    customized: [
      { id: "cyber-engine-tweaks", folder: "Cyber Engine Tweaks", files: files("bin/x64/plugins/cyber_engine_tweaks/bindings.json") },
      { id: "mod-settings", folder: "Mod Settings", files: files("red4ext/plugins/mod_settings/user.ini") },
    ],
  };
}

async function simulateUpdate() {
  const steps = ["Обновление: Cyber Engine Tweaks", "Обновление: RED4ext", "Установка: Better Vehicle Handling", "Удаление: Immersive Traffic", "Порядок загрузки"];
  const total = build.downloadSize;
  let done = build.downloaded;
  for (const [i, label] of steps.entries()) {
    progress?.({ kind: "step", index: i + 1, total: steps.length, label });
    for (let t = 0; t < 10 && i < 3; t++) {
      if (cancelled) return finished?.({ ok: false, error: null });
      await delay(120);
      if (i === 1 && t === 3) {
        progress?.({ kind: "retry", attempt: 2, delaySecs: 2, error: "connection reset" });
        await delay(1500);
      }
      done = Math.min(total, done + total / 30);
      progress?.({ kind: "bytes", done, total });
    }
  }
  progress?.({ kind: "done" });
  status = { ...status, updating: false, installedVersion: build.latestVersion };
  build = {
    ...build,
    installedVersion: build.latestVersion,
    upToDate: true,
    changes: 0,
    downloadSize: 0,
    downloaded: 0,
    mods: build.mods.map((m) => (m.kind === "mod" ? { ...m, outdated: false, installed: true } : m)),
  };
  finished?.({ ok: true, error: null });
}

let nexus: NexusStatus = {
  keySaved: false,
  account: null,
  accountError: null,
  sso: false,
  handler: { supported: true, ours: false, other: "nxmhandler.exe" },
};

function tracked(folder: string, modId: number, version: string, extra: Partial<ModUpdateRow> = {}): ModUpdateRow {
  return {
    folder,
    game: "cyberpunk2077",
    modId,
    version,
    personal: false,
    managed: true,
    canUpdate: settings.authorMode,
    status: { kind: "upToDate" },
    pageUrl: `https://www.nexusmods.com/cyberpunk2077/mods/${modId}?tab=files`,
    needs: [],
    ...extra,
  };
}

/** Unix seconds of the last "Проверить обновления": until then Nexus knows nothing. */
let checkedAt: number | null = null;

function updatesView(): UpdatesView {
  const checked = checkedAt !== null;
  const update = (fileId: number, version: string, name = "Main File") => ({
    kind: "update" as const,
    file: { fileId, name, fileName: `mod-${fileId}.zip`, size: 12 * MB },
    version,
  });
  const mods: ModUpdateRow[] = [
    tracked("Cyber Engine Tweaks", 107, "1.35.0", { status: checked ? update(98000, "1.36.0") : { kind: "unknown" } }),
    tracked("RED4ext", 2380, "1.27.0", { status: checked ? update(97001, "1.28.1") : { kind: "unknown" } }),
    tracked("redscript", 1511, "0.5.27"),
    tracked("Nova LUT", 2075, "2.3", { status: checked ? { kind: "update", file: null, version: "2.4" } : { kind: "unknown" } }),
    tracked("Immersive Rippers", 9330, "1.2", { status: { kind: "unavailable" } }),
    tracked("Better Lightning", 15520, "3.1", {
      personal: true,
      managed: false,
      canUpdate: true,
      status: checked ? update(96000, "3.2", "Better Lightning — main") : { kind: "unknown" },
    }),
    tracked("Photo Mode Unlocker", 2711, "1.0", {
      personal: true,
      managed: false,
      canUpdate: true,
      needs: checked
        ? [
            { modId: 4197, name: "ArchiveXL", url: "https://www.nexusmods.com/cyberpunk2077/mods/4197", notes: "", disabled: null },
            { modId: null, name: "ReShade", url: "https://reshade.me/", notes: "Для фильтров", disabled: null },
          ]
        : [],
      status: checked ? update(95000, "1.1", "Main") : { kind: "unknown" },
    }),
  ];
  return { mods, untracked: ["LYNO Settings", "My Test Weapon"], checkedAt, rateLimit: limits };
}

let jobs: NexusJob[] = [];
let limits: RateLimit = { daily: 19840, hourly: 500, dailyLimit: 20000, hourlyLimit: 500 };
const now = () => Math.floor(Date.now() / 1000);
let downloads: DownloadItem[] = [
  { fileName: "Better Lightning-15520-3-2-1735000000.7z", size: 12 * MB, modified: now() - 600, modName: "Better Lightning", fileTitle: "Main File", version: "3.2", modId: 15520, fileId: 96000, installed: true },
  { fileName: "Photo Mode Unlocker-2711-1-1-1734000000.zip", size: 2 * MB, modified: now() - 7200, modName: "Photo Mode Unlocker", fileTitle: "Main", version: "1.1", modId: 2711, fileId: 95000, installed: false },
  { fileName: "Weird Layout Pack.rar", size: 40 * MB, modified: now() - 90000, modName: "Weird Layout Pack", fileTitle: null, version: null, modId: 0, fileId: 0, installed: false },
];
let userMods: UserRow[] = [
  { kind: "mod", name: "Better Lightning", enabled: true, version: "3.1", nexusUrl: "https://www.nexusmods.com/cyberpunk2077/mods/15520" },
  { kind: "separator", title: "Мои текстуры", color: "#3cf2a0" },
  { kind: "mod", name: "Photo Mode Unlocker", enabled: false, version: "1.0", nexusUrl: "https://www.nexusmods.com/cyberpunk2077/mods/2711" },
];
let overwrite: OverwriteInfo = { files: 14, size: 182_000 };
const userKey = (r: UserRow) => (r.kind === "mod" ? r.name : `${r.title}_separator`);
const mockRoots: ArchiveRoot[] = [
  { path: "", files: 6, valid: false },
  { path: "Pack/", files: 6, valid: false },
  { path: "Pack/Option A/", files: 3, valid: true },
  { path: "Pack/Option A/archive/", files: 3, valid: false },
  { path: "Pack/Option B/", files: 3, valid: true },
];

function emitJob(id: number, patch: Partial<NexusJob>) {
  jobs = jobs.map((j) => (j.id === id ? { ...j, ...patch } : j));
  const job = jobs.find((j) => j.id === id);
  if (job) nexusHandlers?.job(job);
}

/** Installs a mock archive: an odd layout asks for the folder first. */
async function simulateInstall(id: number, file: string, after: string | null) {
  const name = file.replace(/\.(zip|7z|rar)$/i, "").replace(/-\d+-.*$/, "").split(/[\\/]/).pop() ?? file;
  await delay(300);
  if (file.endsWith(".rar") && !jobs.find((j) => j.id === id)?.state.kind.startsWith("installing")) {
    emitJob(id, { state: { kind: "choosingRoot" } });
    return;
  }
  emitJob(id, { state: { kind: "installing" } });
  await delay(600);
  const version = downloads.find((d) => d.fileName === file)?.version ?? null;
  const row: UserRow = { kind: "mod", name, enabled: true, version, nexusUrl: null };
  const at = after === null ? -1 : userMods.findIndex((r) => (r.kind === "mod" ? r.name : `${r.title}_separator`) === after);
  userMods = userMods.filter((r) => r.kind !== "mod" || r.name !== name);
  if (after === "LYNO USER MODS_separator") userMods = [row, ...userMods];
  else if (at < 0) userMods = [...userMods, row];
  else userMods = [...userMods.slice(0, at + 1), row, ...userMods.slice(at + 1)];
  downloads = downloads.map((d) => (d.fileName === file ? { ...d, installed: true } : d));
  emitJob(id, { state: { kind: "done", outcome: { kind: "installed", folder: name } } });
  limits = { ...limits, daily: (limits.daily ?? 0) - 2 };
  nexusHandlers?.limits(limits);
  nexusHandlers?.changed();
}
let nexusHandlers: NexusHandlers | null = null;
let jobSeq = 0;

// A FOMOD installer: the second step shows only for the athletic body, like a flag dependency.
const fomodOutline: FomodOutline = {
  name: "Better Lightning",
  image: null,
  steps: [
    {
      name: "Основной вариант",
      groups: [
        {
          name: "Яркость",
          kind: "exactlyOne",
          plugins: [
            { name: "Мягкий свет", description: "Ближе к ванильной игре: ночи темнее, неон не слепит.", image: null },
            { name: "Кинематографичный", description: "Контрастнее и насыщеннее. Рекомендуется для HDR.", image: null },
          ],
        },
      ],
    },
    {
      name: "Дополнения",
      groups: [
        {
          name: "Модули",
          kind: "any",
          plugins: [
            { name: "Объёмный туман", description: "Требует Nova LUT.", image: null },
            { name: "Отражения в лужах", description: "Тяжело для видеокарты.", image: null },
          ],
        },
        {
          name: "Совместимость",
          kind: "atMostOne",
          plugins: [
            { name: "Патч для Nova LUT", description: "Не нужен: Nova LUT уже учтён.", image: null },
            { name: "Патч для ReShade", description: "Если вы пользуетесь ReShade.", image: null },
          ],
        },
      ],
    },
  ],
};

function fomodEval(sel: FomodSelection): FomodState {
  const first = sel[0]?.[0] ?? [1];
  const visible = [true, first[0] === 1];
  const kinds: FomodState["kinds"] = [
    [["optional", "recommended"]],
    [
      ["optional", "optional"],
      ["notUsable", "optional"],
    ],
  ];
  const second = visible[1] ? (sel[1] ?? [[], []]).map((g, i) => g.filter((p) => kinds[1][i][p] !== "notUsable").slice(0, i === 1 ? 1 : 2)) : [];
  const selection = [[first], second];
  return { visible, kinds, selection, valid: [first.length === 1, true] };
}

async function simulateDownload(job: NexusJob, fomod = false) {
  const emit = (patch: Partial<NexusJob>) => {
    job = { ...job, ...patch };
    jobs = jobs.map((j) => (j.id === job.id ? job : j));
    nexusHandlers?.job(job);
  };
  await delay(400);
  const total = 12 * MB;
  emit({ title: job.title ?? "Better Lightning", fileTitle: "Main File", version: "3.2", replaces: "Better Lightning", state: { kind: "downloading", done: 0, total } });
  for (let done = 0; done <= total; done += total / 20) {
    if (jobs.find((j) => j.id === job.id)?.state.kind === "cancelled") return;
    emit({ state: { kind: "downloading", done, total } });
    await delay(120);
  }
  if (fomod) {
    emit({ state: { kind: "choosing" } });
    return;
  }
  // A Nexus download only lands in the downloads.
  emit({ state: { kind: "done", outcome: { kind: "downloaded", replaces: "Better Lightning" } } });
  nexusHandlers?.changed();
}

async function simulateFomodInstall(id: number) {
  const emit = (state: NexusJob["state"]) => {
    jobs = jobs.map((j) => (j.id === id ? { ...j, state } : j));
    const job = jobs.find((j) => j.id === id);
    if (job) nexusHandlers?.job(job);
  };
  emit({ kind: "installing" });
  await delay(600);
  emit({ kind: "done", outcome: { kind: "installed", folder: "Better Lightning" } });
  nexusHandlers?.changed();
}

function useInstance(entry: InstanceEntry) {
  const instances = settings.instances.some((i) => i.dir === entry.dir) ? settings.instances : [...settings.instances, entry];
  settings = { ...settings, instanceDir: entry.dir, instances };
  status = { ...status, mo2Installed: true, installedVersion: entry.build ? build.installedVersion : null };
}

export const mock = {
  getSettings: async () => settings,
  saveSettings: async (s: Settings) => {
    if (s.authorMode && !settings.authorMode) throw new Error("Режим автора включается по токену GitHub: «Я автор сборки» внизу настроек");
    settings = { ...s, instanceDir: settings.instanceDir, instances: settings.instances };
    status = { ...status, gameDir: s.gameDir, gameFound: !!s.gameDir };
  },
  detectGames: async (): Promise<GameInstall[]> => [{ path: "D:\\SteamLibrary\\steamapps\\common\\Cyberpunk 2077", store: "Steam" }],
  getStatus: async () => status,
  fetchBuild: async () => {
    await delay(200);
    return settings.instances.find((i) => i.dir === settings.instanceDir)?.build ? build : null;
  },
  startUpdate: async () => {
    if (settings.authorMode) throw new Error(AUTHOR_MODE_NO_UPDATE);
    cancelled = false;
    status = { ...status, updating: true };
    void simulateUpdate();
  },
  cancelUpdate: async () => {
    cancelled = true;
    status = { ...status, updating: false };
  },
  verifyBuild: () => simulateVerify(),
  cancelVerify: async () => {
    verifyCancelled = true;
  },
  startRepair: async (_ids: string[], _base: boolean, _resetSettings: boolean) => {
    if (settings.authorMode) throw new Error(AUTHOR_MODE_NO_UPDATE);
    cancelled = false;
    status = { ...status, updating: true };
    void simulateUpdate();
  },
  setModEnabled: async (id: string, enabled: boolean) => {
    if (status.gameRunning || status.mo2Running) throw new Error("Закройте игру и Mod Organizer 2, чтобы включать и выключать моды");
    build = { ...build, mods: build.mods.map((m) => (m.kind === "mod" && m.id === id ? { ...m, enabled } : m)) };
  },
  authorChanges: async (): Promise<Pending> => {
    await delay(150);
    return pending;
  },
  authorAdopt: async () => {
    await delay(300);
    status = { ...status, installedVersion: build.latestVersion };
    build = { ...build, installedVersion: build.latestVersion, upToDate: true, changes: 0, downloadSize: 0 };
  },
  authorEnable: async (token: string | null) => {
    await delay(400);
    if (!token?.startsWith("github_pat_")) {
      throw new Error("Токен не подходит: repository ItsLyno/LYNO-HARDWIRED: GitHub answered 401: Bad credentials (the token is wrong or expired)");
    }
    secrets.github = true;
    settings = { ...settings, authorMode: true };
  },
  authorSecrets: async () => ({ ...secrets }),
  authorSetSecret: async (secret: Secret, value: string | null) => {
    await delay(400);
    if (secret === "githubToken" && value && !value.startsWith("github_pat_")) {
      throw new Error("Токен не подходит: repository ItsLyno/LYNO-HARDWIRED: GitHub answered 401: Bad credentials (the token is wrong or expired)");
    }
    secrets[secret === "githubToken" ? "github" : "nexus"] = !!value;
  },
  authorBuilt: async () => built,
  authorBuild: async (version: string, gameVersion: string, notes: string[]) => {
    if (status.gameRunning || status.mo2Running) throw new Error("Закройте игру и Mod Organizer 2: MO2 переписывает список модов при выходе");
    if (build.latestVersion !== status.installedVersion) {
      throw new Error(`Опубликована версия ${build.latestVersion}, а установлена ${status.installedVersion}. Сначала примите опубликованную версию.`);
    }
    built = null;
    void simulateBuild(version, gameVersion, notes);
  },
  authorPublish: async () => {
    if (!secrets.github) throw new Error("Добавьте токен GitHub");
    void simulatePublish();
  },
  authorCancel: async () => {
    authorCancelled = true;
  },
  onAuthorEvent: async (h: (e: AuthorEvent) => void) => {
    // StrictMode subscribes the same handler twice: a wrapper per subscription
    // keeps the first unsubscribe from dropping the second one.
    const mine = (e: AuthorEvent) => h(e);
    author = mine;
    return () => {
      if (author === mine) author = null;
    };
  },
  openModFolder: async (_id: string) => {},
  openFolder: async (_folder: Folder) => {},
  launchGame: async () => {},
  launchExecutable: async (_executable: string) => {},
  executables: async (): Promise<Executable[]> => [
    { title: "Cyberpunk 2077 (REDmod)", binary: "C:/Games/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe" },
    { title: "REDmod", binary: "C:/Games/Cyberpunk 2077/tools/redmod/bin/redMod.exe" },
    { title: "WolvenKit", binary: "C:/Tools/WolvenKit/WolvenKit.exe" },
  ],
  openMo2: async () => {},
  exportReport: async () => {
    await delay(600);
    return "C:\\Users\\V\\Desktop\\LYNO-report-2026-10-04_18-20-00.zip";
  },
  launcherVersion: async () => "0.1.0",
  checkLauncherUpdate: async (): Promise<LauncherUpdate | null> => ({
    version: "0.2.0",
    currentVersion: "0.1.0",
    notes: "Отчёт для автора сборки, автообновление лаунчера",
  }),
  installLauncherUpdate: async () => {
    await delay(1500);
  },
  openUrl: async (url: string) => {
    window.open(url, "_blank");
  },
  nexusStatus: async () => {
    await delay(150);
    return nexus;
  },
  nexusSetKey: async (key: string) => {
    await delay(500);
    if (key.trim().length < 20) throw new Error("Nexus не принял ключ: проверьте, что он скопирован целиком");
    nexus = { ...nexus, keySaved: true, account: { name: "V", premium: false }, accountError: null };
    return nexus;
  },
  nexusLogout: async () => {
    nexus = { ...nexus, keySaved: false, account: null };
  },
  nexusSsoLogin: async (): Promise<NexusStatus> => {
    throw new Error("Вход через Nexus пока недоступен: вставьте ключ API");
  },
  nexusSsoCancel: async () => {},
  nxmRegister: async (): Promise<NxmHandler> => {
    nexus = { ...nexus, handler: { supported: true, ours: true, other: null } };
    return nexus.handler;
  },
  nxmUnregister: async (): Promise<NxmHandler> => {
    nexus = { ...nexus, handler: { supported: true, ours: false, other: "nxmhandler.exe" } };
    return nexus.handler;
  },
  nexusUpdates: async () => updatesView(),
  nexusCheck: async (_force: boolean) => {
    if (!nexus.account) throw new Error("Войдите в Nexus Mods в настройках");
    for (let done = 0; done <= 7; done++) {
      nexusHandlers?.check({ done, total: 7 });
      await delay(150);
    }
    checkedAt = now();
    return updatesView();
  },
  nexusCheckCancel: async () => {},
  nexusJobs: async () => jobs,
  nexusCancelJob: async (id: number) => {
    jobs = jobs.map((j) => (j.id === id ? { ...j, state: { kind: "cancelled" } } : j));
    const job = jobs.find((j) => j.id === id);
    if (job) nexusHandlers?.job(job);
  },
  nexusClearJobs: async () => {
    jobs = jobs.filter((j) => !["done", "failed", "cancelled"].includes(j.state.kind));
  },
  nexusDownload: async (game: string, modId: number, fileId: number) => {
    if (!nexus.account?.premium) throw new Error("Без Premium Nexus отдаёт файлы только по кнопке «Mod Manager Download» на сайте");
    const job: NexusJob = { id: ++jobSeq, source: "nexus", game, modId, fileId, title: null, fileTitle: null, version: null, replaces: null, needs: [], state: { kind: "queued" } };
    jobs = [...jobs, job];
    nexusHandlers?.job(job);
    // Every second download carries a FOMOD installer.
    void simulateDownload(job, jobSeq % 2 === 0);
    return job.id;
  },
  nexusLimits: async () => (nexus.account ? limits : null),
  downloadsRecent: async () => downloads,
  installArchive: async (file: string, after: string | null) => {
    const job: NexusJob = {
      id: ++jobSeq,
      source: "file",
      game: "",
      modId: 0,
      fileId: 0,
      title: file.split(/[\\/]/).pop() ?? file,
      fileTitle: null,
      version: null,
      replaces: null,
      needs: [],
      state: { kind: "queued" },
    };
    jobs = [...jobs, job];
    nexusHandlers?.job(job);
    void simulateInstall(job.id, file, after);
    return job.id;
  },
  archiveTarget: async (file: string): Promise<ArchiveTarget> => {
    const d = downloads.find((x) => x.fileName === file);
    const modName = d?.modName ?? file.split(/[\\/]/).pop() ?? file;
    const have = userMods.find((r): r is Extract<UserRow, { kind: "mod" }> => r.kind === "mod" && r.name === modName);
    return { modName, version: d?.version ?? null, replaces: have?.name ?? null, installedVersion: have?.version ?? null };
  },
  installRoots: async (_id: number) => {
    await delay(200);
    return mockRoots;
  },
  installSetRoot: async (id: number, _root: string) => {
    const job = jobs.find((j) => j.id === id);
    emitJob(id, { state: { kind: "installing" } });
    void simulateInstall(id, (job?.title ?? "mod").replace(/\.rar$/, ".zip"), null);
  },
  userMods: async () => userMods,
  setUserModEnabled: async (folder: string, enabled: boolean) => {
    userMods = userMods.map((r) => (r.kind === "mod" && r.name === folder ? { ...r, enabled } : r));
  },
  openUserModFolder: async (_folder: string) => {},
  deleteMod: async (folder: string) => {
    userMods = userMods.filter((r) => r.kind !== "mod" || r.name !== folder);
  },
  renameMod: async (folder: string, name: string) => {
    if (userMods.some((r) => r.kind === "mod" && r.name === name)) throw new Error(`Мод «${name}» уже есть`);
    userMods = userMods.map((r) =>
      r.kind === "mod" && r.name === folder ? { ...r, name } : r.kind === "separator" && userKey(r) === folder ? { ...r, title: name } : r,
    );
  },
  moveUserMod: async (folder: string, after: string | null) => {
    const row = userMods.find((r) => userKey(r) === folder);
    if (!row) throw new Error("Перетаскивать можно только ваши моды: порядок сборки задаёт её автор");
    userMods = userMods.filter((r) => r !== row);
    const at = after === null ? userMods.length : userMods.findIndex((r) => userKey(r) === after) + 1;
    userMods.splice(at, 0, row);
  },
  addSeparator: async (title: string, after: string | null) => {
    if (userMods.some((r) => r.kind === "separator" && r.title === title)) throw new Error(`Разделитель «${title}» уже есть`);
    const at = after === null ? userMods.length : userMods.findIndex((r) => userKey(r) === after) + 1;
    userMods.splice(at, 0, { kind: "separator", title, color: null });
  },
  setSeparatorColor: async (folder: string, color: string | null) => {
    userMods = userMods.map((r) => (r.kind === "separator" && userKey(r) === folder ? { ...r, color } : r));
  },
  overwriteInfo: async (): Promise<OverwriteInfo> => overwrite,
  overwriteToMod: async (name: string) => {
    userMods = [...userMods, { kind: "mod", name, enabled: true, version: null, nexusUrl: null }];
    overwrite = { files: 0, size: 0 };
  },
  clearOverwrite: async () => {
    overwrite = { files: 0, size: 0 };
  },
  reinstallMod: async (folder: string): Promise<number> => {
    throw new Error(`Архива мода «${folder}» нет в загрузках Mod Organizer 2: скачайте мод заново и перетащите его в список`);
  },
  instanceSelect: async (dir: string) => useInstance(settings.instances.find((i) => i.dir === dir)!),
  instanceAdd: async (dir: string) => useInstance({ name: dir.split("\\").pop() ?? dir, dir, build: false }),
  instanceAddBuild: async () => useInstance({ name: "LYNO//HARDWIRED", dir: BUILD_DIR, build: true }),
  startMo2Setup: async () => {
    await delay(800);
    useInstance({ name: "Mod Organizer 2", dir: "C:\\Users\\V\\AppData\\Local\\dev.lyno.hardwired\\mo2", build: false });
  },
  // No Explorer in a browser.
  onFileDrop: async (_h: FileDropHandlers) => () => {},
  nexusFomod: async (_id: number): Promise<FomodWizard> => {
    await delay(300);
    const previous: FomodSelection = [[[1]], [[0], []]];
    return { outline: fomodOutline, images: {}, previous, state: fomodEval(previous) };
  },
  nexusFomodEval: async (_id: number, selection: FomodSelection) => fomodEval(selection),
  nexusFomodInstall: async (id: number, selection: FomodSelection) => {
    if (!fomodEval(selection).valid.every(Boolean)) throw new Error("Шаг «Основной вариант»: выберите варианты");
    void simulateFomodInstall(id);
  },
  onNexus: async (h: NexusHandlers) => {
    const mine = { ...h };
    nexusHandlers = mine;
    return () => {
      if (nexusHandlers === mine) nexusHandlers = null;
    };
  },
  onVerifyProgress: async (p: (e: UpdateEvent) => void) => {
    const mine = (e: UpdateEvent) => p(e);
    verifyProgress = mine;
    return () => {
      if (verifyProgress === mine) verifyProgress = null;
    };
  },
  onUpdate: async (p: (e: UpdateEvent) => void, f: (r: UpdateFinished) => void) => {
    const [mineP, mineF] = [(e: UpdateEvent) => p(e), (r: UpdateFinished) => f(r)];
    progress = mineP;
    finished = mineF;
    return () => {
      if (progress === mineP) progress = null;
      if (finished === mineF) finished = null;
    };
  },
};
