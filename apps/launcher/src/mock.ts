// Browser-only stand-in for the Tauri backend (`pnpm dev` without Tauri),
// so screens can be built and screenshotted without Windows or MO2.
// The data is illustrative, not the real build.
import type {
  AuthorEvent,
  BuildInfo,
  BuiltRelease,
  Folder,
  GameInstall,
  LauncherUpdate,
  ModRow,
  Pending,
  Secret,
  Settings,
  Status,
  UpdateEvent,
  UpdateFinished,
  VerifyReport,
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
];

let settings: Settings = {
  instanceDir: "C:\\Users\\V\\AppData\\Local\\dev.lyno.hardwired\\instance",
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

export const mock = {
  getSettings: async () => settings,
  saveSettings: async (s: Settings) => {
    if (s.authorMode && !settings.authorMode) throw new Error("Режим автора включается по токену GitHub: «Я автор сборки» внизу настроек");
    settings = s;
    status = { ...status, gameDir: s.gameDir, gameFound: !!s.gameDir };
  },
  detectGames: async (): Promise<GameInstall[]> => [{ path: "D:\\SteamLibrary\\steamapps\\common\\Cyberpunk 2077", store: "Steam" }],
  getStatus: async () => status,
  fetchBuild: async () => {
    await delay(200);
    return build;
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
